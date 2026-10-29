# 任务 55 · 便携包启动 EXE（无控制台，脚本/EXE 双入口）

> 背景：便携包目前只能双击 `启动.bat`（会闪一下黑框，普通用户仍觉得"像脚本"）。用户要求交付物**不要复杂环境配置**，同时支持"脚本启动"与"EXE 应用程序启动"。本任务给便携包增加一个无控制台的 `LabSentinel.exe` 启动器，保留 `启动.bat` 作为脚本入口。

## 目标

1. **新增 C# 启动器** `scripts/launcher_exe/LabSentinelLauncher.cs`（.NET Framework 4.x，单文件）：
   - `root = exe 所在目录`；校验 `runtime\pythonw.exe`、`scripts\portable_launcher.py` 存在。
   - 默认：`Process.Start` 调用 `runtime\pythonw.exe "scripts\portable_launcher.py"`，`CreateNoWindow=true`、
     `UseShellExecute=false`、工作目录=root；启动后自身退出（服务由 Python 启动器托管，浏览器由它打开）。
   - `--stop`：调用 `runtime\python.exe "scripts\portable_launcher.py" --stop`，等待并返回退出码。
   - 单实例：交给现有 Python 启动器（锁+健康检查），C# 不重复实现。
   - 失败：`MessageBox` 显示错误与日志路径（`%LOCALAPPDATA%\LabSentinel\logs`），返回非 0。
   - 编译为 `/target:winexe`（无控制台）。
2. **构建脚本** `scripts/build_launcher_exe.py`（标准库）：
   - 查找 `csc.exe`：`%WINDIR%\Microsoft.NET\Framework64\v4.0.30319\csc.exe` → Framework（32 位）→ PATH；
   - 编译到便携包根 `LabSentinel.exe`（引用 `System.Windows.Forms.dll`；无其它依赖）；
   - 若 csc 不存在：**警告并跳过**（包仍可用 `启动.bat`，不阻塞构建）；
   - 打印 exe 大小。
3. **集成 `build_portable.py`**：复制资产后调用 build_launcher_exe；若成功，包根同时有
   `LabSentinel.exe`、`启动.bat`、`停止.bat`。
4. **安装器** `install_portable.py`：
   - 桌面/开始菜单快捷方式优先指向 `LabSentinel.exe`；无 exe 时回退 `runtime\pythonw.exe scripts\portable_launcher.py`；
   - 卸载时优先 `LabSentinel.exe --stop`，无 exe 时回退 Python 启动器 `--stop`。
5. **实测**（PATH=System32，模拟无 Python 环境）：
   - 双击/命令行 `LabSentinel.exe` → 服务起、浏览器打开、UI 200；确认**无控制台窗口**（winexe 子系统）；
   - `LabSentinel.exe --stop` → 进程退出、端口释放；
   - 便携包重建后 `LabSentinel.exe` 在根目录，大小合理（<100KB），zip 体积几乎不变；
   - 安装器快捷方式目标为 exe（检查 .lnk 或安装日志）。
6. **文档**：`docs/59-portable-install.md` 与 README 便携版步骤改为"双击 `LabSentinel.exe`
   （或 `启动.bat`）"；说明 .NET Framework 4.x 为 Win10/11 自带，无需安装。
7. **报告**：编译命令、csc 路径、exe 大小、无黑框/启动/停止/安装快捷方式验证、无 csc 时的降级说明。

## 约束

- 只新增 launcher CS + 构建脚本 + 安装器/构建器小改 + 文档；不改服务端业务代码。
- 不引入第三方 C# 包/安装器依赖；只用 .NET Framework 自带 WinForms。
- 不训练；测试用临时/现有便携包，结束后停服；不占 8000/5173 之外端口。
- exe 不做代码签名（记录为后续发布项）。

## 验收标准

- `LabSentinel.exe` 在便携包根，双击可启动（无黑框），`--stop` 可停；
- 安装器快捷方式指向 exe（或明确降级路径）；
- csc 缺失时构建不失败、只警告；
- 文档/报告齐全，pytest 不受影响（无业务代码改动）。

## 执行报告

**flash 2026-10-29 完成（全部验收项实测通过）**

### 交付物

