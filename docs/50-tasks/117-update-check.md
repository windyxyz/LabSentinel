# 任务 117 · 版本更新检查（P2，不自动安装）

> 背景：便携版靠用户手动下载新 zip；希望面板能提示“有新版本”。本任务只做**检查与提示**，
> 不自动下载/安装（无托管渠道、签名未定，自动更新风险高）。

## 目标

### 1. 配置

- `LS_UPDATE_MANIFEST_URL`：版本清单 URL（默认空=关闭）；
- `LS_UPDATE_ALLOW_HTTP=0`：默认仅 https（自建局域网可显式开）；
- `LS_UPDATE_TIMEOUT_S=5`。

清单格式（自建静态 JSON）：
```json
{"version":"2026.11","url":"https://.../LabSentinel-portable-20261101.zip",
 "sha256":"...","notes":"...","published_at":"2026-11-01"}
```

### 2. API

- `GET /api/update/check?refresh=0`（require_read）：
  - 未配置 → `{enabled:false}`；
  - 配置 → httpx GET（不跟随重定向、超时、仅 https 除非 allow_http）；
  - 比较清单 version 与当前版本（`app/VERSION` 或 `_system_version`）：字符串不等即 `update_available:true`
    （不做 semver 排序，文档说明）；
  - 返回 `{enabled,current,latest,update_available,url,sha256,notes,published_at,error}`；
  - 结果缓存 1h（`refresh=1` 强制）；网络失败返回 `error` 不 500；
  - **不下载/不执行**任何 URL。

### 3. 前端 `Dashboard.vue`

- 顶部/底部小卡片“版本”：当前版本；有更新时显示“发现新版本 2026.11” + 「查看」链接
  （新窗口打开清单 url）+ 「忽略此版本」（localStorage）；
- 未配置/检查失败不打扰（不显示红条）；手动「检查更新」按钮；
- 不新增路由/不改全局四件套。

### 4. 测试 `server/tests/test_update_check.py`

- 未配置 enabled=false；https 清单解析/版本不同/相同；
- http 默认拒绝、allow_http 通过；超时/非 JSON/缺字段 → error 非 500；
- 缓存：连续两次只请求一次，refresh=1 再请求；
- 不下载 URL（mock 断言只 GET manifest）；
- 鉴权口径。

### 5. 文档

- `.env.example` 3 项；`部署FAQ`「怎么配更新提示/为什么不在线升级」；
- `运维手册` 升级一节加“手动下载 zip→备份→覆盖升级”。

## 约束

- 不自动下载/安装/执行；不跟随重定向；不发送本机信息（仅 GET manifest）；
- 不引入依赖；测试 mock httpx，不访问外网；
- 前端只改 Dashboard.vue/api.js。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 用本地 HTTPS/HTTP mock 清单实测 available/相同/失败三态；
- 报告附响应 JSON（脱敏）。

## 执行报告

（待填：改动 / 三态证据 / 测试与 build / 遗留）
