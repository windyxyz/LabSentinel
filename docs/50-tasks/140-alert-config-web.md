# 任务 140 · 网页端告警渠道配置（P1）

> 背景：告警渠道（企业微信/钉钉/飞书/Server酱/PushPlus/Qmsg/邮件/通用 Webhook）原本只能改
> `server/.env` 后重启；对现场部署不友好。本任务做成网页表单：按渠道指引填写、保存即生效、
> 可单渠道发测试，密钥不回显。

## 功能

- 新增「告警配置」页（导航栏），8 个渠道卡片：状态/来源（网页保存/`.env`/未配置）、字段、指引、
  保存、发送测试、清除（回退 `.env`）。
- 配置写入本机 SQLite `app_settings`（迁移 **v10**），随数据库备份；保存时 `setattr(settings)`
  即时生效，无需重启。
- API（全部 `require_admin`）：
  - `GET /api/alerts/config`：返回各渠道 `configured/source` 与非密钥字段；webhook 只返回掩码，
    密钥只返回 `has_*` 布尔；
  - `PUT /api/alerts/config`：部分更新，空值=保持；校验域名/https、端口、邮箱、QQ、Webhook 凭证；
  - `POST /api/alerts/config/reset?channel=`：清除数据库覆盖，回退启动时 `.env`；
  - `POST /api/alert-test?channel=wecom`：单渠道测试（无参数=全部，兼容旧 UI）。
- 指引内置：企业微信/钉钉/飞书群机器人、Server酱/PushPlus/Qmsg、QQ/163 SMTP 授权码、通用
  Webhook HMAC；DingTalk 明确「用自定义关键词 LabSentinel（当前不支持加签）」。

## 安全

- 仅管理员可读写；密钥/完整 webhook 不回显；日志/响应不含明文；
- 保存前校验：官方渠道仅 https + 域名白名单，通用 webhook 拒绝 `user:pass@`、http 需显式
  `allow_http`；SMTP 主机/端口/收件邮箱格式；Qmsg Key+QQ 必须同时存在；
- 不清空 `.env`；「清除」只删除 DB 覆盖并恢复启动时值。

## 测试

- `server/tests/test_alert_config.py`（7 项）：掩码/来源、保存即时生效+DB 持久化+空值保持、
  非法地址/字段/端口/邮箱/QQ/http webhook 拒绝、reset 回退、单渠道测试过滤、v10 迁移、
  API 直调 400。
- `pytest tests/` 全量 + `npm run build`；真实页面以隔离实例手测（保存/测试/清除）。

## 约束

- 不改变告警发送/重试/静默语义；不引入依赖；
- 密钥只存本机 DB，不写回 `.env`；不把密钥写进报告/日志。

## 执行报告

**主线 2026-11-04 完成**

- 后端：`alert_config.py`（字段映射/掩码/校验/DB 覆盖/reset）、`store.py` v10
  `app_settings`、`main.py` 启动加载、`api.py` 3 个端点 + `alert-test?channel=`、
  `alerts.py` 单渠道测试过滤。
- 前端：`AlertSettings.vue`（8 渠道卡片/指引/状态/保存/测试/清除）、`api.js` 方法、`App.vue` 导航。
- 文档：任务 140 + README 索引；部署指南/FAQ 后续补充入口。
- 测试：`test_alert_config.py` 7 项；`pytest tests/test_alert_config.py` 全绿；
  `npm run build` 通过（gzip 66.59KB）。
- 说明：密码/Webhook 在 UI 只显示“已配置”或掩码；留空保存保持原值；清除回退 .env。
- **端到端**（隔离 8013 + 本地 18099 接收器）：保存通用 Webhook（http+allow_http）→
  GET 返回掩码/`has_url` → 单渠道测试收到 `version=1/title=LabSentinel 级别1告警/level=1/source=labsentinel`
  → reset 回退为未配置；面板 `/` 与 `index-QAQ4KPo_.js` 200。
