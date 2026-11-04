# 任务 114 · 摄像头 worker 守护与自动重启（P1）

> 背景：任务 65 给 worker 加了有限重连，但 worker 线程一旦退出（重试耗尽/未捕获异常），
> 只有用户手动点“启动”才会恢复；24h 挂机场景不可接受。本任务加 manager 级守护：
> 对“应该是运行中”的摄像头自动重启，带退避与上限，并暴露重启计数。

## 目标

### 1. `CameraManager` 守护线程

- `start_camera` 把 cid 加入 `_desired_running`；`stop_camera` 移除（用户主动停止不自动拉起）。
- 守护线程每 `LS_SUPERVISOR_INTERVAL_S=10` 检查：
  - cid 在 desired 且（worker 不存在或 `not is_alive()`）→ 重启；
  - 退避：连续失败 1/5/30/60s（按次数），最多 `LS_SUPERVISOR_MAX_RESTARTS=10`（0=不限？定义：0=禁用守护）；
  - 成功启动（worker alive 超过 5s）重置退避计数；
  - 达到上限：`store.set_camera_status(cid,"error")`、记录 `last_error="自动重启次数耗尽"`，停止该 cid 的守护；
- 状态字段：`restart_count`、`last_restart_ts`、`supervisor_state`（running/backoff/given_up/stopped）；
- 进程退出（stop_all）停止守护线程；不在 API 线程里做 sleep/重启。
- 配置：`LS_SUPERVISOR_ENABLED=1`、interval/max_restarts；`.env.example` 注释。

### 2. 可观测性/API

- `manager.status()` 每路追加 restart_count/last_restart_ts/supervisor_state；
- `GET /api/cameras/{cid}/status` 同步；
- `POST /api/cameras/{cid}/start` 清除 given_up 状态并重新纳入守护；
- 不新增路由。

### 3. 前端 `Cameras.vue`

- 状态列：`backoff` 显示“重试中（第 N 次）”，`given_up` 显示“自动重启失败，请检查”；
- tooltip 显示 last_restart_ts/restart_count；
- 不新增路由/不改全局四件套。

### 4. 测试 `server/tests/test_camera_supervisor.py`

- 假 worker（is_alive False）→ 守护调用 start 一次；连续失败达到上限后不再调用；
- 用户 stop 后不再自动重启；手动 start 清除 given_up；
- 退避时间可用注入 clock 单测（不真 sleep）；
- 守护线程 stop 后不残留；status 字段正确；
- 不影响现有 manager 测试。

### 5. 文档

- `部署FAQ`「摄像头自己停了会自动恢复吗 / 重试次数」；`运维手册` 摄像头排障；
- `35-api-reference` 单路状态字段。

## 约束

- 不改变 worker 内部重连语义（65）；不无限重启（默认上限 10）；
- 不引入依赖；不在每帧/API 请求里做守护判断；
- 测试不启动真实摄像头/不真 sleep。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 临时实例用坏源（不存在文件）验证：worker 退出→守护按退避重启→达上限 given_up；
- 报告附 restart_count/状态变化。

## 执行报告

**flash 2026-11-04 完成（pytest 386 passed = 373+13 新增；npm build gzip 54.45KB；8188 实机验证全链）**

### 1. CameraManager 守护（server/app/pipeline.py）

- 状态：`_desired_running: set[cid]`（start_camera 加入 / stop_camera 移出=用户主动停止不拉起）、
  `_sup: {cid: {restarts, last_restart_ts, state, next_attempt_ts, start_ts}}`；
  `supervisor_state ∈ running/backoff/given_up/stopped`。
- `supervise_tick()`（锁内决策、锁外执行 `start_camera`——避免 _mgmt_lock 重入死锁）：
  worker 存活 >5s（`SUPERVISOR_SUCCESS_S`）→ restarts 清零；desired 内 worker 死亡 →
  退避到期才重启（`SUPERVISOR_BACKOFF_S=(1,5,30,60)`，第 k 次失败等 BACKOFF[min(k-1,3)]）。
- `_sup_restart`：达 `LS_SUPERVISOR_MAX_RESTARTS`（默认 10；0=禁用守护）→ `given_up` +
  `store.set_camera_status(cid,"error")` + 退出 desired；摄像头已删 → 退出守护。
  每次尝试打 `[supervisor] cam-N ...` 日志。
- `start_supervisor()/stop_supervisor()`：守护线程（daemon，`LS_SUPERVISOR_INTERVAL_S=10`
  等待驱动，异常不退出）；`stop_all()` 先停线程。main.py lifespan `start_all()` 后启动。
- 配置（config.py + .env.example）：`LS_SUPERVISOR_ENABLED=1 / INTERVAL_S=10 / MAX_RESTARTS=10`。

### 2. 可观测性

- `manager.status()` 每路 + `GET /api/cameras/{cid}/status` 追加
  `restart_count / last_restart_ts / supervisor_state`（新增 `supervisor_info()`，未纳守护=
  中性值）；无新路由。
- `POST /api/cameras/{cid}/start` 自动清 given_up（start_camera 内实现）。
- Cameras.vue 状态列：`backoff`→`重试中（第 N 次）`、`given_up`→`自动重启失败，请检查`，
  tooltip 带次数/时间（复用 viewFor 合并，全局四件套未动）。

### 3. 测试（test_camera_supervisor.py，13 用例，注入 clock 不真 sleep）

退避序列 1/5/30/60s、退避窗内不重复拉起、达上限 given_up+error+不再调用、用户 stop 不拉起、
手动 start 清 given_up、成功 5s 清零、删摄像头退守护、status 字段、未纳守护中性值、
ENABLED=0/MAX=0 不起线程、线程启停无残留。

### 4. 实机验证（8188 临时实例 + 临时库；坏文件源；MAX_RESTARTS=2、INTERVAL_S=1）

- 时间线：worker 打不开 `D:/definitely-missing-114.mp4` 退出 → 守护自动重启 ×2
  （每次打 `[supervisor] cam-1 worker 已退出，自动重启（第 N 次尝试）`）→
  `supervisor_state: backoff(restarts=2)` → **`given_up`**（t≈12.6s），DB status=error，
  日志 `自动重启 2 次仍失败，停止守护（given_up）`；`/api/system/shutdown` 正常退出。
- 顺带发现并处理：测试实例 lifespan 的 `go2rtc.sync_streams` 会写仓库根 `tools/go2rtc.yaml`
  （LS_GO2RTC_YAML 默认值指向仓库 tools/）——已按真实库（只读连接）再生该文件（4 路 file 源），
  临时目录已清、8188 无监听。

### 5. 文档

- 部署FAQ 新增「摄像头 worker 退出了会自动恢复吗？重试多少次？」；
- 运维手册 §4.1 worker 自动恢复；35-api-reference §8.3 守护字段与行为。

### 约束遵守

- worker 内部重连语义（65）未动；无新依赖；守护只在独立线程做，API/帧循环零改动；
- 不改变 start/stop 对外行为（除 given_up 清除语义为规格要求）。

### 遗留

- 无。下一项 123（监控机防休眠）。
