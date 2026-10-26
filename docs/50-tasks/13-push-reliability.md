# 任务 13 · 推送可靠性（先读本目录 README.md）

> 规划依据：60-milestones M3。现状：6 个外部渠道 fire-and-forget，失败只打印日志——告警漏发不可查、不可重试。

## 背景

实验室安全告警"必达"是产品底线。当前 `_HttpPushChannel._post` 单次尝试、失败静默。需要：重试、死信、可观测。

## 目标（服务端为主）

### 13A 投递记录与重试
- 新表 `alert_deliveries(id, event_id, channel, status, attempts, last_error, updated_ts)`
- `AlertDispatcher.dispatch`：每个外部渠道写一条 deliveries（pending → ok / failed），失败按 **三次指数退避**（1s/4s/15s，线程池内）重试
- 重试仍失败 → 状态 failed（死信），可经 API 手动重发：`POST /api/alert-deliveries/{id}/retry`
- 渠道发送结果判定：HTTP 2xx 且响应体不含错误码（钉钉/企微返回 body 里 errcode 非 0 也算失败——各渠道解析函数写进对应类 `_is_ok(resp)`）

### 13B 可观测
- `GET /api/alert-deliveries?status=failed&limit=50` 列表
- `/api/system` 增加 `alerts: {pending: n, failed: n}` 计数
- Web 仪表盘：失败告警数卡片（红色，点击跳事件页筛选）

### 13C 渠道健康自检（轻量）
- `POST /api/alert-test`：向所有已配置渠道发一条测试消息（标记 event_id=0，不落 events 表），返回各渠道结果——部署验收用

## 约束

- 重试在线程池中执行，**不得阻塞 worker**（沿用 09 任务的异步化成果）
- deliveries 表写入走 Store 的锁；**不得**把 deliveries 写进 events
- 域名白名单/https/禁重定向逻辑不动

## 验收标准

pytest 全绿 + 新增 ≥5 测试（重试次数/退避可注入 time mock/errcode 解析/失败标记/手动重发）；MockTransport 全程不真发；API 文档同步。

## 执行报告

### 交付物
- **13A 投递记录与重试**：
  - `store.py`：`alert_deliveries` 表（event_id/channel/status/attempts/last_error/updated_ts）+
    add/update/list/get/counts 方法（带锁写）
  - `alerts.py`：各渠道 `send() -> bool`（未配置/URL 拒绝/网络异常/HTTP 非 2xx/业务码错误 → False）；
    **`_is_ok(resp)`** 静态判定（2xx 且 errcode=0 或 code∈{0,200}，非 JSON 2xx 视为成功）；
    `AlertDispatcher._deliver_with_retry`：每渠道一条 deliveries（pending→ok/failed），失败按
    **RETRY_BACKOFF=(1s,4s,15s)** 三次重试，仍失败置 failed（死信）并打日志；重试在线程池内（不阻塞 worker）
- **13B 可观测**：`GET /api/alert-deliveries?status=&limit=`（≤200）；`/api/system` 增加
  `alerts: {pending, failed, ok}`；Web 端"失败告警数卡片"随任务12 Dashboard 卡片区一起做（见 12B 报告，仪表盘已加未处置卡；失败告警卡见下）
- **13C 渠道自检**：`POST /api/alert-test` → 向全部 7 渠道发测试消息（event_id=0 不落 events），
  返回逐渠道 ok/error；结果同样记入 deliveries
- **死信重发**：`POST /api/alert-deliveries/{id}/retry` → 按原渠道取原事件文案重投（event_id=0 则发测试文案），
  状态先置 pending 再异步重跑三次退避

### 验收结果
- `pytest tests/ -q`：**51 passed**（新增 5：三次失败进死信 attempts=4 / errcode 判定四态 /
  第三次成功 attempts=3 / dispatch 异步+死信计数 / 自检端点+列表+重发404+system 计数字段）
- 全程 httpx.MockTransport，无真实外发；退避常量注入 (0,0,0) 保证测试速度
- 白名单/https/禁重定向逻辑未动；send 签名变化（返回 bool）已同步全部渠道

### 遗留问题
- 死信无自动过期清理（保留 180 天策略暂未覆盖 deliveries 表；任务14 的 retention 可扩）
- Web"失败告警卡片"待 Dashboard 下轮统一布局时补（API 字段已就绪）
- EmailChannel 抛错也算一次失败尝试（SMTP 慢失败会占满退避窗口 ~20s，线程池 2 worker 可能短时排队）
