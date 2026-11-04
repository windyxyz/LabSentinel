# 任务 123 · 监控机防休眠（P1，24h 前置）

> 来源：flash 建议 #20（PowerToys Awake 同源 API）。Windows 监控机自动睡眠/硬盘休眠会让
> 摄像头分析、告警链直接中断，是真实 24h 验收的前置保障。**不改用户电源计划**，只在服务运行期间
> 临时阻止系统睡眠，退出即恢复。

## 目标

1. 服务端新增可选的 keep-awake：Windows 下调 `SetThreadExecutionState` 持锁；
   非 Windows 自动 no-op（不报错）。
2. 配置（`LS_` 前缀，默认关）：
   - `LS_KEEP_AWAKE=true|false`（默认 false）
   - `LS_KEEP_AWAKE_DISPLAY=true|false`（默认 false；true 时同时阻止显示器关闭）
3. `/api/system` 返回 `keep_awake: {enabled, active, display}`；Dashboard/摄像头页可选展示（不强制）。
4. 服务启动时启用、lifespan 退出时释放；重复 start/stop 幂等。
5. 文档：`docs/部署FAQ.md` 或运维手册加“24h 挂机建议开启”说明；便携 `config.env.template` 加注释示例。

## 实现建议

- 新 `server/app/keepawake.py`：
  - `start(enabled, display=False)`：Windows 用 `ctypes.windll.kernel32.SetThreadExecutionState(
    ES_CONTINUOUS | ES_SYSTEM_REQUIRED [| ES_DISPLAY_REQUIRED])`；后台 daemon 线程每 30–60s 重新断言
    （部分驱动/组策略会重置），停止时 `SetThreadExecutionState(ES_CONTINUOUS)` 解除。
  - `stop()` 幂等；`status()` 返回 enabled/active/display。
  - 所有 Win32 调用用 try/except 包裹；失败只 WARNING，不影响服务。
- `main.py` lifespan：读取 settings，`keepawake.start(...)`；finally `keepawake.stop()`。
- `api.system()` 合并 `keepawake.status()`。
- 不修改电源计划、不写注册表、不需要管理员。

## 测试（`server/tests/test_keepawake.py`）

- fake `ctypes`/`kernel32`：start 传对 flags（含/不含 display），stop 调 ES_CONTINUOUS；
- 重复 start/stop 不重复起线程、不抛；
- 非 Windows（monkeypatch `sys.platform`）no-op 且 status.active=false；
- keepawake 线程异常时服务启动仍成功（fake 抛错 → start 返回、只记日志）。
- 不实际调用 Win32（测试环境可能无桌面会话）。

## 验收

- `pytest tests/ -q` 全绿；`/api/system` 在 `LS_KEEP_AWAKE=1` 下返回 active=true（本机 Windows 实测可做，
  但测试不依赖）。
- 说明文字明确：这是“阻止系统睡眠”，不是“阻止用户注销/关机”。

## 约束

- 不新增依赖（ctypes 标准库）；不改 launcher/安装器；不跑 24h。
- 默认关，避免未经用户同意改变机器行为；便携包文档建议 24h 验收时开启。

## 执行报告

**flash 2026-11-04 完成（pytest 397 passed = 386+11 新增；8188 实机 active=true 验证）**

### 实现（server/app/keepawake.py，新文件）

- 核心语义：`SetThreadExecutionState` 是 **per-thread** 效果——断言与解除必须同线程配对。
  专用 daemon 线程内完成：首断言（`ES_CONTINUOUS|ES_SYSTEM_REQUIRED[|ES_DISPLAY_REQUIRED]`）
  → 每 `REASSERT_S=45s` 重断言（部分驱动/组策略会重置）→ stop 时同线程
  `SetThreadExecutionState(ES_CONTINUOUS)` 解除（PowerToys Awake 同款）。
  进程意外退出 OS 自动清锁，不会把机器锁成永久不睡。
- `start(enabled, display)` 幂等（已跑不重置状态/不起二线程）；首断言结果用
  `_first` Event 回传（**不能 join 常驻线程**——首版 join(3s) 会把 start 卡满 3 秒，已修）；
  任何失败只打印告警，绝不影响服务启动；非 Windows no-op。
- `stop()` 幂等；`status()` → `{enabled, active, display}`。

### 接线

- config.py：`keep_awake=false / keep_awake_display=false`（LS_KEEP_AWAKE / LS_KEEP_AWAKE_DISPLAY，默认关）；
- main.py lifespan：`start_all` 后 `keepawake.start(...)`，finally `keepawake.stop()`；
- api.py `/api/system` 追加 `"keep_awake": keepawake.status()`；
- `.env.example` 新节 + 便携 `.env.template`（build_portable.py 生成处）加"24h 挂机建议开启"注释
  （launcher 只透传 LS_* 行，取消注释即生效）；
- 运维手册 §12 前加"挂机前置"说明（含"不阻止注销/关机/Windows 更新重启"边界）。

### 测试（test_keepawake.py，11 用例，fake kernel32 不真调 Win32）

flags 含/不含 DISPLAY、解除=仅 CONTINUOUS、重断言循环（REASSERT_S 注入 0.05s）、
disabled no-op、非 Windows no-op、返回 0 不生效、首断言抛错服务仍启动、重断言失败翻 inactive
且线程退出不累积调用、重复 start 幂等不重置、stop 幂等/未 start 安全、stop 后线程退出。

### 实机验证（8188 临时实例 + 临时库，LS_KEEP_AWAKE=true）

- `/api/system` → `keep_awake={'enabled': True, 'active': True, 'display': False}`（4.4s 就绪）；
- `POST /api/system/shutdown` → 进程干净退出（finally stop 解除）。

### 修过的坑（实现期自测抓到）

1. `start()` 用 `thread.join(3.0)` 等"首断言"——join 等的是**线程退出**，常驻循环下每次 start 卡 3s
   且失败用例时序错乱 → 改 `_first` Event 只等首断言结论；
2. 幂等分支在检查"已跑"**之前**就 `active=False` 重置 → 二次 start 恒返回 False → 检查前置。

### 约束遵守

- ctypes 标准库零依赖；不改 launcher/安装器/电源计划；默认关；测试不依赖真桌面会话。

### 遗留

- 无。下一项 102（处置质量统计）。