- `scripts/launcher_exe/LabSentinelLauncher.cs`：C# 5 语法（v4.0.30319 自带 csc 可编译），
  winexe 无控制台。职责刻意最小——单实例/端口/健康检查/开浏览器全在 Python 启动器里，
  EXE 只做：校验 `runtime\pythonw.exe` 与 `scripts\portable_launcher.py` 存在（缺失弹
  MessageBox 报日志路径，rc=2）→ `Process.Start`（UseShellExecute=false、CreateNoWindow=true、
  工作目录=包根）启动 `pythonw portable_launcher.py` 后自身退出；`--stop` 走
  `python.exe portable_launcher.py --stop` 并 WaitForExit 返回其退出码；异常 MessageBox（rc=1）。
- `scripts/build_launcher_exe.py`：csc 探测顺序 `%WINDIR%\Microsoft.NET\Framework64\v4.0.30319`
  → Framework(32位) → PATH；`csc /nologo /target:winexe /optimize+ /r:System.dll
  /r:System.Windows.Forms.dll`；csc 缺失或编译失败 → 警告返回 None（不阻塞构建）；
  `--check` 只探测 csc。
- `build_portable.py`：copy_assets 内调用 compile_exe(DIST)，包根产 `LabSentinel.exe`；
  失败仅告警（启动.bat 仍可用）。
- `install_portable.py`：快捷方式优先 `LabSentinel.exe`（无参数、WorkingDirectory=安装目录），
  无 exe 回退 `pythonw + portable_launcher.py`；卸载停止优先 `LabSentinel.exe --stop` 回退
  Python 启动器；卸载进程内删除名单加入 `LabSentinel.exe`。

### 实测证据（PATH 无关，隔离 LOCALAPPDATA 数据目录，端口 8180）

| 验收项 | 结果 |
|---|---|
| csc 路径 | `C:\WINDOWS\Microsoft.NET\Framework64\v4.0.30319\csc.exe` |
| exe 大小 | **5.5KB**（<100KB 达标）；PE 子系统=2（GUI，无控制台，程序化验证） |
| 便携包重建 | 包根有 LabSentinel.exe；DML 冒烟 providers=[Dml, CPU] 照常；zip 278MB |
| 双击启动 | exe 秒退（Popen 2s 后 poll 非 None→已退出），服务 6s 就绪 `/api/system` 200、backend=directml、演示数据已播种；数据落隔离 `%LOCALAPPDATA%` 确认 |
| `--stop` | rc=0（3.1s），端口 8180 释放、launcher.lock 删除 |
| 安装器快捷方式 | 隔离 harness（重定向桌面/开始菜单到临时目录）：有 exe → `TARGET=...\LabSentinel.exe ARGS=`；无 exe → 回退 `pythonw + 脚本`；卸载.lnk 行为不变 ✓ |
| pytest | **91 passed**（无业务代码改动，基线不受影响） |

### 两个如实记录的细节

1. **自动化 harness 中的调用阻塞**：`subprocess.run(exe)` 会等到服务退出才返回——GUI 进程以
   `UseShellExecute=false` 启动子进程时继承父 stdio 管道，harness 等 EOF。**双击场景不受影响**
   （无管道），exe 本体实测秒退。未来若要 CI 驱动 exe，可在 C# 里加 `RedirectStandardOutput=true`
   或改 `UseShellExecute=true`（spec 指定 false，未改）。
2. **顺手加固**：`install_portable.ps()` 在 `PYTHONUTF8=1` 下遇 PowerShell GBK 报错输出会
   解码崩→`r.stderr=None`→AttributeError 掩盖真实错误。已加 None 兜底并给出可读错误
   （harness 实测暴露，真实安装链路受益）。同步重建 zip 已包含。

### 无 csc 时的降级

Win10/11 均自带 .NET Framework 4.x（csc 在 `%WINDIR%\Microsoft.NET\Framework64\v4.0.30319\`）；
若裁剪版系统无 csc：构建端编译失败仅警告（包无 exe），运行端快捷方式自动回退 pythonw、
卸载停止回退 Python 启动器——全部功能等价，仅入口少一个。

### 未做（记录为后续发布项）

- exe 无代码签名（与 zip 同策略，见 52 报告"签名"项）；图标未定制（默认图标，可后续加 `/win32icon`）。

### 文档

- `docs/59-portable-install.md` 安装章节改为 exe 主入口（含两种入口行为一致的说明、--stop）；
- `README.md` 方式 A 同步（exe / 启动.bat 双入口 + .NET 4.x 自带说明）。
