# 任务 62 · 数据备份 / 恢复 CLI

> 背景：便携版数据在 `%LOCALAPPDATA%\LabSentinel\data`（事件库 + 截图 + 录像 + 误报池）。用户升级、
> 换机、排障时需要安全备份；现在只能按文档手工拷贝，**运行中直接复制 SQLite 可能拿到不一致快照**。
> 本任务提供在线备份（SQLite backup API）+ 校验 + 恢复。

## 目标

新增 `scripts/backup_data.py`（只用标准库）：

### `backup`

```
python scripts/backup_data.py backup --data-dir <data> --out <zip> [--no-media]
```

- 用 `sqlite3.Connection.backup()` 把 `labsentinel.db` 热备份到临时文件（**不要**直接复制正在写的 db）；
- 打包：`labsentinel.db` + （默认）`snapshots/`、`videos/`、`annotation_pool/` 整个目录；
  `--no-media` 只备份 db；
- 写 `manifest.json`：`created_at`、`source_data_dir`、`db_bytes`、`db_sha256`、`media_files`、
  `media_bytes`、`integrity_check`（`ok`/错误文本）、`tool_version`；
- 输出 zip 后打印文件数/大小；`--out` 已存在默认拒绝（`--force` 覆盖）。

### `restore`

```
python scripts/backup_data.py restore --zip <zip> --data-dir <dir> [--force] [--no-media]
```

- 先 `verify`：manifest 存在且可解析、db 通过 `PRAGMA integrity_check`、所有条目路径必须位于目标内
  （拒绝 `..`/绝对路径/盘符——路径穿越防护）；
- 目标目录已有 db 且未 `--force` → 拒绝并提示；
- `--force` 时先把现有 db 复制为 `labsentinel.db.bak-<时间戳>`（保留最近 3 个，多了删最旧）；
- 恢复 db 与媒体（`--no-media` 时跳过媒体）；不删除目标中备份里没有的文件（避免误删用户新数据）；
- 恢复后对 db 再跑一次 integrity_check，失败 exit 2。

### `verify`

```
python scripts/backup_data.py verify --zip <zip>
```

只校验 manifest、db 完整性、路径安全，不落盘到 data 目录。

### 测试 `server/tests/test_backup_restore.py`

- 临时 data 目录：建小 sqlite（含 cameras/events 几行）+ 假截图/视频文件；
- backup → 解压检查 manifest 字段/hash/integrity → 删原数据 → restore → 行数与文件一致；
- 无 `--force` 恢复非空目录 → 拒绝；`--force` → 生成 `.bak-*`；
- 损坏 zip / manifest 缺字段 / db 损坏 → 明确报错非 0；
- 构造含 `../evil` 或绝对路径条目的 zip → 拒绝；
- `--no-media` 只恢复 db；
- 测试只用临时目录，**不碰真实 `server/data`**。

### 文档

- `docs/部署FAQ.md` 增加「如何备份/迁移数据 / 恢复前注意什么」；
- `docs/59-portable-install.md` 数据目录一节加备份/恢复命令（指向 `%LOCALAPPDATA%\LabSentinel\data`）。

## 约束

- 不备份模型/程序文件；不引第三方依赖；Windows 路径与中文文件名正确处理；
- 恢复默认保守（不覆盖、不删多余文件）；所有失败给出可读原因与退出码；
- 不修改服务端运行逻辑；备份期间服务可继续运行（SQLite backup API 保证一致快照）。

## 验收标准

- `pytest tests/ -q` 全绿（基线 91 + 新增）；
- 对本机真实 `server/data` 做一次 backup 到临时目录并 `verify`（**只读**，不 restore），
  db 快照可打开、事件行数与源库一致；备份后删除临时 zip；
- `--help` 三个子命令均可用；报告附实际输出与文件大小。

## 执行报告

**flash 2026-10-30 完成（pytest 117 passed，真实库只读备份验证通过）**

- **交付**：`scripts/backup_data.py`（backup/verify/restore 三子命令，纯标准库）：backup 用
  `sqlite3.Connection.backup()` 热快照（服务运行中可直接备）+ 媒体三目录 + manifest（哈希/计数/
  integrity/tool_version）；verify 只读校验（manifest 字段、db sha256、PRAGMA integrity_check、
  条目路径安全）；restore 先 verify、无 --force 拒绝覆盖、--force 转存 .bak-<ts>（保留 3 个）、
  保守恢复不删多余文件、恢复后复检 integrity。
- **测试**：`test_backup_restore.py` 8 用例（往返含中文媒体文件名 / --no-media 双向 / 无 force 拒绝
  +force 生成 .bak / .bak 保留 3 个 / 路径穿越三种 `../`、盘符、绝对路径全拒 / 篡改 db sha256 不匹配 /
  缺 manifest / 非 zip）。全量 **pytest 117 passed**（109 + 8）。
  （过程中修一处自坑：backup 的 --no-media 初版未生效。）
- **真实数据实跑（只读，未 restore）**：`server/data` → 临时 zip：**1673 文件 / 341.7MB**
  （db 0.4MB + 媒体 1671 个/342.1MB，integrity=ok）；verify OK（sha256 前缀 c6bfe2c93edf…）；
  **事件行数 源库=1201 = 快照=1201**；临时 zip 已删除。
- **文档**：部署FAQ 加「如何备份/迁移数据/恢复前注意」（含保守语义与 .bak 说明）；docs/59 数据
  位置节加便携版三命令（runtime\python.exe）。
- **遗留**：无。
