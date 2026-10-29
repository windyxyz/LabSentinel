# 任务 64 · SQLite WAL + busy_timeout + 并发读写验证

> 背景：`Store` 每次操作新开连接，默认 rollback journal + 无 busy_timeout；事件写入（推理线程）、
> API 查询、保留清理、告警投递可能同时写库，高并发时会出现 `database is locked`。WAL 能让读写并行，
> busy_timeout 给写锁竞争一个等待窗口。这是长期挂机稳定性的基础项。

## 目标

### 1. 连接与 PRAGMA

- `Store.__init__` 建库时执行一次：
  - `PRAGMA journal_mode=WAL`（返回 `wal` 才算成功；内存库/只读库失败则记录警告不崩）
  - `PRAGMA synchronous=NORMAL`
  - `PRAGMA busy_timeout=<ms>`（默认 5000，配置项 `LS_SQLITE_BUSY_TIMEOUT_MS`）
- `Store._conn()` 每次连接都设置 `busy_timeout`（PRAGMA 是 per-connection 的，不能只在初始化设）；
- `server/app/stats.py` 的直连也要加 `busy_timeout`（统一走一个小 helper，避免漂移）；
- 可选：`PRAGMA wal_autocheckpoint=1000`（默认值，不必改）。

### 2. 配置

- `server/app/config.py` 增加 `sqlite_busy_timeout_ms: int = 5000`；
- `.env.example` 增加注释示例；
- 说明 WAL 会产生 `labsentinel.db-wal`/`-shm`：**属正常文件，不要手删**；正常关闭会 checkpoint。

### 3. 测试 `server/tests/test_sqlite_wal.py`

- 临时库初始化后 `PRAGMA journal_mode` 返回 `wal`；`busy_timeout` 返回配置值；
- 并发验证：4 个线程各写 50 条事件 + 2 个线程持续读，全部成功、无 `database is locked`，
  最终行数 = 200；
- 优雅关闭：最后一个连接关闭后（或显式 `wal_checkpoint(TRUNCATE)`）`-wal` 文件可被 checkpoint；
- **回归**：`test_backup_restore.py`（任务 62）在 WAL 模式下仍通过；`test_retention.py` 仍通过；
- 不测真实 `server/data`。

### 4. 文档

- `docs/部署FAQ.md` 增加「数据库出现 -wal/-shm 文件正常吗 / locked 错误怎么调」；
- `docs/59-portable-install.md` 数据目录一节注明 WAL 文件与备份注意事项（备份必须用 backup API，
  不要只拷 `.db`）。

## 约束

- 不改变任何表结构/SQL 语义；不引入依赖；不调整现有事务边界（除非为 busy_timeout 必要）；
- 不改 `server/data` 真实库；测试全用临时目录；
- 若发现 WAL 与保留清理/备份存在冲突，**不要静默绕过**：写报告说明并给出建议，必要时回退该 PRAGMA。

## 验收标准

- `pytest tests/ -q` 全绿（基线 91 + 新增；含 62 的备份恢复回归）；
- 并发测试实际跑通（报告附线程数/行数/耗时）；
- 临时库目录清理干净；真实库 `journal_mode` 未被测试改动（测试只碰临时库）。

## 执行报告

**flash 2026-10-30 完成（pytest 126 passed，含 62 备份/retention 回归全绿）**

- **改动**：`store.py` 新增统一连接助手 `connect(path, readonly=False)`（**每条连接**设
  `busy_timeout` + `synchronous=NORMAL`——两者都是 per-connection PRAGMA，只在 init 设一次无效，
  实测 synchronous 曾返回 2=FULL）；`Store.__init__` 建库执行 `journal_mode=WAL`（失败只警告：
  内存库返回 memory 属预期）；`stats.py` 直连改走助手（消除漂移）；`api.py` 就绪探针只读连接补
  busy_timeout；`config.py` 加 `sqlite_busy_timeout_ms=5000`；`.env.example` 加 SQLite/日志段。
- **测试**：`test_sqlite_wal.py` 4 用例（journal_mode=wal + busy_timeout=5000 + synchronous=1 /
  **并发 4 写线程×50 事件 + 2 读线程持续读：0 locked 错误、最终 200 行、耗时 0.22s** /
  -wal 文件存在 + checkpoint(TRUNCATE) 后清空 / 内存库降级只警告）。
  全量 **pytest 126 passed**——任务 62 的 `test_backup_restore.py`（8 用例，backup API 在 WAL 下
  一致）与 `test_retention.py` 全部通过，无回归。
- **与备份/保留的冲突排查**：backup 用 `Connection.backup()` 走 SQLite 内部快照机制，WAL 源一致
  （62 回归验证）；collect_diagnostics 的 mode=ro 只读打开在同用户可写目录下可建 -shm，正常；
  保留清理为常规 SQL 写，WAL 兼容。**无需回退 PRAGMA**。
- **文档**：部署FAQ 加「-wal/-shm 正常吗 / locked 怎么调」（明确不要手删 + 备份必须走 backup API）；
  docs/59 数据位置节加 WAL 文件说明与备份警告。
- **遗留**：无。真实库 journal_mode 未被测试改动（测试全用临时库；真实库将在服务下次重启时由
  init 自然切到 WAL）。
