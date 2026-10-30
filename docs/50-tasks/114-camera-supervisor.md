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

（待填：改动 / 重启证据 / 测试与 build / 遗留）
