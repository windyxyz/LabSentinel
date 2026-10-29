# 任务 65 · 摄像头状态可观测性（错误/重连/帧龄）

> 背景：现在 `/api/system` 里每路只有 `id/fps/status`；摄像头打不开或掉线时用户只看到 `error/stopped`，
> 不知道原因、不知道是否在自动重连、也不知道画面是否卡住。本任务把 worker 的运行细节暴露出来，
> 并在摄像头页给出可读状态。

## 目标

### 1. `CameraWorker` 运行指标

新增实例字段（线程安全：状态字段用 `self._stat_lock` 或简单赋值，读取允许轻微竞态）：

- `last_frame_ts: float | None`：最后一次成功读到帧的 `time.time()`；
- `last_error: str | None`：最近一次打开失败/读帧失败/分析异常的简短原因（截断 200 字，**不含凭证**：
  若 source 是 RTSP，错误文案里不能出现 `user:pass@`）；
- `last_error_ts: float | None`；
- `reconnect_count: int`：进入重连分支的次数；
- `opened_at: float | None`：本次成功打开的时间；
- `frames_total: int`：累计读到的帧数。

行为调整（保守）：

- `run()` 打开失败时：设置 `last_error`，`set_camera_status(cid,"error")`；对 `rtsp`/`usb` 源做
  **有限重试**（例如 5 次，间隔 3s，可用常量），仍失败再退出线程；`file` 源保持直接退出（坏路径不反复刷）。
- 读帧失败进入重连分支时：`reconnect_count += 1`、设置 `last_error`；重连成功清空 `last_error`、
  更新 `opened_at`、`set_camera_status(cid,"running")`。
- 分析异常只记 `last_error`（前缀 `analyze:`），不改变 running 状态（沿用现有自愈语义）。

### 2. 状态输出

- `CameraManager.status()` 每路增加：`last_frame_ts`、`frame_age_s`（无帧为 null）、`last_error`、
  `last_error_ts`、`reconnect_count`、`opened_at`、`frames_total`；
- 新增 `GET /api/cameras/{id}/status`（读保护口径同其它 GET）：返回单路上述字段 + DB 里的
  `name/type/status`；摄像头不存在 404。
- `/api/system` 保持兼容：只追加字段，不删除/改名现有键。

### 3. 前端 `web/src/views/Cameras.vue`

- 状态列：`running` 绿、`error` 红、`stopped` 灰；旁边显示 `fps`；
- 有 `last_error` 时显示红色小字（截断 60 字 + `title` 全文）；
- 显示 `frame_age_s`：>10s 标黄「画面停滞」；`reconnect_count>0` 显示「重连 N 次」；
- 不引入图表库；只改本组件（可改 `web/src/api.js` 加 `cameraStatus`）。

### 4. 测试 `server/tests/test_camera_status.py`

- 用假 worker 对象（`is_alive` 返回 True、带上述字段）monkeypatch `manager.workers`，
  断言 `status()` 字段与 `frame_age_s` 计算（用固定 `last_frame_ts`）；
- `GET /api/cameras/{id}/status` 200/404、鉴权开关两种口径；
- 错误文案脱敏函数单测：`rtsp://u:p@host/x` → 不含 `u:p`；
- 不启动真实摄像头；不推理。

### 5. 文档

- `docs/35-api-reference.md` 增加单路状态端点与字段说明；
- `docs/部署FAQ.md` 增加「摄像头显示 error/画面停滞怎么看」：看 last_error、重连次数、
  检查网络/账号/子码流/编码。

## 约束

- 不改判定/告警逻辑；不改 `capture_source`；不引入依赖；
- 前端只改 `Cameras.vue`/`api.js`，不改全局样式四件套；
- 测试不占 8000/8001（TestClient 或假对象）。

## 验收标准

- `pytest tests/ -q` 全绿（基线 91 + 新增）；`npm run build` 通过；
- 8001 临时实例用文件源启动一路：`/api/cameras/{id}/status` 返回 `frames_total` 增长、
  `frame_age_s` 很小；停掉后 `status=stopped`；
- 报告附实际 JSON 与前端 build 结果，不编造 fps。

## 执行报告

**flash 2026-10-30 完成（pytest 130 passed、build 43.43KB、8001 实跑指标流动验证）**

- **改动**：`pipeline.py`——CameraWorker 新增 6 个指标字段（last_frame_ts/last_error/last_error_ts/
  reconnect_count/opened_at/frames_total）+ `redact_creds()`（URL 凭证 `***:***@` + 200 字截断）；
  打开失败/断流改**有限重连**（rtsp/usb 5 次×3s，file 直接退出），重连成功清错误+刷新 opened_at+
  置 running，重试耗尽 `_exit_status="error"`（finally 按它写库，不再一律 stopped）；分析异常只记
  `analyze:` 前缀错误不改状态（沿用自愈语义）；CameraManager.status() 每路追加全部指标 + frame_age_s。
  `api.py` 加 `GET /api/cameras/{id}/status`（无 worker 的摄像头返回零值指标；404 语义）。
  `Cameras.vue` 状态列三态着色 + fps + 红字 last_error（60 字截断+title 全文）+ 画面停滞黄字（>10s）
  + 重连次数；`refresh()` 并拉 /api/system 合并（viewFor）。
- **测试**：`test_camera_status.py` 4 用例（脱敏函数含截断/manager.status 字段与 frame_age_s 固定值
  /端点 200·404·鉴权双口径/system 路由新字段）。全量 **pytest 130 passed**（126 + 4）。
- **实跑**（8001 + 文件源循环）：`{"status":"running","fps":21.4,"frame_age_s":0.0,
  "frames_total":191,"reconnect_count":0,"last_error":null}`，**frames_total 126→191 持续增长**；
  删除摄像头后 /api/system cameras=[]。
- **build**：通过，gzip **43.43KB**（43.16 → +0.27KB）。
- **文档**：35-api-reference 加 §8.2（单路指标字段/有限重连行为）；部署FAQ 加「摄像头显示 error/
  画面停滞怎么看」排查顺序。
- **遗留**：无。断线重连的实测（杀 go2rtc）已在 docs/54 §4 演练过 ≈2s 恢复，本次为指标可观测化。
