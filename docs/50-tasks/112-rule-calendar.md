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

**flash 2026-11-04 完成（pytest 409 passed = 402+7 新增；build gzip 55.92KB；8188 实机 PASS）**

### 1. 迁移 v6（store.py）

- `_mig_006`：rules ALTER 加 `days_of_week TEXT DEFAULT '1,2,3,4,5,6,7'`、
  `holiday_only INTEGER DEFAULT 0`（PRAGMA 防重入）；新建 `calendar_exceptions(date PK, mode, note)`；
  `_SCHEMA` 基线同步（新库/旧库两路收敛同构）；
- `Store.is_holiday(d)`：先查例外表（holiday/workday 覆盖），无记录按周末
  （`LS_WEEKEND_IS_HOLIDAY=0` 关闭"周末算假期"，config.py 新增）；`calendar_exceptions/
  set_calendar_exception（upsert 返回 replaced）/delete_calendar_exception` CRUD。

### 2. 引擎（rules.py）

- `_matches_calendar(rule, now)`：ISO 星期集合（缺列 None=全星期兜底；**空串/非法串 fail-closed
  不匹配**——坏数据不产生事件不抛异常）→ holiday_only 时要求 `is_holiday(today)`；
  match 判定顺序：星期 → 假期 → 时间窗（跨午夜语义不变）→ 冷却。冷却/级别语义未动。
- `add_rule` 持久化两新列；`store.rules()` 把 `holiday_only` 转 bool（与 enabled 同口径）。

### 3. API/UI

- `RuleCreate` 加 `days_of_week`/`holiday_only`（可选，默认全星期/非假期）；POST /api/rules
  校验 days ⊆ {1..7} 且非空（400）；
- `GET /api/calendar`（读保护）/ `POST`（admin，date 格式+mode 校验，同日期 upsert 返回
  `replaced`）/ `DELETE /api/calendar/{d}`（404）；
- Zones.vue：星期 7 复选（toggle 规范顺序存储）、仅假期勾选、规则表 muted 列追加
  "周X / 仅假期"摘要；页面底部「日期例外」小区块（日期/模式/备注+列表+删除）；
  api.js +3 方法。路由零新增。

### 4. 测试（test_rule_calendar.py，7 用例，注入日期不依赖墙钟）

星期过滤矩阵（周一规则×周一/周二、全星期、空串/非法串 fail-closed）、
holiday_only（工作日不触发/周末触发/workday 例外覆盖周末/holiday 例外覆盖工作日/
WEEKEND_IS_HOLIDAY=0）、时间窗+星期+假期组合、**旧库迁移默认值且旧行为不变**（手工建 v5 库
迁移后断言）、calendar CRUD/覆盖语义/格式校验/404/鉴权 401、规则创建校验与读回。

### 5. 实机验证（8188 临时实例+临时库）

`schema_version=6`；日历 upsert `replaced False→True`、列表 1 条；规则
`days_of_week="6,7", holiday_only=true` HTTP 创建+读回一致 → **PASS**。
事件级星期过滤由单测注入日期矩阵覆盖（触发矩阵见上 §4）。

### 6. 文档

35-api-reference（/api/calendar 三端点+规则新字段+判定顺序+occupancy 边界）、
部署FAQ「怎么设置只在夜间/周末/假期告警」、运维手册 §4.2 规则日历条件。

### 约束/遗留

- 现有规则 API 字段语义未变（新增可选字段）；无新依赖；前端只改 Zones.vue/api.js；
- **已知边界（记录不修）**：`occupancy_over` 全局规则当前不走时间窗/日历过滤（既有行为，
  112 规格只要求 match 路径）；如需统一，主线另立任务。
