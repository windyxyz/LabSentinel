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
