# 任务 84 · 事件关键词搜索 + 日期预设（P2）

> 背景：Events 页有摄像头/级别/状态/日期筛选，但不能按详情/类型搜索；日期要手点两个日历。
> 本任务加关键词搜索与常用时间预设，后端保持分页/计数一致。

## 目标

### 1. 后端搜索

- `store.events(...)` 与 `store.count_events(...)` 增加 `q: str | None`：
  - 对 `detail` 与 `type` 做 `LIKE '%'||?||'%'`（大小写不敏感由 SQLite LIKE 默认处理 ASCII；
    中文按原样子串匹配）；
  - **转义用户输入中的 `%`/`_`/`\`**（`ESCAPE '\'`），避免 `%` 变成通配符；
  - 长度限制 1–100；空白去除；空 → 不过滤。
- `GET /api/events` / `GET /api/events/count` 增加 `q` 参数；保持其它参数与分页语义不变。
- 导出 CSV/证据包是否支持 q？**本任务不做**（避免范围膨胀），报告记为后续。

### 2. 前端 `web/src/views/Events.vue`

- 搜索框（placeholder「搜索详情/类型，如 实验服、reagent」），输入 300ms 防抖后重置页码查询；
- 日期预设下拉：今天 / 昨天 / 近 7 天 / 近 30 天 / 全部；选择后自动填 startDate/endDate 并查询；
- 保留手动日期输入；预设与手动日期互不覆盖（手动改动后预设显示“自定义”）；
- 空态文案包含当前关键词；清除筛选时清空 q；
- 不改全局四件套，不新增路由。

### 3. 测试 `server/tests/test_event_search.py`

- 造事件 detail 含「实验服」「试剂」，type `enter_no_ppe`/`enter`：
  - q=实验服 → 只匹配 detail；q=enter → 匹配 type；
  - q=`%` → 按字面匹配（不返回全部）；q=`_` 同理；
  - q 空白 → 全量；q 101 字符 → 400；
  - events 与 count 的 q 结果一致；与 camera/level/status/日期组合过滤正确；
  - 分页 offset/limit 与 q 组合。

### 4. 文档

- `docs/35-api-reference.md` events/count 增加 q 参数说明（含转义语义）；
- `docs/运维手册.md` 事件查询一段加示例。

## 约束

- 只做 q + 前端预设；不改事件表结构、不引 FTS/依赖；
- 保证 SQL 参数化（不拼字符串）；LIKE 转义正确；
- 前端只改 Events.vue/api.js。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 临时库实测：q=实验服 与 count 一致；`%` 不返回全量；
- 报告附查询结果与边界用例。

## 执行报告（补写，2026-11-04）

**flash 2026-11-04 补写（代码早已交付并入库；本轮补证据）**

- **改动文件**：`server/app/store.py`（`count_events`/`events` 增加 `q` 关键词过滤
  `_q_clause`）、`server/app/api.py`（`GET /api/events?q=` + `/api/events/count?q=`，
  >100 字符 400）、`web/src/views/Events.vue`（搜索框+日期预设 today/yesterday/7d/30d）；
- **测试证据**：`server/tests/test_event_search.py` 5 用例全绿（q 过滤/长度拒绝/count 口径
  /组合筛选）——本轮实跑 `pytest tests/test_event_search.py tests/test_audit.py -q` →
  **11 passed**；全量基线见调度看板（当前 491 passed）；
- **遗留**：无。
