# 任务 110 · 停止进程的 PID 复用防护 + 冒烟清理（P1）

> 来源：flash 审阅建议 #5/#7。`portable_launcher.do_stop` 按锁内 PID 直接杀；锁残留 + PID 被系统复用
> 时会误杀无关进程。`smoke_portable` 兜底只杀 uvicorn，启动器 pythonw 在 30s 等待窗内可能残留弹窗。

## 目标

1. `portable_launcher` 新增 `_pid_matches_launcher(pid) -> bool`：
   - Windows 用 `Get-CimInstance Win32_Process -Filter "ProcessId=<pid>"`（PowerShell，`encoding="mbcs"`）
     查询 `Name` 与 `CommandLine`；
   - 仅当进程名是 `python.exe/pythonw.exe` 且命令行包含 `portable_launcher.py` 才允许 kill；
   - 查询失败（权限/PS 不可用）→ **拒绝 kill 并打印锁路径**，让用户手动处理（安全优先）；
   - `do_stop` 在 taskkill 前调用；不匹配时删锁并返回 0（说明“锁已陈旧，未杀进程”）。
2. `smoke_portable`：
   - 启动前记录 lock 中 launcher PID；
   - `--stop` 后若 launcher PID 仍存活（30s 等待窗），用 `taskkill /PID <pid> /T /F` 清理；
   - 最终断言无该 PID、端口释放；清理失败打印 PID 供人工处理。
3. 测试：
   - `test_stop_pid_safety.py` 单测 `_pid_matches_launcher`（monkeypatch subprocess 输出：
     匹配/不匹配/空输出/PS 失败）；
   - `do_stop` 在匹配时才调用 taskkill（mock 记录），不匹配/查询失败时不杀；
   - smoke 的清理函数用假 PID 单测（不真杀）。
4. 文档：运维手册“停止服务/锁残留怎么办”。

## 约束

- 宁可拒绝杀也不能误杀；不动 `pid_alive` 的 ctypes 判活；
- 不引入 psutil；PowerShell 查询超时 5s；
- 测试不真启动/杀进程。

## 验收

- `pytest tests/ -q` 全绿；报告附匹配/拒绝分支证据；
- 便携包 `--stop` 正常路径仍 rc=0（75 重建后可复测）。

## 执行报告

（待填：改动 / 分支证据 / 测试 / 遗留）
