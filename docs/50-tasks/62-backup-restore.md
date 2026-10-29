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

（待填：改动文件 / 实跑输出 / 测试 / 遗留）
