# 任务 41 · 数据飞轮闭环演练（误报 → 标注池 → 抽帧）

> 背景：任务 12 做了"误报截图复制到 annotation_pool"，任务 11 做了抽帧/预标注工具，但**整条链路没有一次端到端实跑**。本任务用临时数据库在 8001 验证：事件 → 标记误报 → 截图进池 → 抽帧可用，形成可复用的飞轮证据。

## 目标

1. **起隔离实例**（只用 8001 + 临时数据目录，绝不污染主库）：
   ```bat
   set LS_DB_PATH=server\data\flywheel_test.db
   set LS_SNAPSHOT_DIR=server\data\flywheel_snapshots
   set LS_VIDEO_DIR=server\data\flywheel_videos
   set LS_ANNOTATION_POOL_DIR=server\data\flywheel_pool
   cd server && D:\LabSentinel\m0-venv\Scripts\python.exe -m uvicorn app.main:app --port 8001
   ```
   确认 `LS_ADMIN_TOKEN` 未设置（写接口免令牌），`/api/system` 就绪。
2. **造一条真实事件**：API 创建文件摄像头（`m0-spike/data/lab-walk-24076.mp4`，autostart=false）→ 画安全区 → 加 `enter` 规则 → 启动 → 轮询到事件（≤60s），记录 event id / 截图路径。
3. **标记误报**：
   ```http
   POST /api/events/{id}/status
   {"status": "false_positive", "handled_by": "flywheel-e2e"}
   ```
   验证：
   - 返回 `{"ok": true, "status": "false_positive"}`；
   - `LS_ANNOTATION_POOL_DIR` 下出现 `ev{id}_{ts}_cam{cam}_{type}.jpg`，与事件截图**同尺寸/同字节**（复制不是移动）；
   - 原事件截图仍可 `GET /api/events/{id}/snapshot` 200；
   - `GET /api/events?status=false_positive` 能查到该事件。
4. **抽帧验证**：
   ```bat
   D:\LabSentinel\m0-venv\Scripts\python.exe tools\frame_sampler.py --video m0-spike\data\lab-walk-24076.mp4 --out logs\flywheel_frames --interval 1.0 --max 8
   ```
   确认输出 8 张左右 jpg（或按差异阈值少几张，记录实际数）；`tools/preannotate.py --help` 可运行（是否实跑预标注由你判断，若需加载模型则跳过，仅验证参数与帮助）。
5. **清理**：停 8001；删除 `flywheel_test.db`、三个临时目录、`logs/flywheel_frames`；确认无残留摄像头/事件（临时库删除即无）。
6. **报告**：`docs/56-flywheel-e2e.md`——命令、时间线、事件 id、池内文件名、字节一致性、抽帧数量、发现的问题。

## 约束

- 只占 8001；不碰 8000/5173；不训练；不做其它推理并发。
- 所有测试数据写临时目录并删除；主库 `server/data/labsentinel.db` 不得被写入。
- 不改业务代码；发现 bug 写"建议任务"。
- 抽帧是 CPU 轻量任务；preannotate 若需加载模型/DML，跳过实跑，避免与 e2e 推理叠加。

## 验收标准

- 事件产生且 screenshot 200；
- 误报后 annotation_pool 出现命名正确的 jpg，且与原截图字节一致；
- 事件状态筛选可查到；原截图未被删除；
- frame_sampler 产出 jpg（数量与参数相符）；
- 清理完成、主库未动、报告齐全。

## 执行报告

（待填：事件 id/时间线 / 池文件与字节校验 / 抽帧数量 / 发现 / 清理确认）
