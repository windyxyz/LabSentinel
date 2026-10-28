# 61 · 导出与报表回归报告（任务 49）

> 日期：2026-10-28 08:25。执行：flash（任务 49）。
> 目的：CSV/PDF/证据包四端点在同一隔离实例上的全量回归留证（发布前证据）。

## 1. 结论

**通过**：四个导出端点全部 200 且内容校验过关（BOM/中文/表头/魔数/ZIP 清单/路径安全/行数一致）；
空库边界行为合法（空 CSV/最小 ZIP，非 5xx）；主库零写入。

## 2. 演练环境与数据

- 隔离实例：8001 + `export_test.db` + `export_data/`（snapshots/videos/pool），`LS_FILE_LOOP=true`。
- 数据：文件摄像头循环播放 20s → **10 条事件，其中 6 条含视频存证**。

## 3. 四端点校验

| 端点 | 响应 | 校验 |
|---|---|---|
| `/api/export/events.csv` | 200 `text/csv; charset=utf-8` 882B | UTF-8 **BOM ✓**；表头含 时间/摄像头/类型/级别/详情/状态 ✓；**数据行 10 = 事件 10** ✓；中文"进入安全区"无乱码 ✓ |
| `/api/export/stats.csv` | 200 629B | BOM ✓；31 行（表头+30 天日统计，空日补 0）✓ |
| `/api/export/monthly.pdf?month=2026-10` | 200 50,127B | **%PDF 魔数 ✓**、>5KB ✓；本机无 pypdf/PyPDF2（魔数校验为准，未做页解析） |
| `/api/export/evidence.zip` | 200 42.2MB | 条目 18：events.csv ✓ / README.txt ✓ / 快照 ✓ / **videos/ 6 条 mp4** ✓；**无 `..`/绝对路径/盘符**（路径穿越检查 ✓）；内嵌 CSV 数据行 10 = 事件数 ✓ |

## 4. 空库边界（如实记录）

- `events.csv`：200，仅表头 78B（0 数据行）——合法空 CSV ✓
- `evidence.zip`：200，2 条目（README + 空 events.csv）——合法最小 ZIP ✓
- 未实测：500 条上限（任务说明允许不造大量数据）

## 5. 清理确认

export_test.db / export_data/ / logs/export-check/ 全删；8001 已停；主库 `labsentinel.db` 零写入 ✓。

## 6. 建议

- PDF 内容建议后续加一次人工目检（本次以魔数+大小为准，解析器未安装）。
