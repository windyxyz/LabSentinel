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

**flash 2026-11-04 完成（pytest 402 passed = 397+5 新增；build gzip 54.88KB；8188 实测验收数字命中）**

### 1. 端点（server/app/stats.py `quality()` + api.py `/stats/quality`）

- 只读 SQL 聚合（COUNT/GROUP BY/SUM CASE，不加载事件行），窗口 `ts >= now-days` 至 `now`；
- 返回：total / by_status / by_level / by_type / by_camera（LEFT JOIN cameras 取名，含
  每路 rate，按 total 降序）/ daily / unhandled / l3_total / false_positive_rate；
- **口径**（docstring+API 文档双写）：`false_positive_rate = fp ÷ (acked + fp + ignored)`，
  分母=已处置（不含 new），无已处置=0；"每天误报次数"是 `daily[].false_positive`——两个口径
  在文档里明确区分，避免把比例当次数；
- `days` 1–365 越界 400；读保护 require_read 同口径；不影响现有 daily/weekly/summary/hourly。

### 2. 前端（Reports.vue + api.js，不新增路由）

- 汇总卡行下新增「处置质量」卡：未处置 / L3（窗口内）/ 误报率（%,1 位小数）/ 已处置÷总数；
- 按摄像头小表（最多 10 路：事件/误报/误报率）；窗口无数据显示"暂无数据"；
- 与日/周视图共用 `days`；**独立容错**：quality 请求失败（如旧服务端无此端点）只隐藏卡片，
  不影响既有图表（try/catch 包裹，seq 保护复用）。

### 3. 测试（test_quality_stats.py，5 用例，conftest 隔离 + api_mod.store 同源造数）

全字段聚合（11 事件跨 2 天 2 摄像头：by_status/level/type/by_camera/daily 与误报率 0.3）、
只有 new 时 rate=0 分母口径、空窗口零值、days 边界 0/365/366 与 camera_id 过滤、
读保护 401/令牌 200、既有 stats 端点回归。
**坑记录**：造数勿强制 `hour=10`——凌晨跑测试该时刻在"未来"，落不到窗口内（与
test_alert_silence 同类墙钟脆弱，已改为 now-days_ago 相对时间）。

### 4. 实机验证（8188 临时实例+临时库，恰按规格造 10 条：6 acked/2 fp/2 new）

`total=10、false_positive_rate=0.25、unhandled=2、l3_total=2`——与规格验收数字逐一命中；
`by_camera[0].rate=0.2`（该路 10 事件 2 误报）自洽。

### 5. 文档

- 35-api-reference §6 新端点+口径（含止损线"≤20 条/天"按 daily 口径的说明）；
- 运维手册每周巡检加"处置质量卡"（交叉引用标注手册/52-model-eval/24h 清单）；
- 摄像头24h验收清单 追加"误报率取证"节；52-model-eval 追加"上线后持续取证"附节。

### 遗留

- 无。**Phase A（发布前置）至此全部完成**：109/110/114/123/102/99/118 记录 ✓。
