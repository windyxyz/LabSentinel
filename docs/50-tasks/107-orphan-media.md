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

（待填：改动 / 预演实删 / 测试 / 遗留）
