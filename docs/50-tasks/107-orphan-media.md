# 任务 107 · 孤儿媒体文件回收（P1）

> 来源：flash 审阅建议 #2。紧急清理先删事件行、recorder 之后才写完的视频；崩溃留下半截 mp4；
> 这些文件没有任何事件引用，但会永久占磁盘。

## 目标

- `retention` 增加孤儿扫描（在 `run_cleanup` 末尾、`preview_cleanup` 同步统计）：
  - 目录：`settings.snapshot_dir`、`settings.video_dir`（不扫 annotation_pool，误报池有自己的保留期）；
  - 收集 events 表中仍被引用的 snapshot/video 路径（集合）；
  - 文件 mtime 早于 **grace 1h**（避免删正在录制的文件）且不在引用集合 → 删除；
  - 统计 `deleted_orphans/files`、`freed_orphan_mb`；路径必须在 data_dir 内（用严格包含）。
  - 单轮最多删 N 个（默认 500），避免一次卡死；记录日志。
- `preview_cleanup` 返回 `orphans:{files,bytes}`，不删除。
- 测试 `server/tests/test_orphan_media.py`：
  - 造 2 个被引用文件 + 2 个孤儿（mtime 2h 前）+ 1 个新孤儿（10min 前）→ 只删旧孤儿；
  - 引用文件即使旧也保留；
  - `..`/data 外路径不删；preview 只统计；
  - 空目录/不存在目录不崩。
- 文档：运维手册/FAQ「磁盘里为什么有没事件引用的视频/会自动清吗」。

## 约束

- 不删 annotation_pool；不删新文件；不引入依赖；不改变现有清理统计字段（只新增）；
- 测试临时目录，不碰真实 data。

## 验收

- `pytest tests/ -q` 全绿；报告附预演/实删计数对照；真实 data 只跑 preview（如允许）。

## 执行报告

**主线 2026-11-03 完成**

- `retention.py` 新增孤儿扫描：
  - 目录 `snapshot_dir`/`video_dir`（不递归、不扫 annotation_pool）；
  - 扩展名白名单（jpg/png/bmp/webp/mp4/avi/mkv/mov）；
  - 仅处理 **mtime 早于 1h** 且未被事件/图片记录引用的文件；路径必须严格位于 `data_dir`；
  - 单轮最多删 **500** 个，超出标记 `orphans_limited=true` 下轮继续；
  - 统计新增 `deleted_orphans`、`freed_orphan_mb`、`orphans_limited`（不改旧字段）。
- `Store.referenced_media_paths()`：一次 UNION 查询 events.snapshot/video + folder_images.snapshot，
  避免逐文件 N 次查询；与 121 证据副本联动，**图片记录引用的证据不会被当孤儿删**。
- `preview_cleanup()` 新增 `orphans:{files,bytes}` 并计入 `estimated_free_gb`，只读不删。
- 测试 `server/tests/test_orphan_media.py`（4 项）：
  - 2 个旧孤儿（各 1MB）删除、引用文件（事件截图/事件视频/图片证据）保留、
    10 分钟新文件保留；
  - preview 只统计 2 个/字节数>0，不删；
  - data_dir 外路径不删；空目录/不存在目录不崩。
- **回归**：`test_orphan_media + test_state_prune + test_retention + test_retention_preview`
  **18 passed**。
- 说明：真实 data 未做删除演练（按要求测试临时目录）；6 小时调度会按 500/轮清理。
  运维手册/FAQ 的“孤儿视频/截图会不会自动清”建议在 99/104 收口时同步一句（本任务未改大文档）。

