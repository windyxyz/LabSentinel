# 任务 69 · 数据库 schema 版本与迁移框架

> 背景：现在 `store.py` 的 `_migrate()` 用"缺列就 ALTER"的临时逻辑，没有版本记录；后续再加字段/表时
> 无法判断库处于哪个版本，也不容易写"必须执行一次"的数据迁移。本任务建立最小的版本化迁移框架，
> 兼容现有旧库（含便携版用户库）。

## 目标

### 1. `schema_version` 表

```sql
CREATE TABLE IF NOT EXISTS schema_version(
  version INTEGER PRIMARY KEY,
  applied_at TEXT
);
```

### 2. 迁移列表（`server/app/store.py`）

- `MIGRATIONS = [(1, "events 增加 video/status/handled_by/handled_ts", fn1), ...]`
- 每个迁移函数接收 `sqlite3.Connection`，必须**幂等**（列已存在则跳过），执行完插入/更新
  `schema_version`；
- 启动时：
  - 建 `_SCHEMA`（保持 `CREATE TABLE IF NOT EXISTS`）+ `schema_version`；
  - 读当前最大 version；依次执行大于当前版本的迁移；
  - **每个迁移在事务内**（失败回滚并抛出，带版本号与错误）；
- 旧库兼容：没有 `schema_version` 但有完整列 → 探测并把版本标为当前最新（视为已迁移）；
  有部分列 → 让迁移函数按"缺列补列"逐项补齐；
- 提供 `Store.schema_version() -> int`，并在 `/api/system` 增加 `schema_version` 字段（只增不改）。

### 3. 测试 `server/tests/test_migrations.py`

- 全新库：初始化后 `schema_version` = 最新，表结构完整；
- 模拟旧库：用最小旧 schema 建库（events 只有基础列），插入 1 条事件 → 打开 Store →
  缺列补齐、`video/status/handled_by/handled_ts` 存在、原数据保留、版本正确；
- 幂等：连续打开 Store 两次，version 不变、不重复插入版本行、不报错；
- 失败迁移：临时插入一个必失败的迁移（monkeypatch MIGRATIONS）→ 抛错且该版本未写入；
- 不碰真实 `server/data`。

### 4. 文档

- `docs/部署FAQ.md` 增加「升级后数据库会自动迁移吗 / 迁移失败怎么办（先备份）」；
- `docs/59-portable-install.md` 升级一节说明：程序覆盖后首次启动自动迁移，建议先备份；
- `docs/35-api-reference.md` `/api/system` 增加 `schema_version`。

## 约束

- 不改变现有表/列语义；迁移只做加列/加表/索引，不做破坏性删除；
- 不引入依赖；错误信息含版本号，便于用户报障；
- 若发现现有 `_migrate` 与 `_SCHEMA` 顺序有隐式依赖，先写报告说明再调整，不要静默改语义。

## 验收标准

- `pytest tests/ -q` 全绿（基线 91 + 新增；含 62 备份恢复、64 WAL 回归）；
- 用 64 产生的临时 WAL 库打开迁移不报错；
- 旧库迁移实测：报告附迁移前后 `PRAGMA table_info(events)` 与行数。

## 执行报告

（待填：改动文件 / 旧库迁移证据 / 测试 / 遗留）
