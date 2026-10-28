# 任务 51 · 演示播种与环境逻辑单源化（消除 deploy/portable 漂移）

> 背景：`scripts/deploy.py` 与 `scripts/portable_launcher.py` 各自维护一份 SEED_SCHEMA / 4 路摄像头 / 区域 / 规则。两边已经出现过一次"首启空库"问题（任务 47），后续改 schema/坐标很容易只改一边。本任务把播种抽成单一模块，双方共用。

## 目标

1. 新增 `scripts/seed_demo.py`（标准库）：
   - 常量：`SCHEMA`（cameras/zones/rules/events/face_consents/alert_deliveries + 索引，必须与 `server/app/store.py` 同步）、`SEED_CAMERAS`、`SEED_ZONES`、`seed_database(db_path, media_dir) -> int`（返回插入摄像头数）。
   - `seed_database` 只在 cameras 表为空时播种；已存在则返回 0，绝不覆盖。
   - `--selftest` 可选：临时库自测，打印 4 路/区域/规则数。
2. **deploy.py**：删除自有 SEED_SCHEMA/SEED_VIDEOS/SEED_ZONES，改为调用 `seed_demo.seed_database`；已有库的"失效路径修正"逻辑保留（按名称修正）。
3. **portable_launcher.py**：删除自有 SEED_*，改为 `sys.path.insert(0, scripts dir)` 后调用 `seed_demo.seed_database(DATA/"labsentinel.db", MEDIA)`；保留 `repair_media_paths()`。
4. **build_portable.py**：把 `scripts/seed_demo.py` 复制进便携包 `scripts/`，确保启动器能导入。
5. **测试**：新增 `server/tests/test_seed_demo.py`（或独立自测）：
   - 空库 → 4 cams/4 zones/8 rules；再次调用返回 0 且数量不变；
   - 已有 1 路用户数据 → 返回 0，不新增；
   - schema 能被 `Store` 打开（可选：用 server Store 读一遍）。
6. 文档：在 `docs/50-tasks/47...`/`部署FAQ` 里注明"播种唯一来源 scripts/seed_demo.py"；执行报告写调用点与测试。

## 约束

- 不改服务端业务代码；不改演示数据内容（4 路/坐标/规则级别保持现状）；不训练。
- 便携包/源码两种模式行为不变；用户已有数据绝不覆盖。
- 旧 `deploy.py` 的兼容路径（D:\LabSentinel\m0-venv）不动。

## 验收标准

- deploy/portable 均只通过 seed_demo 播种；仓库内 SEED_SCHEMA 只剩一份；
- 自测/单测覆盖空库、幂等、用户数据不覆盖；
- 便携包重建后首启仍是 4 路；pytest 全绿；
- 报告列出改动与验证。

## 执行报告

（待填：模块接口 / 两个调用点 / 测试结果 / 便携包重建 / 遗留）
