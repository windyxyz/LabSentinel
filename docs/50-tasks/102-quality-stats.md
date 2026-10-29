# 任务 102 · 处置质量统计（误报率/未处置/L3，P2）

> 背景：模型止损线要求“误报 ≤20%/天”，但现在只能人工数事件。需要按时间/摄像头/类型统计
> 处置结果与误报率，作为模型迭代和发布验收的证据。

## 目标

### 1. `GET /api/stats/quality?days=7&camera_id=`

返回（`days` 1–365，默认 7）：
```json
{"days":7,"start_ts":"...","end_ts":"...","total":120,
 "by_status":{"new":10,"acked":80,"false_positive":20,"ignored":10},
 "by_level":{"1":5,"2":90,"3":25},
 "by_type":{"enter_no_ppe":60,"enter":50,"occupancy_over":10},
 "by_camera":[{"camera_id":1,"name":"...","total":70,"false_positive":12}],
 "false_positive_rate":0.18,
 "unhandled":10,
 "l3_total":25,
 "daily":[{"date":"2026-10-29","total":40,"false_positive":7}]}
```
- `false_positive_rate` 定义：`false_positive / (acked + false_positive + ignored)`（已处置为分母；
  无处置时为 0，并在文档写明）；不要把它当“每天误报次数”。
- 统计用 SQL 聚合（按 `ts` 范围），不加载全部事件；camera 名称 join；空数据返回 0/空数组。
- 读保护口径同其它 GET。

### 2. 前端 `web/src/views/Reports.vue`

- 新增「处置质量」卡片行：总事件/未处置/L3/误报率（百分比，保留 1 位）；
- 下方小表：按摄像头 total/false_positive/rate（最多 10 行，按 total 降序）；
- 与现有日/周切换共用 days；空数据显示“暂无数据”；
- 只改 Reports.vue/api.js，不新增路由。

### 3. 测试 `server/tests/test_quality_stats.py`

- 临时库造多摄像头/多状态/多类型/多日期事件：total、by_status/level/type、by_camera、daily 正确；
- 误报率公式（含 0 分母、只有 new）；days 边界/非法 400；camera_id 过滤；
- 不影响现有 stats 端点；鉴权口径。

### 4. 文档

- `docs/35-api-reference.md` 端点与指标口径；
- `docs/运维手册.md` 日周巡检加“误报率”；
- `docs/52-model-eval.md`/24h 清单交叉引用（作为真实运行质量证据）。

## 约束

- 只读聚合；不引入依赖；不改事件表；
- 指标口径必须写清（避免把比例当次数）；前端只改 Reports。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 临时库实测：造 10 条（2 误报/6 确认/2 未处置）→ 页面/端点 rate=0.25、unhandled=2；
- 报告附 SQL/结果对照。

## 执行报告

（待填：改动文件 / 指标对照 / 测试与 build / 遗留）
