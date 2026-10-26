# 任务 15 · 报表导出（先读本目录 README.md）

> 规划依据：60-milestones M3（安全汇报场景：月度安全报告要能离线交给学校/院系）。

## 目标

### 15A CSV 导出（服务端，必做）
- `GET /api/export/events.csv?days=30&camera_id=&status=` → `text/csv`（UTF-8 带 BOM，Excel 中文不乱码）
  - 列：时间, 摄像头, 类型(中文), 级别, track, 详情, 状态, 处理人, 处理时间
- `GET /api/export/stats.csv?days=30` → 每日统计表（date,total,l1,l2,l3,各行为类型）
- 流式返回（事件多时不一次性载入内存：生成器 + StreamingResponse）

### 15B PDF 月报（可选，做好 A 再做）
- `GET /api/export/monthly.pdf?month=2026-10` → 一页式安全月报：
  - 标题/统计数字（总事件、各级别、未处理数）/每日柱状（SVG 内嵌）/Top 规则触发/合规声明脚注
- 实现方式二选一（执行报告说明理由）：reportlab（纯 Python）或 服务端渲染 HTML → 已有前端？**不得引入 wkhtmltopdf 等外部二进制**。若 reportlab 中文需 TTF，用系统 `C:\Windows\Fonts\msyh.ttc`（路径可配 `LS_PDF_FONT`）。
- 内容脱敏：不含人脸照片；截图不放 PDF（合规）

### 15C Web 端
- Reports.vue 增加"导出 CSV""导出月报 PDF"按钮（直接 `<a href>` 下载即可，无需前端处理流）

## 约束

- 导出接口只读，不写库；大查询分页生成
- 中文列名固定（Excel 用户视角），代码里用常量表，禁止散落字符串

## 验收标准

pytest 全绿 + 新增 ≥3 测试（CSV 列数与内容、BOM 存在、days 边界钳制）；PDF 可打开且中文正常（截图为证）；API 文档同步。

## 执行报告

### 交付物
- **15A CSV**：`GET /api/export/events.csv?days=&camera_id=&status=` —— 生成器分页（每批 200，
  id 游标）+ StreamingResponse，**UTF-8 带 BOM**（Excel 直接打开中文不乱码）；列名固定常量表
  `CSV_EVENT_HEADERS`；类型用 constants.TYPE_TEXT 中文化、状态中文化。
  `GET /api/export/stats.csv?days=` 每日统计表。days 钳制 1-366
- **15B PDF**：`GET /api/export/monthly.pdf?month=YYYY-MM` —— **reportlab**（纯 Python，选它因为
  不引外部二进制；HTML 渲染方案需 headless 浏览器，排除）。中文字体注册 `C:\Windows\Fonts\msyh.ttc`
  （`LS_PDF_FONT` 可配，失败回退并告警）。一页式：标题/生成时间/五张统计卡（总数/L1/L2/L3/未处置）/
  每日柱状/合规脚注。**不含截图与人脸（合规）**
- **15C Web**：Reports.vue 工具行加"导出事件 CSV / 导出统计 CSV / 月报 PDF"三个下载按钮
  （`<a download>` 直下，PDF 月份取日期控件的当月）

### 验收结果
- `pytest tests/ -q`：**59 passed**（新增 4：CSV 表头与 BOM 与行序 / days 钳制 / stats 形状 /
  PDF 魔数+中文字体嵌入+非法月份 400）
- **PDF 目检通过**：真实生成 2026-10 月报渲染为 PNG 人工检查——中文正常、卡片/柱状图/脚注齐全
  （截图在会话记录；reportlab 5.1.0）
- `npm run build` 通过（gzip 38.1KB）

### 遗留问题
- PDF 仅一页统计版；"Top 规则触发"列暂缺（rules 与 events 无外键关联，需加 rule_id 才能统计——列入下轮）
- 大导出（>10 万行）未做压测；分页游标已就绪，必要时加 LIMIT 上限
