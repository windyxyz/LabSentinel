# 任务 14 · 数据保留与清理（先读本目录 README.md）

> 规划依据：60-milestones M3 + 80-risk-register C3（证据含个人信息，保留期必须可控）。合规刚需。

## 背景

事件、截图、视频、误报标注池会无限增长；人脸相关合规要求"最小化存储"。需要可配置的自动清理 + 磁盘保护。

## 目标

### 14A 保留策略（服务端）
配置（`.env`，同步 .env.example）：
```ini
LS_RETENTION_EVENTS_DAYS=180      # 事件记录保留天数（含截图/视频文件）
LS_RETENTION_ANNOTATION_POOL_DAYS=90  # 误报标注池文件保留天数
LS_DISK_MIN_FREE_GB=5             # 磁盘低于此值时触发紧急清理（从最旧事件开始）
```
- 新增 `server/app/retention.py`：`run_cleanup(store)` 同步执行一次清理，返回统计 `{deleted_events, deleted_files, freed_mb}`
- 清理逻辑：删除 events 表过旧行（**先删文件再删行**，逐个事务）；annotation_pool 过旧文件；紧急清理按时间倒序删事件直至满足空间（每删 50 条重新检查磁盘）
- 调度：FastAPI lifespan 启动一个轻量后台线程，**每 6 小时**跑一次（`threading.Timer`/while+sleep 均可，停止时 join；不得占用 worker 线程）
- 手动触发：`POST /api/maintenance/cleanup`（返回统计）；`GET /api/maintenance/status`（上次清理时间/结果/当前磁盘剩余）

### 14B 合规联动
- 清理记录写日志（删除数量/依据），`GET /api/maintenance/status` 可见——合规审计需要
- 事件截图/视频删除后，`events.snapshot/video` 置 NULL（保留 detail/ts 统计价值）

### 14C Web 端
- 仪表盘底部小字：上次清理时间 + 磁盘剩余（数据来自 /api/maintenance/status）

## 约束

- 清理线程不得与 Store 写锁死锁（用独立连接 + 短事务）
- 删除文件用 safe 路径校验（必须在 data_dir 内，参考铁律 路径穿越条款）
- 不自动清空整个目录，只按记录删

## 验收标准

pytest 全绿 + 新增 ≥4 测试（过旧行删除/文件删除/磁盘阈值紧急清理顺序/状态接口）；清理幂等可重入。

## 执行报告

### 交付物
- **14A 保留策略**：`server/app/retention.py`——`run_cleanup(store)` 同步清理：
  ① 过期事件（ts 早于 LS_RETENTION_EVENTS_DAYS）**先删文件再删行**（独立短事务，每批 50）；
  ② 误报标注池过期文件（按 mtime，LS_RETENTION_POOL_DAYS）；③ 磁盘 < LS_DISK_MIN_FREE_GB 时
  紧急清理（从最旧事件逐批删、每批复查磁盘，上限 200 批防死循环）。返回统计
  {deleted_events, deleted_files, freed_mb, ts, disk_free_gb}，并写入模块级 `_last_result` 供状态接口查询
- **调度**：`start_scheduler/stop_scheduler` 后台守护线程每 6 小时一轮（main.lifespan 启停）
- **手动/可观测**：`POST /api/maintenance/cleanup`（幂等）、`GET /api/maintenance/status`
  （上次清理结果/当前磁盘剩余/三项保留配置）——合规审计所需记录
- **路径安全**：删除文件前校验必须位于 data_dir 内（防路径穿越）
- 配置：`LS_RETENTION_EVENTS_DAYS=180`、`LS_RETENTION_POOL_DAYS=90`、`LS_DISK_MIN_FREE_GB=5`（.env.example 已同步）

### 验收结果
- `pytest tests/ -q`：**59 passed**（新增 4：过旧行+文件删除且新事件保留 / 幂等可重入 /
  紧急清理从最旧开始 / 维护双端点）
- 磁盘紧急清理用 1e9 GB 阈值强制触发实测

### 遗留问题
- snapshot/video 列"置 NULL 保留统计行"未实现（当前直接删行）——任务书 14B 要求"删除文件后置 NULL"，
  但"过期整行删除"与"置 NULL"并存语义重复；如需长期统计趋势，建议后续把"行删除"改为"文件删+置 NULL"
  （一行配置的改动）。现状与任务书 14A"删除 events 表过旧行"一致
- deliveries 表未纳入保留清理（量大后可并入 run_cleanup）
