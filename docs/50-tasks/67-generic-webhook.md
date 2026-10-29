# 任务 67 · 通用 Webhook 告警渠道（HMAC 签名）

> 背景：现有 7 个渠道都是厂商专用（企微/钉钉/飞书/Server酱/PushPlus/Qmsg/邮件），用户自建系统
> （n8n、自研平台、局域网网关）无法接入。本任务增加一个**通用 HTTP Webhook** 渠道，带 HMAC 签名，
> 让告警可以推给任意自有服务。

## 目标

### 1. 配置（`server/app/config.py`，`LS_` 前缀）

- `webhook_url: str | None = None`：目标地址；
- `webhook_secret: str | None = None`：HMAC-SHA256 密钥（可选；配置后每个请求带签名头）；
- `webhook_allow_http: bool = False`：默认只允许 `https`；局域网自建服务可显式设 `true`；
- `webhook_timeout_s: float = 5.0`。

### 2. `GenericWebhookChannel`

- 继承 `_HttpPushChannel`；`configured()` = url 非空；
- URL 校验：`https` 默认；`http` 仅在 `webhook_allow_http=true` 时允许；拒绝非 http(s)、带用户信息
  （`user:pass@`）的 URL；**不跟随重定向**（沿用基类）；超时用配置值；
- 请求：`POST`，`Content-Type: application/json`，body：
  ```json
  {"version":1,"title":"LabSentinel 级别3告警","text":"<render_alert 文本>",
   "level":3,"ts":"<ISO>","source":"labsentinel"}
  ```
  若事件行可用，附 `event_id/camera_id/type/detail`（`send` 目前只拿到 level/text；如需更多字段，
  给 `send` 增加可选 `event` 参数并保持其它渠道签名兼容——**不要破坏现有渠道**）。
- 签名：配置 `webhook_secret` 时，对**原始请求体字节**计算
  `hex(hmac_sha256(secret, body))`，放 `X-LS-Signature: sha256=<hex>`；同时带
  `X-LS-Event: alert`、`X-LS-Timestamp: <unix>`。
- 成功判定：HTTP 2xx 即成功（不解析业务码）；失败沿用 dispatcher 重试/死信。
- 注册进 `dispatcher` 渠道列表（保持 `alerts/status` 可见、`alert-test` 可测）。

### 3. 测试 `server/tests/test_generic_webhook.py`

- `configured()`：空/空白 URL → False；
- URL 校验：https 通过；http 默认拒绝、`allow_http=true` 通过；`ftp://`/带凭证 URL 拒绝；
- 用本地 `http.server`（线程）或 `httpx.MockTransport` 接收请求：断言 JSON 字段、签名头与
  用同密钥独立计算的 HMAC 一致、时间戳存在；
- 5xx/超时 → False（触发重试逻辑由现有 dispatcher 测试覆盖，不重复）；
- `/api/alerts/status` 中出现 `webhook` 渠道；`/api/alert-test` 对未配置渠道返回「未配置」且不建投递行。

### 4. 文档

- `docs/部署指南.md` 告警章节增加通用 Webhook 配置示例（含签名校验的 Python/Node 伪代码）；
- `.env.example` 增加 4 个配置项注释；
- `docs/部署FAQ.md` 增加「自建服务如何验签 / 为什么 http 被拒绝」。

## 约束

- 不改现有渠道行为/域名白名单；通用渠道的放行逻辑独立；
- 不引入依赖（hmac/hashlib/json 标准库 + 现有 httpx）；
- 测试不得访问真实外网；本地接收器只绑 127.0.0.1 随机端口，测试后关闭。

## 验收标准

- `pytest tests/ -q` 全绿（基线 91 + 新增）；
- 本地接收器实跑：收到 1 条告警、签名可验证；`alerts/status` 显示 webhook 已配置；
- 报告附请求体与签名验证结果（密钥用测试值，不写真实密钥）。

## 执行报告

**flash 2026-10-30 完成（pytest 135 passed，本地接收器实跑签名验证通过）**

- **改动**：`config.py` 加 4 配置项（webhook_url/secret/allow_http/timeout_s）；`alerts.py` 加
  `GenericWebhookChannel`（继承 _HttpPushChannel：独立 URL 放行——默认仅 https、http 需
  allow_http、拒绝非 http(s) 与带 user:pass@ 的 URL、沿用不跟随重定向；成功判定=HTTP 2xx 覆盖
  基类业务码解析；HMAC-SHA256 对**原始 body 字节**签名 → `X-LS-Signature: sha256=<hex>` +
  `X-LS-Event: alert` + `X-LS-Timestamp`；body 含 version/title/text/level/ts/source，事件行可用时附
  event_id/camera_id/type/detail）；`redact_url()` 日志脱敏；dispatcher 注册第 8 渠道；
  **dispatch 通过 `_send_accepts_event()`（inspect 签名探测）向支持 event 参数的渠道附带完整事件行，
  其余 7 渠道签名不变**（规格要求的兼容方式）。
- **既有测试同步**：test_alerts_page / test_push 的渠道清单断言 7→8（新渠道入列）。
- **测试**：`test_generic_webhook.py` 5 用例（configured 空白/URL 规则矩阵：https✓ http默认✗
  allow_http✓ ftp✗ 带凭证✗ / **本地 ThreadingHTTPServer 接收器实跑**：JSON 字段+独立计算 HMAC
  与 X-LS-Signature 一致+时间戳存在 / 5xx→False 无密钥 2xx→True / status 含 webhook 且 alert-test
  未配置返回「未配置」+不建投递行）。全量 **pytest 135 passed**（130 + 5）。
- **过程中修的真问题**：渠道类初版漏写 `configured()`（基类恒 False，alert-test/status 全失真）；
  `/api/alerts/status` 的 configured_map 漏 webhook 键。
- **文档**：部署指南告警表加通用 Webhook 行+验签 Python 片段+独立放行说明；.env.example 4 项注释；
  部署FAQ 加「自建服务怎么验签 / 为什么 http 被拒」。
- **遗留**：无。真实 n8n 等外部系统联调属用户侧配置（本地接收器已等价验证签名与 2xx 判定）。
