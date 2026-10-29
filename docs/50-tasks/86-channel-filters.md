# 任务 86 · 告警渠道按级别/类型过滤（P2）

> 背景：不同渠道用途不同（短信/电话只收 L3，群机器人收全部）。现在所有已配置渠道都收所有事件，
> 造成噪音和费用。本任务允许每个渠道配置最低级别与事件类型白名单。

## 目标

### 1. 配置（简单、可验证）

- `LS_ALERT_FILTERS`：JSON 字符串，例如：
  ```json
  {"wecom":{"min_level":2,"types":["enter_no_ppe"]},
   "email":{"min_level":3},
   "webhook":{"types":["enter_no_ppe","occupancy_over"]}}
  ```
  - 键为渠道 name（wecom/dingtalk/feishu/serverchan/pushplus/qmsg/email/webhook）；
  - `min_level` 1–3（可选）；`types` 事件类型数组（可选；空数组=不限制？定义为“空=不限”，并在文档写明）；
  - 非法 JSON/未知渠道/非法级别 → 启动时 WARNING 并忽略该条（不崩）；`config.py` 提供解析函数
    `alert_filters() -> dict`，缓存结果。
- 默认 `{}` = 所有渠道收所有事件（行为不变）。

### 2. 派发过滤

- dispatcher 在 `_deliver`/`_deliver_with_retry` 前按渠道名检查：
  - `event.level < min_level` → 跳过（不建投递行、不重试、不产死信，与“未配置”语义一致）；
  - `types` 非空且 `event.type` 不在其中 → 跳过；
  - 记录 INFO 日志（渠道/event/原因），不写 DB。
- `/api/alerts/status` 每渠道增加 `filters` 字段（只回显配置，不含敏感值）；
- `alert-test` 不受过滤影响（测试消息应能发到任意已配置渠道）；在报告说明。

### 3. 测试 `server/tests/test_channel_filters.py`

- 解析：合法/非法 JSON/未知渠道/级别越界/大小写；
- 过滤：wecom min_level=3，L2 事件→零调用、L3→调用；types 白名单匹配/不匹配；
- 未配置过滤 → 全部调用（回归）；
- 跳过时不产生投递行/死信；
- `/api/alerts/status` 回显 filters；alert-test 仍调用。

### 4. 文档

- `.env.example` 注释 + 示例；
- `docs/部署指南.md` 告警章节加过滤示例；
- `docs/部署FAQ.md`「怎么让短信只收 L3 / 群只收未穿实验服」。

## 约束

- 不改渠道发送协议/重试；不引入依赖；
- 过滤只影响外部渠道，WebSocket/事件入库不受影响；
- 非法配置不能导致服务启动失败。

## 验收

- `pytest tests/ -q` 全绿；
- 8001 + 本地 stub：配置 wecom min_level=3，L2 不发、L3 发；status 显示 filters；
- 报告附配置/调用计数。

## 执行报告

（待填：改动文件 / 过滤证据 / 测试 / 遗留）
