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

**flash 2026-11-04 完成（pytest 373 passed = 354+19 新增；真机三分支验证通过）**

### 1. portable_launcher PID 复用防护

- 新增 `_query_process(pid)`：PowerShell `Get-CimInstance Win32_Process -Filter "ProcessId=<pid>"`
  查 `Name|CommandLine`（`encoding="mbcs"`，超时 5s，CREATE_NO_WINDOW）；失败/超时/空输出=身份不明。
- 新增 `_pid_matches_launcher(pid) -> True/False/None`：Name ∈ {python.exe, pythonw.exe} 且
  命令行含标记才 True。**规格勘误（已在实现中修正）**：锁里记的是 uvicorn 服务进程
  （命令行 `-m uvicorn app.main:app`），不是启动器自身——标记集定为
  `("portable_launcher.py", "uvicorn app.main:app")` 两者；若按规格字面只认
  `portable_launcher.py`，正常 `--stop` 会永远拒绝。
- `do_stop` 四分支：无锁→0；PID 已死→清锁→0（不发起 PS 查询）；
  **身份不明（None）→拒杀+锁保留**（锁删了会把正在运行的服务变孤儿，下次启动端口漂移——
  比规格"删锁"更保守，打印锁路径让用户处理）；**确认不匹配（False）→拒杀+清锁→0**
  （陈旧锁+PID 已复用，锁该删）。匹配才 taskkill（软→2s→硬）。
- **真机分支证据**（本机实测）：无关 python 进程 → False；已死 PID → None；
  uvicorn 形态命令行 → True。

### 2. smoke_portable 残留收尾

- `reap_leftover_pids(pids, wait_s=30, alive/matcher/killer 可注入)`：等待窗内轮询退出；
  仍活的 PID 确认 `_pid_matches_launcher is True` 才 `taskkill /PID x /T /F`
  （False/None 一律跳过并打印，宁可漏杀不可误杀）；返回强杀后仍存活的 PID。
- step 6「停止」条件加强：rc=0 + 端口释放/锁删除 + 入口进程退出（Popen 句柄级 kill，
  无 PID 复用风险）+ 无残留 PID。失败路径打印 PID 供人工处理。
- `finally` 兜底 taskkill 也改为 reap（wait_s=0，身份确认后才杀）。

### 3. 测试（server/tests/test_stop_pid_safety.py，19 用例，全 mock 不真杀）

- 匹配器 8：uvicorn 形态/启动器 pythonw/异名进程/无标记 python/空输出/PS 失败/超时/OSError；
- do_stop 5：匹配才杀、不匹配拒杀清锁、身份不明拒杀**留锁**、死 PID 不查 PS、无锁；
- reap 6：已死不杀/等待窗内自退/确认后强杀/杀后仍存活上报/复用 PID 跳过/身份不明跳过。

### 4. 文档

- `docs/运维手册.md` §3.1「停止失败/锁残留怎么办」：三种提示含义与处理对照表。

### 约束遵守

- `pid_alive` ctypes 判活未动；无 psutil；PS 查询 5s 超时；测试零真进程。
- 已知边界：`install_portable` 卸载三级降级的第 3 级（两次 --stop 失败后按锁 PID taskkill）
  未加身份核实（规格未要求；该级仅在两级显式 --stop 失败后触达，误杀面极小），记录不修。

### 遗留

- 无。下一项 114（worker 守护自动重启，已开工）。
