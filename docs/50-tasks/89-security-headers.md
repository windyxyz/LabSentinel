# 任务 89 · 安全响应头（P1）

> 背景：服务默认内网 HTTP，但用户可能通过反向代理暴露到办公网；当前响应没有
> `X-Content-Type-Options`/`X-Frame-Options` 等基础防护，页面可被第三方站点 iframe 嵌套。
> 本任务加基础安全头，CSP 采用**可选配置**避免误伤 go2rtc/WebRTC。

## 目标

### 1. 中间件

- `server/app/main.py` 增加安全头中间件（可配置关闭）：
  - `X-Content-Type-Options: nosniff`
  - `X-Frame-Options: DENY`（若需要嵌入大屏可配置 SAMEORIGIN）
  - `Referrer-Policy: no-referrer`
  - `Permissions-Policy: camera=(), microphone=(), geolocation=()`
  - `Cross-Origin-Resource-Policy: same-origin`（注意：WebRTC/go2rtc 跨端口资源是否受影响？
    若实测影响，改为 `same-site` 或移除并在报告说明）
- 配置：
  - `LS_SECURITY_HEADERS=1`（默认开）；`0` 关闭；
  - `LS_FRAME_OPTIONS=DENY|SAMEORIGIN`（默认 DENY）；
  - `LS_CSP=""`（默认空=不发送 CSP）；配置后原样作为 `Content-Security-Policy` 头发送。
    文档给一个保守示例（含 `ws:`/`wss:`/`http://127.0.0.1:*`），并提示先用
    `Content-Security-Policy-Report-Only` 验证（可选配置 `LS_CSP_REPORT_ONLY=1`）。
- 所有响应（API/静态/错误）都带；不覆盖已有同名头。

### 2. 测试 `server/tests/test_security_headers.py`

- 默认：`/`、`/api/system`、404 都带 4 个基础头且值正确；
- `LS_SECURITY_HEADERS=0`：不添加；
- `LS_FRAME_OPTIONS=SAMEORIGIN` 生效；
- `LS_CSP` 配置后头发送、report-only 开关正确；
- 不破坏现有静态/API 测试。

### 3. 文档

- `docs/部署指南.md` 增「反向代理/HTTPS 与安全头」小节（Nginx/Caddy 最小示例，HSTS 由代理加）；
- `docs/部署FAQ.md`「能嵌到别的系统里吗（X-Frame-Options）」「CSP 怎么开」；
- `.env.example` 4 个配置注释。

## 约束

- 默认不发送 CSP，避免夜间无法人工排查的页面白屏；基础头默认开但都低风险；
- 不引入依赖；不改前端四件套；
- 若 `Cross-Origin-Resource-Policy` 影响 go2rtc WebRTC（跨端口），在报告说明并调整/移除该项。

## 验收

- `pytest tests/ -q` 全绿；
- 8001 实跑 `/` 与 `/api/system` 响应头符合预期；前端页面能正常加载（截图/playwright 简查）；
- 报告附响应头原文。

## 执行报告

（待填：改动文件 / 响应头证据 / 测试 / 遗留）
