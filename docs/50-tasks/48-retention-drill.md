# 任务 48 · 数据保留与清理演练（过期事件/误报池/清理接口）

> 背景：任务 14 实现了 retention（6h 后台清理 + 手动触发 + 磁盘紧急清理），但没有一次"造旧数据→触发清理→验证文件与行都删"的端到端演练。本任务在隔离实例上补齐证据。

## 目标

1. **隔离实例**（8001 + 临时数据目录，主库零写入）：
   ```bat
   set LS_DB_PATH=server\data\retention_test.db
   set LS_SNAPSHOT_DIR=server\data\retention_snapshots
   set LS_VIDEO_DIR=server\data\retention_videos
   set LS_ANNOTATION_POOL_DIR=server\data\retention_pool
   set LS_RETENTION_EVENTS_DAYS=30
   set LS_RETENTION_POOL_DAYS=30
   set LS_DISK_MIN_FREE_GB=1
   cd server && D:\LabSentinel\m0-venv\Scripts\python.exe -m uvicorn app.main:app --port 8001
   ```
2. **造数据**：创建文件摄像头 + 安全区 + enter 规则 → 启动 → 等到事件（≤60s）→ 确认截图文件存在；再
   `POST /api/events/{id}/status` 标记 `false_positive`，确认 annotation_pool 有副本。
3. **造旧**（只改临时库/临时文件）：
   - 用 sqlite3 把该事件 `ts` 改成 40 天前（`YYYY-MM-DDTHH:MM:SS`）；
   - 把 pool 副本的 mtime 改成 40 天前（`os.utime`）。
4. **触发清理**：`POST /api/maintenance/cleanup`，验证：
   - 响应 `deleted_events>=1`、`deleted_files>=1`；
   - 旧事件行消失；其截图/视频文件不存在；pool 副本被删；
   - **新建一条近期事件不受影响**（保留）；
   - `GET /api/maintenance/status` 返回 last_cleanup/disk_free_gb/retention 配置。
5. **软失败项**：磁盘紧急清理（`_disk_free_gb < 下限`）无法在不动磁盘的情况下安全模拟，允许**不实测**，在报告写明原因；不要伪造数字。
6. **清理**：停 8001；删除 retention_test.db 与三个临时目录；确认主库未写入。
7. **报告**：`docs/57-retention-drill.md`（命令、事件 id、sqlite 改动、cleanup 响应、文件校验、未实测项）。

## 约束

- 只占 8001；不碰 8000/5173；不改业务代码/阈值；不训练；不做其它推理并发。
- 只改临时库/临时文件；不碰 `server/data/labsentinel.db` 与主快照目录。
- 结束后清理临时数据；报告如实记录软失败项。

## 验收标准

- 过期事件行+截图+pool 副本全部删除；近期事件保留；
- cleanup 响应与 status 字段正确；
- 主库/主数据目录零写入；
- 报告含完整证据与未实测项说明。

## 执行报告

（待填：临时实例 / 事件与旧化 / cleanup 结果 / 文件校验 / 未实测项 / 清理确认）
