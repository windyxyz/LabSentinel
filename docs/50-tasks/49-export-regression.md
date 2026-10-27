# 任务 49 · 导出与报表回归（CSV/PDF/证据包）

> 背景：任务 15/21/33 分别做了 CSV/PDF/证据包导出，但缺少一次"同一隔离实例上四个导出端点全跑 + 内容校验"的回归。发布前值得留一份证据。

## 目标

1. **隔离实例**（8001 + 临时目录，主库零写入）：
   ```bat
   set LS_DB_PATH=server\data\export_test.db
   set LS_SNAPSHOT_DIR=server\data\export_snapshots
   set LS_VIDEO_DIR=server\data\export_videos
   set LS_ANNOTATION_POOL_DIR=server\data\export_pool
   cd server && D:\LabSentinel\m0-venv\Scripts\python.exe -m uvicorn app.main:app --port 8001
   ```
2. **造 2–3 条事件**：文件摄像头 + 安全区 + 规则 → 启动 → 等事件；尽量让其中一条产生**视频存证**
   （等 4–5s 让 recorder 写完；若视频未生成，如实记录）。
3. **四个导出端点**（python httpx 或浏览器下载）：
   - `GET /api/export/events.csv`：UTF-8 BOM（`\xef\xbb\xbf`）、表头包含时间/摄像头/类型/级别/详情/状态、中文不乱码；
   - `GET /api/export/stats.csv`：按日统计行数正确；
   - `GET /api/export/monthly.pdf?month=YYYY-MM`：`%PDF` 魔数、文件 >5KB、可被 pdf 解析器打开（有 reportlab 环境）；
   - `GET /api/export/evidence.zip`：zip 可打开；含 `events.csv`、`README.txt`、快照文件；若有视频则含 `videos/`；
     检查条目名不含 `..`/绝对路径；解压后 CSV 行数与事件数一致。
4. **边界**：空库/无事件时导出应返回合法空 CSV/ZIP（或明确 4xx），记录实际行为；500 条上限不实测（不可造大量数据）。
5. **清理**：停 8001；删除 export_test.db 与三个临时目录及下载产物；确认主库零写入。
6. **报告**：`docs/61-export-regression.md`（命令、响应码/大小/魔数、ZIP 清单、CSV 行数、边界行为、未实测项）。

## 约束

- 只占 8001；不碰 8000/5173；不改业务代码；不训练；不做其它推理并发。
- 导出文件写到 `logs/export-check/`（gitignored），结束后删除；不要污染 `server/data`。
- PDF/CSV 校验用 Python（zipfile/csv/字节魔数），不要依赖外部 GUI。

## 验收标准

- 四个端点均 200，Content-Type 合理；CSV BOM/中文/行数正确；PDF 魔数正确；ZIP 清单与事件一致且无路径穿越；
- 边界行为如实记录；
- 主库零写入、临时数据清理完成；
- 报告含证据与未实测项。

## 执行报告

（待填：隔离实例 / 事件数 / 四端点响应 / ZIP 清单 / CSV/PDF 校验 / 边界 / 清理）
