# 任务 92 · 定时自动备份（P1）

> 背景：任务 62 提供了手动备份，但普通用户不会每天敲命令。需要服务端按周期自动备份数据库
> （默认只备 db，不备大媒体），保留最近 N 份，并在界面/端点可见。

## 目标

### 1. 后台备份器 `server/app/backups.py`

- 线程模式与 retention 一致（`start_scheduler/stop_scheduler`，main lifespan 调用）；
- 配置：
  - `LS_BACKUP_INTERVAL_H=24`（0=关闭）
  - `LS_BACKUP_KEEP=7`
  - `LS_BACKUP_INCLUDE_MEDIA=false`（true 时调用 backup_data 带媒体，可能很大，文档警告）
- 备份目录 `settings.data_dir/backups/`，文件名 `auto-YYYYmmdd-HHMMSS.zip`；
- 复用 `scripts/backup_data.py`（subprocess `sys.executable script backup --data-dir ... --out ...`）：
  - 脚本路径解析兼容源码/便携（`server` 同级 `scripts/`，便携 `ROOT/scripts/`）；
  - 失败只记录日志，不退出服务；
- 启动时若“最新 auto 备份 age > interval”则后台立即补一次；检查周期 min(1h, interval/4)；
- 保留：按 mtime 删除超出 keep 的 `auto-*.zip`；不删手动备份；
- 状态持久化：无需新表，按目录文件判断；`last_backup` 从最新文件 mtime。

### 2. API

- `GET /api/backups/status`（读保护口径）：`{enabled, interval_h, keep, include_media,
  last:{name,size,mtime,age_h}|null, next_due_h, count, total_bytes}`；
- `POST /api/backups/run`（require_admin）：后台触发一次，返回 `{started:true}`；若正在跑返回 409；
- 备份文件下载？**不做**（避免暴露整库下载；用户可在数据目录取）。

### 3. 前端

- Dashboard 磁盘卡旁加一行“自动备份：最近 X 小时前 / 未开启”；点击可触发一次（admin，confirm）；
- 只改 Dashboard.vue/api.js。

### 4. 测试 `server/tests/test_scheduled_backup.py`

- 纯函数：`_needs_backup(files, now, interval_h)`、`_prune(files, keep)`、状态汇总；
- 用假 runner（subprocess monkeypatch）验证调用参数、失败不抛、并发保护（409）；
- 临时 data 目录造 `auto-*.zip` 验证只保留 N 份且不动手动 zip；
- API 字段/鉴权；
- 不真等 24h、不真备份大媒体。

### 5. 文档

- `.env.example` 3 项；
- `docs/59-portable-install.md` 数据目录/备份一节加自动备份说明与恢复命令；
- `docs/运维手册.md` 备份恢复一节；
- `docs/35-api-reference.md` 端点。

## 约束

- 默认只备 db（小、快），媒体备份需显式开；备份线程异常不能影响主服务；
- 不删除非 auto 文件；不引入依赖；
- 测试不碰真实 `server/data`。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 临时 data 实跑：`POST /api/backups/run` 生成 auto zip（db 可打开、事件数一致）、keep=2 时只留 2 份；
- 报告附文件列表/大小。

## 执行报告

（待填：改动文件 / 实跑证据 / 测试与 build / 遗留）
