

## 执行报告

### 交付物
- **21A 服务端**（stats.py 重构出 `_base_where` 公共过滤片段）：
  - `GET /api/stats/weekly?weeks=4&camera_id=&status=`：ISO 周聚合（周一为起点，SQLite
    `date(d,'-6 days','weekday 1')` 周一起点技巧，跨年正确；1-26 周，缺数据周补零，字段 week_start）
  - `GET /api/stats/summary?days=30&camera_id=&status=`：{total, by_level:{1,2,3}, by_type, top_days(≤5)}
  - `GET /api/stats/daily` 增强 start_ts/end_ts 自定义窗口（纯日期补边界；>400 天只返回有数据的天）
- **21B 前端**（Reports.vue）：顶部汇总卡片行（总数/三级别/最高事件日）；**日↔周视图切换**
  （周视图复用同一 SVG 折线几何，week_start 作 X 轴）；自定义起止日期范围（daily 走 19 的窗口参数）；
  样式全部沿用 theme.css 令牌（.cards/.card 既有类），无新增全局样式
- **文档**：35-api-reference.md 同步三个端点

### 验收结果
- `pytest tests/ -q`：**71 passed**（新增 4：周一边界聚合（同周归桶+缺周补零）/ summary 与 daily 同窗口
  各级别+总数吻合 / camera 过滤 / API 三端点与 daily 自定义窗口）
- `npm run build` 通过（gzip 41.5KB）

### 遗留问题
- 周视图暂无独立周表格（每日明细表仍为日粒度）；如需周明细表下轮加
- top_days 固定取 5 天（未做可配）
