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

**flash 2026-10-30 完成（pytest 161 passed，8001 实跑四类响应头全达标）**

- **改动**（全部在 `server/app/main.py`，无新依赖）：
  - `SPAStaticFiles.get_response` 加缓存策略：`assets/*` → `public, max-age=31536000, immutable`；
    `index.html` 与 SPA 回退 → `no-cache`；其余静态 → `public, max-age=3600`；304 响应同享；
    `/api`、`/ws` 不经此处（行为不变）。保留 Starlette ETag/Last-Modified。
  - 自写选择性 gzip 中间件：只压 `text/*`、`image/svg+xml`、`application/javascript`、
    `application/json`；zip/video/jpeg/octet-stream 天然不在白名单；≥1024 字节才压；Range 请求
    让路（视频/证据包断点续传优先）；加 `Vary: Accept-Encoding`。
- **测试**：`test_web_static.py` 扩展 4 用例（缓存头三档 + 304 带 immutable / gzip 资产解压后与
  原文一致 + Vary / zip 不压 + Range 优先 / 既有 SPA 回退与未知 API 404 保持）。全量
  **pytest 161 passed**（157 + 4）。
- **8001 实跑响应头证据**：
  - `GET /assets/index-oKAKbDXN.js`（Accept-Encoding: gzip）→ 200，
    `cache-control=public, max-age=31536000, immutable`、`content-encoding=gzip`、
    `vary=Accept-Encoding`、`content-length=44923`（原始 117.62KB → **压缩后 44.9KB，-62%**）；
  - `GET /` 与 `GET /events` → `cache-control=no-cache`；
  - `GET /api/export/evidence.zip` → `content-encoding` 无（`application/zip` 不压）。
- **实跑暴露并修复的 1 个问题**：`GET /` 时 StaticFiles 内部传给 get_response 的 path 是
  `"."`（非空串）——首次实现漏判导致 / 命中 1h 档；已把 `"."` 纳入 index 判定（插桩定位）。
- **文档**：部署FAQ「升级后页面还是旧的怎么办」；部署指南单端口注记补缓存策略。
- **遗留**：无。
