# 任务 105 · 审阅修复的回归补丁（P0，主线复核发现）

> 背景：flash 2026-10-30 审阅报告修了 24 个真实问题（见 `docs/审阅报告-2026-10-30.md`），但主线复核
> 发现 **4 个新回归**，其中 2 个会造成事件丢失/数据损坏，必须先修再提交。当前源码改动**未提交**，
> 本任务在现有工作区上继续修，不要回滚审阅成果。

## P0-1 空帧离场只发事件、不清状态（事件丢失）

- 位置：`server/app/pipeline.py` `_analyze()` 的 `if not dets:` 分支。
- 现状：
  ```python
  for zid, st in self._zone_states.items():
      for tid in st.inside_ids:
          self._fire_event(zid, "exit", tid, ...)
  return
  ```
  `inside_ids` 不变 → 每个空帧重复发 exit（靠冷却压制），且人再进入时 `was=True`，
  **不会再触发 enter**。
- 修复：对每个 `(zid, tid)` 调用 `st.update(tid, False)`，仅在返回 `exited=True` 时发一次 exit；
  同时处理“画面里还有别人、但某个 track 消失”的情况：用当前 `sdets.tracker_id` 集合，
  对每个 zone 中 `inside_ids - active_tids` 的 track 做同样离场更新（否则 track 丢失后状态永真）。
- 测试 `server/tests/test_zone_exit_state.py`（或扩展 `test_core`）：
  - `ZoneState.update` 语义：进入→空帧离场→再进入，第二次必须返回 entered；
  - 用假 engine/frame 或抽出的纯函数验证 `_analyze` 空帧分支只发一次 exit；
  - 部分 track 消失（A 仍在、B 消失）→ B 只 exit 一次、A 不受影响；
  - A 区→B 区直移 → A exit + B enter 各一次（保留审阅的语义修复）。

## P0-2 读保护后 `<img>/<video>/<a>` 全部 401（前端功能回归）

- 位置：`api.py` 给 `cameras/{id}/snapshot`、`events/{id}/snapshot|video`、`export/*` 加了
  `require_read`；但 `Cameras.vue/Events.vue/Zones.vue/Reports.vue` 用裸 URL（`<img src>`/`<a href>`/
  `<video src>`），浏览器不会带 `X-Admin-Token` → 开启 `LS_AUTH_READ_PROTECTED=true` 时截图/视频/导出
  全坏。
- 修复（二选一，推荐 A）：
  - A. `require_read` 接受 `?token=` 查询参数作为回退（与 header 同用 `hmac.compare_digest`）；
    `api.js` 增加 `mediaUrl(path)`，在有令牌时追加 `token`；四处媒体/下载 URL 改用它。
    文档明确：URL token 仅用于 `<img>/<video>/下载`，会出现在浏览器历史/代理日志，建议配置独立
    `LS_READ_TOKEN` 并配合 `Referrer-Policy: no-referrer`（任务 89）。
  - B. 媒体/导出端点改回公开，仅 JSON 读保护（不推荐：证据包含全部截图视频）。
- 测试：开启读保护时 header/query 两种方式均 200；错误/缺失 token 401；前端 build 通过；
  8001 实测快照/视频/CSV 下载带 token 可用。

## P0-3 restore 在“拒绝覆盖”前就删 WAL（可能丢数据）

- 位置：`scripts/backup_data.py` `cmd_restore()`。
- 现状：先删 `-wal/-shm`，再判断 `db.exists()` 且无 `--force` → 直接返回失败。**拒绝恢复却已经删了
  旧库的 WAL**；若服务在跑，删除/写入也可能在占用检查之前。
- 修复顺序：
  1. 先判断目标 db 存在且无 `--force` → 直接失败，**不碰任何文件**；
  2. 需要覆盖时，先确认服务未占用（尝试独占打开/`PRAGMA wal_checkpoint(TRUNCATE)`，失败给可读错误）；
  3. checkpoint 成功后把旧 db 转存 `.bak`，再删除已空的 `-wal/-shm`；
  4. 写入新 db、复检 integrity；失败时保留 `.bak` 并提示恢复方法；
  5. 目标本无 db 时，清理孤儿 `-wal/-shm` 可保留，但要在写入前做。
- 测试：
  - 有 WAL、无 `--force` → 返回失败且 WAL 文件仍存在、内容未变；
  - `--force` → .bak 生成、WAL/SHM 清理、新库 integrity ok；
  - 模拟 db 被占用 → 可读错误，WAL 不被删、旧库可打开；
  - 现有 8 个 backup 测试不回归。

## P1-1 规则可指向“别的摄像头”的区域

- `add_rule` 校验了 zone 存在，但没校验 `zone.camera_id == body.camera_id`；跨摄像头规则会静默失活。
- 修复：不匹配返回 400；补测试（camera 2 + camera 1 zone）。

## P1-2 测试隔离与耗时回归

- 现状：pytest 165 passed 但耗时 **10s → 78–94s**；多个 `with TestClient(app)` 的用例走完整 lifespan，
  可能仍指向真实 `server/data`（起演示摄像头线程）。`test_health_errors` 的 `tempfile.mkdtemp()`
  也从不清理。
- 修复：在 `server/tests/conftest.py` 加 autouse fixture，把 `settings.db_path/data_dir/snapshot_dir/
  video_dir/annotation_pool_dir` 默认指向 `tmp_path`（已有 monkeypatch 的用例不受影响）；
  清理临时目录；确认没有任何测试写真实 `server/data`。
- 目标：全套 <30s；报告给出修复前后耗时。若无法达标，说明根因（不要强改业务）。

## P1-3 release_check 误报（阻塞发布门）

- `release_check.py` 扫描到 `server/tests/test_release_check.py` 的假 `sk-` 串 → FAIL 1，退出码 2。
- 修复：allowlist 该测试文件（注释“测试夹具，非真实密钥”）或改用不匹配的假值；保留“真密钥→FAIL”测试。
- 验收：仓库 `release_check.py` FAIL 0、退出码 0。

## 约束

- 在现有未提交改动上增量修；**不要回滚审阅已修的其他 20 项**；
- 不改 `web/src/theme.css/style.css/App.vue/main.js`；
- 每项补回归测试；不 commit；不重建 zip（75/104 负责）。

## 验收

- `pytest tests/ -q` 全绿；新增测试覆盖上述 4 个回归；全套耗时记录；
- `npm run build` 通过；
- 8001 读保护实测媒体/导出 header+query 双路径；
- `release_check.py` FAIL 0；
- 报告逐项写“修复前现象/修复后证据”。

## 执行报告

（待填：4 回归修复 / 测试 / 耗时 / release_check / 遗留）
