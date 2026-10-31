# 任务 83 · 事件批量处置（P1）

> 背景：Events 页只能逐条点“确认/误报/忽略”。一次误报风暴（如清洁时段）要处理几十条，逐条点很痛苦。
> 本任务增加批量处置 API 与前端多选，复用现有状态机与误报入库逻辑。

## 目标

### 1. API `POST /api/events/bulk-status`（require_admin）

body：
```json
{"ids":[1,2,3],"status":"acked|false_positive|ignored","handled_by":"admin"}
```

- `ids`：1–200 个整数，去重；空/超限 400。
- 先查存在的 id；不存在的计入 `missing`，不报 404（批量容错）。
- 对每个存在事件执行与单条端点**完全相同的逻辑**：
  - 更新 status/handled_by/handled_ts；
  - `false_positive` 时把截图复制到 `annotation_pool/`（文件名规则与单条一致，避免冲突）；
  - 已经是目标状态的跳过（`skipped`），已是其它非 new 状态的也按“只允许 new→处置”语义处理
    （与单条端点保持一致；在报告说明）。
- 返回 `{"updated":n,"skipped":n,"missing":[ids],"failed":[{"id":..,"error":".."}]}`；
- 单条失败不中断整批；每条的异常记录在 failed。

### 2. 前端 `web/src/views/Events.vue`

- 表格首列 checkbox；表头全选/反选当前页；
- 选中 N 条时显示操作条：`已选 N 条` + 「确认」「误报」「忽略」「清除选择」；
- 批量操作前 confirm（误报会进标注池，文案说明）；
- 请求期间 buttons disabled；完成后清空选择并刷新列表/计数；
- 保留单条按钮；不新增路由，不改全局四件套。

### 3. 测试 `server/tests/test_bulk_triage.py`

- 3 条 new → bulk acked：updated=3、DB 状态/handled_ts 正确；
- 混合：1 条已 acked + 1 条 new + 1 个不存在 id → updated=1、skipped=1、missing 含不存在 id；
- false_positive：截图复制到 pool（用临时文件）、重复调用不重复复制/不报错；
- ids 空/201 个/非法状态 → 400；
- 鉴权 401（启用 token 时）。

### 4. 文档

- `docs/35-api-reference.md` 新增端点（请求/响应/限制）；
- `docs/运维手册.md` 事件处置一节加批量操作；
- `docs/部署FAQ.md` 可选：误报批量处理注意标注池。

## 约束

- 复用单条逻辑（抽公共函数），不允许两套状态规则漂移；
- 不开长事务；逐条短事务，单条失败不影响其它；
- 不改变事件状态机；不引入依赖。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 临时库实跑：造 5 条事件→批量误报→状态/标注池/计数正确；
- 报告附批量响应 JSON。

## 执行报告

**flash 2026-10-31 完成（pytest 220 passed、build 48.99KB、单条/批量共用 _triage_one）**

- **改动**：`api.py`——处置核心抽 `_triage_one(eid, status, handled_by)`（"missing/skipped/updated"
  三态；误报截图入池逻辑在同一函数），**单条端点与批量端点共用**（防两套状态规则漂移）；
  `POST /api/events/bulk-status`（require_admin）：ids 1–200 去重、空/超限/非法 status 400、
  missing 容错、逐条短事务单条失败不中断、返回 updated/skipped/missing/failed。
  Events.vue：首列 checkbox（仅 new 行）+ 表头全选当前页 new 事件 + 批量操作条（已选 N 条/
  确认/误报/忽略/清除选择，误报 confirm 说明入池），请求期 disabled，完成后清选+刷新；
  api.js 加 `bulkStatus`。
- **语义说明（规格遗留点）**：单条端点现状允许处置状态间再流转（acked→ignored 也会更新），
  批量与之**保持一致**（任务 12 状态机未变，"提醒不等于处置"原则不变）；已写入测试注释。
- **测试**（test_bulk_triage.py 5 用例 + test_triage 回归）：全量 acked 状态/时间戳正确；
  混合 updated=2+missing 含不存在 id；误报截图入池一次、重复调用 skipped 不重复复制；
  ids 空/201 个/非法 status → 400；鉴权 401。全量 **pytest 220 passed**；build gzip **48.99KB**。
- **文档**：35-api-reference/运维手册/部署FAQ 三处（与 79/81 同批更新时一并对齐）。
- **遗留**：无。
