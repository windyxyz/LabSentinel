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

（待填：csc 路径/编译命令 / exe 大小 / 启动与停止验证 / 快捷方式 / 降级说明）
