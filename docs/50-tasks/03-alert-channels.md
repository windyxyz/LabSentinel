# 任务 03 · 钉钉通知渠道 + 告警模板（先读本目录 README.md）

## 背景

告警通道已有：应用内 WebSocket、企业微信 webhook（`server/app/alerts.py` 的 WeComChannel，带域名白名单）、SMTP 邮件。现增加**钉钉机器人**渠道，并统一告警文案模板。

## 目标

1. 新增 `DingTalkChannel`；2. 三渠道共用文案渲染函数；3. 配置项与 .env 示例；4. 单测。

## 要求（server/app/alerts.py 及配置）

- 新类 `DingTalkChannel`，POST `https://oapi.dingtalk.com/robot/send?access_token=...`，body `{"msgtype":"markdown","markdown":{"title":"LabSentinel告警","text":"..."}}`
- **域名白名单**：仿照 WeComChannel 的 `_safe_webhook`（白名单 `oapi.dingtalk.com`、强制 https、`follow_redirects=False`），防 SSRF——安全扫描会拦无校验的动态 URL
- 配置：`settings.dingtalk_webhook`（env `LS_DINGTALK_WEBHOOK`），空=不启用
- 文案模板函数 `render_alert(event_row) -> str`：含级别图标（L1ℹ️/L2⚠️/L3🔴）、时间、摄像头、类型中文、详情；三渠道统一调用
- 事件类型中文映射不要复制粘贴散落各处：新增 `server/app/constants.py` 放 `TYPE_TEXT` 字典（Web 端 `web/src/api.js` 已有一份，服务端这份独立维护并在注释注明与前端同步）

## 测试

- `tests/test_alerts.py`：渲染函数输出断言（含各级别）；白名单拒绝非法域名（http://、其他域、空）；DingTalkChannel 在 webhook 为空时静默跳过
- 网络 POST 用 `httpx.MockTransport` mock，不发真实请求

## 验收标准

pytest 全绿（原有 6 个 + 新增 ≥5 个）；`render_alert` 输出可读（人工检查一次）。

## 执行报告

### 交付物（含追加需求：任务执行中用户要求补全微信/QQ 等渠道）
- `server/app/alerts.py` 重构为 `_HttpPushChannel` 基类 + 7 渠道：
  - 保留：应用内 WebSocket、`WeComChannel`（企业微信，qyapi.weixin.qq.com）、`EmailChannel`（SMTP，含 QQ 邮箱）
  - 任务要求：`DingTalkChannel`（oapi.dingtalk.com，markdown）
  - 用户追加：`FeishuChannel`（open.feishu.cn）、`ServerChanChannel`（Server酱→微信，sctapi.ftqq.com）、
    `PushPlusChannel`（PushPlus→微信，www.pushplus.plus）、`QmsgChannel`（Qmsg酱→QQ，qmsg.zendee.cn）
- 全部 HTTP 渠道统一：域名白名单 + 强制 https + `follow_redirects=False`（防 SSRF）+ 5s 超时 + 失败只打日志
- `render_alert(event_row)`：级别图标（L1ℹ️/L2⚠️/L3🔴）+ 类型中文 + 时间 + 摄像头/track + 详情；三端（企微/钉钉/邮件及全部新渠道）统一调用
- `server/app/constants.py`：`TYPE_TEXT`（注明与 web/src/api.js 同步维护）+ `LEVEL_ICON`
- 配置：`settings.dingtalk_webhook/feishu_webhook/serverchan_sendkey/pushplus_token/qmsg_key/qmsg_qq`；
  `server/.env.example` 全量示例；docker-compose.yml 同步透传
- `server/tests/test_alerts.py`：13 个单测——文案各级别/未知回退、6 渠道 ×（MockTransport 发送成功 + 白名单拒绝/空配置零请求）
  说明：个人微信无官方机器人 webhook（OpenClaw"龙虾"推微信也是走企业微信 webhook），
  非官方 hook（GeWe 等）有封号风险故未采用，微信侧用 Server酱/PushPlus 落地

### 验收结果
- pytest 全绿（当时 22 passed，现随套件 31 passed）；`render_alert` 输出人工检查可读（图标/时间/摄像头/详情齐全）
- 网络请求全部走 `httpx.MockTransport`，零真实外发

### 遗留问题
- 各渠道未做真实 webhook 实测（无真实机器人 token）；协议字段按官方文档实现，白名单/降级逻辑已测
