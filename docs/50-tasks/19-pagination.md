# 任务 19 · 事件分页与查询性能（先读本目录 README.md）

## 背景

`GET /api/events` 只有 limit，无 offset；前端事件页一次性拉 50 条。长期运行后需要翻页与按时间范围筛选。

## 目标

### 19A 服务端
- `GET /api/events` 增加参数：`offset=0`、`start_ts=`、`end_ts=`（ISO 日期或完整时间，含边界）
- 响应增加总数：`GET /api/events/count?camera_id=&level=&status=&start_ts=&end_ts=` → `{"count": n}`（或 events 返回 `{"items": [...], "total": n}`——**兼容性优先，选新增 count 端点**，events 结构不变）
- SQL 全部参数化（铁律），时间比较用 ts 字符串前缀/范围（索引已存在）
- limit 上限 200

### 19B 前端
- Events.vue：上一页/下一页（offset 累加）+ 显示总数；日期范围（两个 date input 转 ts 参数）；筛选变化重置 offset
- 样式用 scoped，不改全局

## 验收标准

pytest 全绿 + 新增 ≥4 测试（offset 翻页、时间范围边界、count 一致性、limit 上限钳制）；`npm run build` 通过；API 文档同步。

## 执行报告

### 交付物
- **19A 服务端**：`GET /api/events` 新增 `offset/start_ts/end_ts`（limit 钳制 1-200；
  纯日期自动补 `T00:00:00`/`T23:59:59` 含边界）；新增 `GET /api/events/count`（同过滤条件总数，
  **兼容性优先**：events 返回结构不变）；`store.events()` 重构为单条参数化 SQL（where 动态拼接，
  offset 用 LIMIT ? OFFSET ?）；`store.count_events()` 同条件计数。SQL 全参数化（铁律）
- **19B 前端**：Events.vue 上一页/下一页（offset 累加，边界禁用）+"第 x-y 条 / 共 n 条"计数显示
  （eventsCount 端点）；日期范围两个 date input；**筛选变化重置 offset**；样式 scoped
- **文档**：35-api-reference.md 同步新参数与 count 端点

### 验收结果
- `pytest tests/ -q`：**67 passed**（新增 4：offset 翻页无缝 / 时间范围边界（纯日期与完整时间+空窗）/
  count 与 events 行数一致性（三种过滤）/ limit 钳制后取全）
- `npm run build` 通过（gzip 40.8KB）
- 查询路径：ts 列已有 idx_events_ts 索引；动态 where 全部占位符

### 遗留问题
- count 每次翻页都查询（SQLite 本地库开销可忽略；远端化时再加缓存）
- 时间范围未做格式强校验（非法格式静默无匹配，行为安全）
