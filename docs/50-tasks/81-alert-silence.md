# 任务 81 · 告警静默/维护窗口（P1）

> 背景：设备维护、实验操作、清洁时会持续触发告警；现在只能停摄像头或关渠道。需要“静默一段时间”
> 的能力：检测/事件照常记录，但**不推送**；到期自动恢复。配置需持久化，重启不丢。

## 目标

### 1. 表与迁移（沿用任务 69 框架，v2）

```sql
CREATE TABLE alert_silences(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  scope TEXT NOT NULL,            -- global|camera|level|type
  target TEXT,                    -- camera_id / level / type；global 为 NULL
  reason TEXT,
  created_ts TEXT NOT NULL,
  until_ts TEXT NOT NULL,
  created_by TEXT
);
CREATE INDEX idx_silence_until ON alert_silences(until_ts);
```

### 2. 匹配与派发

- `alerts.dispatcher` 在投递前检查 `is_silenced(event)`：
  - 取所有 `until_ts > now` 的静默；scope 匹配规则：
    global 全匹配；camera 匹配 `str(event.camera_id)`；level 匹配 `str(event.level)`；
    type 匹配 `event.type`；
  - 命中 → **不创建投递行、不重试、不发任何渠道**；记录一条 INFO 日志（含 silence id/event id）；
  - 事件本身照常入库/WebSocket 推送（静默只针对外部渠道；如需连 WS 也静默，在报告说明并默认不做）。
- 导出/测试用纯函数 `match_silence(silences, event, now)` 便于单测。

### 3. API（require_admin 写，读按现有口径）

- `GET /api/alert-silences?active=1`：返回列表（含 `remaining_s`）。
- `POST /api/alert-silences`：`{scope,target,duration_s,reason}`；duration 60–7*86400，默认 3600；
  校验 scope/target（camera 必须存在；level ∈ 1..3；type ∈ 现有事件类型）；返回记录。
- `DELETE /api/alert-silences/{id}`：提前解除。
- 所有写操作进审计（若 87 审计已就绪；否则留 TODO 注释）。

### 4. 前端（`web/src/views/Cameras.vue` 告警面板，或 Dashboard）

- 显示当前生效静默（scope/target/剩余时间/原因）；
- 快捷按钮：「全局静默 1h」「本摄像头静默 1h/4h/24h」；
- 自定义：scope/target/时长/原因 + 添加；列表项可「解除」；
- 无静默时显示“无静默”；失败显示 detail；不改全局四件套。

### 5. 测试 `server/tests/test_alert_silence.py`

- match_silence：global/camera/level/type 命中与不命中、过期不命中、多条取任一；
- dispatcher：静默时发送函数**零调用**、无投递行；未静默正常；
- API：创建/列表/删除、duration 边界、invalid scope/target/camera 400、auth 401；
- 迁移：旧库升级到 v2 后表存在（与 69 测试风格一致）。

### 6. 文档

- `docs/35-api-reference.md` 新增端点；
- `docs/部署FAQ.md`「维护时怎么临时不告警 / 事件还会记录吗」；
- `docs/运维手册.md` 告警排障一节加静默说明。

## 约束

- 静默不改变检测/事件入库/截图；只抑制外部渠道；
- 过期记录可保留（不自动删），列表默认只显示 active；
- 不引入依赖；前端只改一个 view。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 临时库：创建静默→触发事件→零投递；删除/到期→恢复投递；
- 报告附静默前后投递计数。

## 执行报告

（待填：改动文件 / 静默前后证据 / 测试与 build / 遗留）
