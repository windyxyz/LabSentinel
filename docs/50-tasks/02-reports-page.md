# 任务 02 · 统计报表（先读本目录 README.md）

## 背景

系统已能产生事件（SQLite `events` 表：ts/camera_id/type/level/track_id/detail/snapshot）和实时在场人数（管线内部）。管理员需要按天/摄像头查看统计趋势，用于安全汇报。

## 目标

1. 服务端新增统计 API；2. Web 端新增"报表"页（折线图+柱状图+表格）。

## 服务端要求（server/app/）

新增 `stats.py` 模块 + `api.py` 挂路由（沿用现有 Store 风格，字面量 SQL + 参数占位符，禁止字符串拼接 SQL）：

- `GET /api/stats/daily?days=7&camera_id=` → `{"days": [{"date": "2026-10-25", "total": 12, "l1": 3, "l2": 7, "l3": 2, "enter": 5, "enter_no_ppe": 4, "exit": 3}]}`
- `GET /api/stats/hourly?date=2026-10-25&camera_id=` → 24 小时事件分布（同上结构，hours 数组）
- 时间解析用 ts 字段的前缀（ISO 格式 `YYYY-MM-DDTHH:MM:SS`），SQLite 用 `substr(ts,1,10)`/`substr(ts,12,2)` 分组即可

## Web 端要求（web/src/）

- 新增 `views/Reports.vue`，并在 `App.vue` 的 tabs 数组注册（label: "报表"）
- 图表**不引入 ECharts 等重型库**，用原生 SVG 画简单折线/柱状（项目体积敏感，现有 bundle gzip 32KB）
- 控件：天数选择（7/30 天）、摄像头筛选；表格列出每日明细
- 复用 `api.js` 的 req 封装

## 验收标准

1. `pytest tests/ -q` 全绿；为 stats 写 ≥3 个单测（空库、多天多级、camera 筛选）
2. `npm run build` 通过
3. 手动：启动服务端后 `GET /api/stats/daily` 返回合法 JSON（库里有演示数据）

## 执行报告

### 交付物
- `server/app/stats.py`：`daily(store, days, camera_id)` / `hourly(store, date_str, camera_id)`，
  字面量 SQL + 参数占位符，`substr(ts,1,10)`/`substr(ts,12,2)` 分组，缺数据的天/小时补零，
  days 上限 90，date 参数非法抛 ValueError（API 层转 400）
- `server/app/api.py`：`GET /api/stats/daily?days=7&camera_id=`、`GET /api/stats/hourly?date=&camera_id=`
- `web/src/views/Reports.vue` + App.vue 注册"报表" tab + api.js 的 statsDaily/statsHourly（复用 req 封装）
- 图表为**原生 SVG**（720x220 viewBox）：每日趋势折线（合计+L1/L2/L3 四条线）、24 小时分布柱状图、
  每日明细表（类型列动态按数据出现的行为类型生成）。控件：7/30 天、摄像头筛选、小时分布日期选择
- `server/tests/test_stats.py`：3 个单测（空库补零 / 多天多级别聚合 / 小时分布+摄像头筛选+非法日期）

### 验收结果
- `pytest tests/ -q`：31 passed（当时 9 passed，套件后续随其他任务扩充）
- `npm run build`：通过；bundle gzip 33.7KB（原 32KB，+1.7KB 无重型依赖）
- 实测（隔离实例 + 演示视频产生真实事件）：`/api/stats/daily` 返回 7 行合法 JSON，
  当日行 `{"date":"2026-10-26","total":2,"l1":1,"l2":1,"l3":0,"exit":1,"enter":1}`；hourly 24 行正确

### 遗留问题
- 报表页小时分布默认查今天，跨日后需手动选日期（未做自动跟随，属交互打磨）
