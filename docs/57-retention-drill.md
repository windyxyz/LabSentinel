# 57 · 数据保留与清理演练报告（任务 48）

> 日期：2026-10-28 08:10。执行：flash（任务 48）。
> 目的：retention（任务 14）的端到端证据——造旧数据 → 触发清理 → 行/文件/池全部删除，近期数据保留。

## 1. 结论

**通过**：隔离实例（8001 + retention_test.db + retention_data/）上，30 天保留策略的清理
精准删除过期事件（行 + 截图文件 + 误报池副本），近期事件完好；`/api/maintenance/cleanup`
与 `/api/maintenance/status` 字段正确；主库零写入。磁盘紧急清理项按任务说明未实测（见 §5）。

## 2. 演练时间线与证据

| 步骤 | 内容 | 结果 |
|---|---|---|
| 隔离实例 | `LS_RETENTION_EVENTS_DAYS=30 / LS_RETENTION_POOL_DAYS=30 / LS_DISK_MIN_FREE_GB=1` | server up ✓ |
| 造事件 | 文件摄像头 → enter(L2) → 6s 出事件 | 旧事件 id=3，截图 `ev_1793146008338.jpg` 235,113 bytes ✓ |
| 误报池 | 标记 false_positive | `annotation_pool/ev3_20261028_080648_cam1_enter.jpg` ✓ |
| 造旧 40 天 | sqlite `update events set ts='2026-09-18T08:06:49'`；`os.utime` 池副本 mtime → 09-18 | ✓ |
| 近期事件 | 相机继续跑（FILE_LOOP） | 新事件 id=1/2/4（ts=10-28）✓ |
| **触发清理** | `POST /api/maintenance/cleanup` | **`{"deleted_events": 1, "deleted_files": 3, "freed_mb": 5.43, "disk_free_gb": 107.94}`** |
| 行校验 | 旧 id=3 已删；近期 [1,2,4] 保留 | ✓ |
| 文件校验 | 旧截图 / 池副本均已删除 | ✓ |
| status | `last_cleanup`（含 deleted_events/deleted_files/freed_mb/ts）+ `disk_free_gb` | ✓ |

## 3. 未实测项（如实记录）

- **磁盘紧急清理**（`_disk_free_gb < LS_DISK_MIN_FREE_GB` 触发的从最旧删起）：无法在不影响本机
  磁盘的前提下安全模拟小剩余空间，按任务说明**未实测**；该分支代码路径与手动 cleanup 共享
  （retention.run_cleanup），逻辑风险低。
- 视频存证文件：本次演练事件未产生视频（file 源录制需触发时长条件），"视频文件不存在"校验
  以截图+池副本为代表。

## 4. 清理确认

retention_test.db / retention_data/（snapshots/videos/pool）全删；8001 已停；
主库 `labsentinel.db` 零写入（全程使用独立库与目录）✓。
