# 任务 120 · 安装器/驱动拆分 + 控制面板卸载 + Web 停止（P1，用户直接指示）

> 来源：2026-10-31 用户在 VM 验收（118）中直接提出的产品化需求。核心诉求：
> ① 双击包 exe 不再永远按便携处理；② 安装像正常软件（控制面板可卸载）；
> ③ 重复安装给三选；④ 启停脚本智能定位；⑤ 网页端一键停止。

## 需求（用户原话归纳）

1. 双击 exe → 弹选项：保留便携版 / 指定位置安装。
   - 保留便携 → 后续按便携走；
   - 指定位置安装 → 后续双击包 exe 自动转到安装副本启动（**驱动与安装逻辑必须分离**，
     否则装完再点包还会被当成便携位置）。
2. 正常写注册表，控制面板"卸载程序"可卸载。
3. 再次点击安装时先检测是否已装：已装 → (a) 跳过直接启动 (b) 覆盖安装保留数据 (c) 清空数据重装。
4. 启动/停止脚本保留，优先作用于已安装位置，无安装则作用于包所在处。
5. 网页服务端提供"停止"：点完停止、关浏览器，一切停掉。

## 交付

### 1. 程序拆分
- **驱动**（启停）= `LabSentinel.exe` + `scripts/portable_launcher.py`：
  - `decide_launch_mode()`（纯函数）：无人值守一律便携（保 59 冒烟隔离语义）；
    交互 + 注册表已装 → 转启安装副本；交互 + 无装 + 无标记 → 首启三选
    （MessageBox 是=便携 / 否=安装 / 取消=退出，ctypes 标准库）；选便携落 `launch.mode` 标记；
  - `relaunch_installed()`：转启失败回落便携（不留死路）；
  - 安装动作全部移交安装器，驱动不再自装。
- **安装器 UI** = 新 `scripts/installer.py`（`install --ui` / `uninstall --ui`）：
  - 已装三选（跳过启动 / 覆盖保留 / 清空重装——清空需二次确认+等 PENDING_DELETE 释放）；
  - 未装 → FolderBrowserDialog 选目录（非 LabSentinel 名自动追加子目录）→ 询问自启 → 安装；
  - 复用 `install_portable.do_install/do_uninstall`（引擎不动，测试口径不变）。

### 2. 控制面板卸载（install_portable.py）
- `set_uninstall_registry()`：HKCU Uninstall\LabSentinel 九键
  （DisplayName/Version/Publisher/InstallLocation/DisplayIcon/UninstallString/NoModify/NoRepair/SystemComponent）；
- `installed_location()`：读 InstallLocation，目录含 LabSentinel.exe 才有效（半删残留不算）；
- `remove_uninstall_registry()`：卸载同步清除；do_install/do_uninstall 已接线。

### 3. 卸载崩溃热修（118 VM 实测 WinError 5）
- `do_uninstall` 停服三级降级：exe --stop → runtime/当前 python --stop → 按锁 PID taskkill；
  全 try 包裹，停止失败不再炸卸载。

### 4. 启停脚本智能定位（scripts/portable_start.bat / portable_stop.bat）
- 先 `reg query` InstallLocation 且存在 LabSentinel.exe → 作用于安装副本；
- 否则按包所在目录（ASCII 内容，GBK 规则不变）。

### 5. Web 停止
- `POST /api/system/shutdown`（require_admin 口径与写接口一致）：响应后 ~0.8s
  清启动器锁（data 上一级 logs/launcher.lock）→ `os._exit(0)`（与 taskkill /F 同级口径，WAL 崩溃安全）；
- Cameras.vue 新「服务控制」卡：停止按钮 + 双重确认 + 提示；api.js `shutdownServer()`。

### 6. 构建接线（build_portable.py）
- 包 scripts/ 增拷 installer.py；`安装.bat` 改调 `installer.py install --ui`。

## 测试

- `test_launch_mode.py`（12）：decide_launch_mode 四分支 / installed_location 有效·半删·reg 失败 /
  set_uninstall_registry 全键+部分失败 / remove / **卸载停服三级降级回归**（spawn PermissionError →
  taskkill 兜底不中断）/ relaunch 成功·失败回落；
- `test_shutdown_endpoint.py`（3）：端点调度 / _do_shutdown 清锁 / 无锁不崩；
- test_installer_options.py fixture 补注册表隔离（不许碰真实 HKCU）。

## 约束

- 标准库 only（对话框=ctypes，目录选择借 PowerShell FolderBrowserDialog）；
- 不动引擎/业务逻辑；bat 保持 ASCII；无人值守语义不回归；
- 不 commit/不发布。

## 验收

- pytest 全绿；npm build 通过；重建 zip + manifest --verify + smoke 8/8；
- 包内安装.bat 指向 installer.py --ui；启动/停止.bat 智能定位逻辑在位。

## 执行报告

**flash 2026-10-31 完成（120-b 终版：pytest 312、本机端到端 10/10、smoke 8/8；zip sha256 702f30a8…）**

- 首版交付后**本机自检实测暴露 3 个回归**（用户在 VM 首点"否"后 reg.exe 黑窗风暴定位）：
  1. **驱动自转启 fork 炸弹（严重）**：安装副本的驱动检测"已安装"又转启自己 → 无限接力，
     每世代从 pythonw 查注册表闪一个 reg.exe 窗（VM/本机双复现）→ 修：`decide_launch_mode`
     加 samefile 自检（app_root==installed → portable），+2 单测回归；
  2. **双实例**：安装器装完自启 + 驱动又转启 → 修：驱动 install 分支让位退出，启动归安装器；
  3. **闪窗**：pythonw 无控制台时每个 reg/taskkill/powershell 子进程弹一窗 →
     install_portable（11 处）/installer/portable_launcher 统一 CREATE_NO_WINDOW。
- **本机端到端自检（隔离 LOCALAPPDATA+注册表实测）10/10 PASS**（43s）：静默安装 6s →
  HKCU 卸载项 → 安装副本启动 13.2s → Web 停止（resp ok/锁删/进程退）→ 卸载 2s →
  注册表清除/数据保留/目录延迟自删/零残留。harness 固化双保险：强制无人值守
  （LS_LAUNCHER_NO_BROWSER+NO_MESSAGEBOX）+ finally 兜底清理。
- **事故记录**：首版自检误用修复前 zip（4e2ed006）交互拉起 → 宿主机复现风暴；已止血，
  教训=自检前必须重建包（已写入发布核对报告 120-b 小节）。
- pytest **312 passed**；npm build 50.85KB；smoke 8/8；manifest --verify OK。
- 遗留：① VM 交互链（首启三选/覆盖三选/控制面板卸载）待用户用 702f30a8 包复测；
  ② 自检 harness 的 3b 进程计数对 python.exe 服务进程不敏感（真空转，回归主要靠单测）——记录不修。
