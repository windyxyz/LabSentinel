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

**flash 2026-10-30 完成（pytest 145 passed，旧库迁移实测对照齐全）**

- **改动**：`store.py` 重构迁移为版本化框架——`schema_version(version, applied_at)` 表（并入
  `_SCHEMA` 之后单独建）；`MIGRATIONS = [(1, "events 增加 video/status/handled_by/handled_ts",
  _mig_001)]`（原 `_migrate` 函数体转为 v1 迁移，幂等缺列补列）；`Store._migrate` 迁移器：无版本记录
  且结构已最新的旧库 → 直接标记最新（老用户零成本升级），部分缺列 → 从 v1 逐个跑；**每个迁移显式
  BEGIN/COMMIT**；错误带版本号+描述+备份提示。兼容别名 `_migrate = Store._migrate`（test_triage/
  test_video 直接 import 模块级函数，保持不破）。`Store.schema_version()` + `/api/system` 追加
  `schema_version` 字段（只增不改）。
- **关键坑（实测发现并修复）**：python sqlite3 默认只对 DML 开隐式事务，**DDL 立即自动提交**——
  初版用 `with connection:` 导致失败迁移的 CREATE TABLE 残留（测试抓到）；改显式 BEGIN 拿到
  SQLite 原生事务性 DDL 后回滚完整。
- **测试**：`test_migrations.py` 6 用例（新库=最新+版本行数=迁移数 / 最小旧库补列+旧数据保留+默认值
  / 结构已齐旧库标记不重跑 / 重复打开幂等 / 失败迁移抛版本号错误+**DDL 回滚**/system 路由字段）。
  全量 **pytest 145 passed**（139 + 6，含 62 备份/64 WAL 回归——WAL 库打开迁移正常）。
- **旧库迁移实测对照**（报告证据）：
  - 前：`[id, ts, camera_id, type, level, track_id, detail, snapshot]`，1 行；
  - 后：`[...原 8 列..., video, status, handled_by, handled_ts]`，仍 1 行
    （旧行内容 `('旧库演示行', 'new')` 完整保留，新列默认值生效）；`schema_version=1`。
- **文档**：部署FAQ「升级后数据库会自动迁移吗/迁移失败怎么办（先备份）」；docs/59 升级节自动迁移
  说明+先备份建议；35-api-reference /api/system 的 schema_version 字段说明。
- **遗留**：无。后续加表/列：在 MIGRATIONS 追加 `(N+1, desc, fn)` 即可，框架自动接力。
