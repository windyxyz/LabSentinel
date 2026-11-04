# 任务 136 · 事件时间线视图（P3）

> 来源：Frigate review 时间线（建议 #16）。目标是把“某天/某小时哪里事件密集”一眼看出来，
> 点击可跳到对应事件；不是替代现有表格/详情。

## 后端

1. 新增 `GET /api/events/timeline?days=7&camera_id=&level=`（`require_read`）：
   - 按 `date(ts)` + `hour` 分桶：`[{date:"2026-11-04", hours:[{hour:0,total:0,l3:2},...]}]`；
   - days 1–30（默认 7）、camera/level 可选；单次最多返回 31 天；
   - SQL group by substr(ts,1,10), substr(ts,12,2)；空数据返回 `days` 列表全 0。
2. 测试：空库/多天/摄像头过滤/level 过滤/权限；数字与 `/events/count` 对拍。

## 前端（Events.vue）

1. 表格上方加“时间线”折叠条：每行一天，24 个小时格；颜色按 total（0=灰、1–2=蓝、3–9=橙、≥10=红）；
   L3 数量用角标；点击小时格 → 设置 start/end 到该小时并刷新列表。
2. 图例/空状态/加载态；不引入图表库（div grid）。
3. 可选：显示当天事件缩略图（复用 snapshot URL，懒加载）——若实现成本高则只做色块。

## 验收

- API 返回与 count 对拍；时间线点击后列表时间范围正确；
- `npm run build` + 后端 pytest 全绿；375px 不溢出（横向滚动可接受）；
- 报告附 API 样例与截图/说明。

## 约束

- 只读聚合，不新增表/迁移；不改现有事件查询语义；
- 深色/浅色主题都要可读（用 CSS 变量）。

## 执行报告

**flash 2026-11-04 完成（pytest 502 passed = 499+3 新增；build gzip 63.03KB；count 对拍在测试内）**

### 后端（stats.timeline + GET /api/events/timeline）

- `timeline(store, days, camera_id, level)`：SQL `GROUP BY substr(ts,1,10), substr(ts,12,2)`
  单次聚合（不加载事件行）；days 窗口逐日 24 格补零（空数据全 0）；每日 total + 每格 l3
  （`SUM(level>=3)`）；返回 `{days, days_list:[{date,total,hours:[{hour,total,l3}×24]}]}`；
- `GET /api/events/timeline`（require_read）：days 1-30（越界 400）；camera/level 可选叠加。

### 前端（Events.vue，无图表库）

- 表格上方 `<details>` 折叠条「事件时间线（近 7 天）」：每行一天（日期 + 24 格 grid + 日合计）；
  色阶按 total（0=灰 divider 变量 / 1-2 蓝 / 3-9 橙 / ≥10 红），L3 红角标；CSS 变量适配双主题；
  tooltip 显示日期小时+条数+L3；**点击非空小时格 → startDate/endDate 设为该日 + resetAndLoad**
  （跳到该天列表）；图例/加载态/横向滚动（375px min-width 520px）；
- 懒加载：首次展开才拉（@toggle handler），失败静默（增强不影响表格）。

### 测试（test_event_timeline.py，3 用例）

分桶正确（今天 10 点 2 条/1 L3、昨天 23 点）+ 72 格补零 + 空日全零；camera/level 过滤 +
**与 count_events 对拍**；API 流程 + days 边界 0/31/30 + 401。

### 遗留

- 缩略图懒加载未做（规格"可选、成本高则只做色块"——色块已满足目标，缩略图随 137/backlog）。

### 主线复审补丁（2026-11-04）

- 原点击小时格只把 start/end 设为该“日”（后端补 00:00–23:59），实际筛整日而非小时；
  改为 `hourFilter` 精确 `dateT hh:00:00 ~ hh:59:59`，筛选条显示当前小时并可清除；
  导出 URL 同步。
- 后端时间线起点改为 d0 当天 00:00（原 now-7d 会多查一天边界数据），查询窗口与返回
  的自然日完全一致。
