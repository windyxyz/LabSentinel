# 任务 82 · 未处置事件升级提醒（P1）

> 背景：L3 事件产生后如果没人看，告警只发一次就沉底。需要“未处置 N 分钟后再提醒一次”的兜底，
> 避免严重事件被忽略；同时不能刷屏（每个事件只提醒一次，可配置）。

## 目标

### 1. 迁移 v3

`events` 增加：
- `escalated_ts TEXT`（首次升级提醒时间，NULL=未提醒）
- 索引可选 `idx_events_status_ts ON events(status, ts)`（加速未处置查询）。

### 2. 后台检查器 `server/app/escalation.py`

- 线程每 60s 检查一次（启动/停止与 retention 同模式，main lifespan 调用）；
- 查询 `status='new' AND escalated_ts IS NULL AND ts < now - escalate_after_min` 且 `level >= escalate_min_level`；
- 对每条：
  - 调用 dispatcher 发送一条**提醒**（复用现有渠道；文案 `render_alert` + 前缀 `⏰ 未处置提醒（已 N 分钟）`；
    若渠道全部未配置，只记日志不建投递行）；
  - 更新 `escalated_ts=now`（即使发送失败也标记，避免每分钟重试风暴；失败写日志）；
  - 最多每轮处理 `escalate_batch` 条（默认 20），防止启动时洪峰。
- 配置：
  - `LS_ESCALATE_AFTER_MIN=15`（0=关闭）
  - `LS_ESCALATE_MIN_LEVEL=3`（只提醒 L3；可设 2）
  - `LS_ESCALATE_BATCH=20`
- 纯函数 `find_unhandled(store, now, ...)` 便于测试；发送函数可注入。

### 3. API/UI（轻量）

- `GET /api/escalation/status`（读保护口径）：`{enabled, after_min, min_level, pending, escalated_today}`；
- Events.vue 状态列：`escalated_ts` 非空时显示「已提醒」小徽章（tooltip 时间）；不新增路由。

### 4. 测试 `server/tests/test_escalation.py`

- 临时库：造 3 条 new（1 条 L3 超时、1 条 L2、1 条已 acked、1 条已 escalated）→ 只选 L3 超时未提醒；
- 发送注入 stub：调用 1 次、`escalated_ts` 写入、再跑不重复；
- `escalate_after_min=0` 关闭；batch 上限生效；
- 渠道未配置时不建投递行（与 51 的 configured 语义一致）；
- `/api/escalation/status` 字段与 auth 口径。

### 5. 文档

- `docs/部署FAQ.md`「严重事件没人处理会怎样 / 怎么调提醒间隔」；
- `docs/35-api-reference.md` 新端点；
- `docs/运维手册.md` 日常巡检加“未处置提醒”说明。

## 约束

- 不改变事件状态机（提醒不等于处置）；不重复提醒同一事件；
- 不引入依赖；后台线程异常必须捕获，不能拖垮服务；
- 测试用临时 DB，不真等 15 分钟（注入 now/直接调用）。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 临时库：把 after_min 设 0/极小值，造 L3 事件→状态端点 pending 变化、escalated_ts 写入；
- 报告附查询/发送计数。

## 执行报告

**flash 2026-10-31 完成（pytest 215 passed、build 48.27KB、注入式发送实测）**

- **改动**：
  - 迁移 v3（events.escalated_ts + idx_events_status_ts）；新增 `server/app/escalation.py`
    （60s 线程/与 retention 同模式；`find_unhandled` 纯查询函数；`escalate_once` 支持 send 注入；
    提醒后写 escalated_ts 即使发送失败——防每分钟重试风暴；batch 防洪峰；
    `_dispatch_reminder` 复用渠道、未配置跳过不建投递行；异常全捕获）；main lifespan 启停；
    config 三项（after_min=15/min_level=3/batch=20）+.env.example；
  - API `GET /api/escalation/status`（require_read）；Events.vue 状态列 ⏰ 徽章
    （escalated_ts 非空，tooltip 时间）。
- **测试**（test_escalation.py 5 用例）：选行只含 L3 超时未处置未提醒（L2/已处置/新事件排除）；
  send 注入——一次调用+escalated_ts 写入+再跑 checked=0（105 空帧教训：状态必须清）；
  after_min=0 关闭、batch=1 生效；渠道未配置仍标记（防风暴语义）+不建投递行；status 端点字段。
  全量 **pytest 215 passed**；build gzip **48.27KB**。
- **文档**：部署FAQ「严重事件没人处理会怎样/怎么调提醒间隔」；35-api-reference status 端点；
  运维手册 §12 巡检项。
- **遗留**：无。
