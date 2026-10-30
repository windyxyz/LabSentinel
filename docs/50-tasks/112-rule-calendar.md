# 任务 112 · 规则日期/星期/假期条件（P1）

> 背景：现有规则只有每日时间段（time_start/time_end），无法表达“仅工作日”“仅周末”“法定假期另行告警”
> 等实验室管理要求。本任务给规则加星期/假期条件，并加一张可维护的日期例外表。

## 目标

### 1. 迁移 v6

- `rules` 增加：
  - `days_of_week TEXT DEFAULT '1,2,3,4,5,6,7'`（ISO 1=周一 … 7=周日）；
  - `holiday_only INTEGER DEFAULT 0`（1=只在假期日期生效）。
- 新表：
  ```sql
  CREATE TABLE calendar_exceptions(
    date TEXT PRIMARY KEY,   -- YYYY-MM-DD
    mode TEXT NOT NULL,      -- holiday | workday
    note TEXT
  );
  ```
- `Store.is_holiday(date) -> bool`：先查 exceptions（holiday=true/workday=false），无记录按周末（Sat/Sun）判断；
  可通过 `LS_WEEKEND_IS_HOLIDAY=0` 关闭“周末算假期”（默认 1）。

### 2. 规则引擎

- `RuleEngine` 在时间窗判断前增加：
  - 当前 ISO weekday 是否在 `days_of_week`；
  - `holiday_only=1` 时 `is_holiday(today)` 必须为真；`holiday_only=0` 时假期不特殊（仍按时间/星期）。
- 规则匹配失败静默跳过（不产生事件）；不改变冷却/级别语义。
- `RuleCreate` 增加两个字段（默认全星期/非假期专用）；`add_rule` 持久化。

### 3. API/UI

- `GET/POST/DELETE /api/calendar`（require_admin 写，读保护口径读）：日期/模式/备注；
  日期格式校验、重复日期 409/覆盖语义（定义并测试）。
- Zones.vue 规则表单：星期多选（7 个 checkbox）、“仅假期”勾选；页面底部「日期例外」小区块
  （添加日期/模式/备注、列表、删除）。
- 不新增路由。

### 4. 测试 `server/tests/test_rule_calendar.py`

- 周一规则 + 周二事件 → 不触发；周二规则 + 周二 → 触发；
- holiday_only=1：普通工作日不触发、holiday 日期触发；workday 例外覆盖周末；
- 时间窗 + 星期 + 假期组合；
- 迁移旧库默认值（全星期/非假期）且旧规则行为不变；
- calendar CRUD/格式校验/鉴权。

### 5. 文档

- `35-api-reference`；`部署FAQ`「怎么设置只在夜间/周末/假期告警」；`运维手册` 规则配置一节。

## 约束

- 不改变现有规则 API 字段语义（新增可选字段）；不引入依赖；
- 日期按本机本地时区（与事件 ts 一致）；跨日时间窗（22:00-06:00）保持现有语义；
- 前端只改 Zones.vue/api.js。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 临时库：造周一/周二事件验证星期过滤；报告附触发矩阵。

## 执行报告

（待填：改动 / 触发矩阵 / 测试与 build / 遗留）
