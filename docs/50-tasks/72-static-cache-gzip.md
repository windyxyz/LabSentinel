# 任务 72 · 静态资源缓存头 + 选择性 gzip

> 背景：单端口模式由 FastAPI 托管 `web/dist`，目前没有显式缓存策略：带 hash 的 JS/CSS 每次刷新都
> 可能重新下载；index.html 反而可能被缓存导致升级后仍加载旧资源。加上选择性 gzip 可降低首屏传输
> （当前 JS gzip 后约 42.77KB，但服务端是否压缩取决于客户端/中间件）。

## 目标

### 1. 缓存头（`server/app/main.py` 的 `SPAStaticFiles`）

按路径设置 `Cache-Control`：

| 路径 | Cache-Control |
|---|---|
| `/assets/*`（Vite 带内容 hash） | `public, max-age=31536000, immutable` |
| `/index.html`、SPA 回退响应 | `no-cache`（必须回源校验，保证升级立即生效） |
| 其它静态文件（favicon、图片等） | `public, max-age=3600` |
| `/api/*`、`/ws/*` | 不设置（保持现状） |

- 保留 Starlette 的 ETag/Last-Modified；返回 304 时也带对应 Cache-Control；
- SPA 回退到 `index.html` 的响应必须是 `no-cache`（不能沿用被请求路径的缓存策略）。

### 2. 选择性 gzip 中间件

- 只压缩：`text/*`、`application/javascript`、`application/json`、`image/svg+xml`；
- **不压缩**：`application/zip`、`video/*`、`image/jpeg`、`application/octet-stream`（证据包/视频/截图）；
- 仅当请求 `Accept-Encoding` 含 gzip 且响应体 ≥ 1024 字节、且尚未有 `Content-Encoding` 时压缩；
- 响应加 `Vary: Accept-Encoding`；
- 可用 Starlette `GZipMiddleware` + 自定义 `Content-Type` 判断，或自写轻量中间件；**不引入新依赖**。

### 3. 测试 `server/tests/test_web_static.py` 扩展

- `/assets/index-*.js`：`Cache-Control` 含 `immutable`；`Accept-Encoding: gzip` 时
  `Content-Encoding: gzip` 且解压后内容与原文一致；
- `/` 与 `/events`（SPA 回退）：`Cache-Control: no-cache`；
- 证据包/导出响应（可用 TestClient 触发一个小 zip 导出，或直接构造 `application/zip` 响应）
  **不应**被 gzip；
- 304 分支：带 `If-None-Match` 请求资源 → 304 且缓存头存在；
- 缺失资源仍 404；未知 `/api/*` 仍 404 不回退 HTML（现有测试保持）。

### 4. 文档

- `docs/部署FAQ.md` 增加「升级后页面还是旧的怎么办（index.html no-cache，强刷/清缓存）」；
- `docs/59-portable-install.md` 单端口一节说明缓存策略；
- `docs/35-api-reference.md` 不需要改（静态资源非 API）。

## 约束

- 不改前端构建配置（Vite 已输出 hash 文件名）；不改 `theme.css/style.css/App.vue/main.js`；
- 不压缩大文件/已压缩格式，避免 CPU 与体积反效果；
- 不破坏 Range 请求（视频/证据下载）；如实现中发现与 Range 冲突，优先保 Range，缓存/gzip 让路；
- 不改变 API 成功响应 body。

## 验收标准

- `pytest tests/ -q` 全绿（基线 91 + 新增）；`npm run build` 后 8001 实跑：
  `curl -H "Accept-Encoding: gzip" -I /assets/<js>` 看到 gzip + immutable；
  `curl -I /` 看到 no-cache；导出 zip 无 gzip；
- 报告附响应头原文（脱敏）与解压后大小对比。

## 执行报告

（待填：改动文件 / 响应头证据 / 测试 / 遗留）
