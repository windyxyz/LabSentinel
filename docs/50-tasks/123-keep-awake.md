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
