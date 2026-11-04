# 任务 118 · Hyper-V 干净 VM 安装验收（P1，低负载）

> 背景：本机 Hyper-V 有现成 VM **`Win10 测试环境`**（Gen2 / 4 vCPU / Default Switch / Off），
> 已有快照 **「安装完毕」**（2026-10-30 22:07）。用它做 Windows 干净机安装/升级/卸载/自启验收。
> **严禁在 VM 里跑推理压测/24h/训练/GPU 任务**——VM 无 DML/GPU 直通，只做安装与 UI。

## 访问方式（当前用户非管理员）

- 当前 `User` 不在 Hyper-V Administrators；用 Windows 自带 `sudo`（强制新窗口）提权：
  ```
  sudo.exe -N powershell.exe -NoProfile -Command "Get-VM | Select Name,State | Format-Table"
  ```
  输出写到固定文件（`C:\Users\User\AppData\Local\Temp\ls_hv_vms.txt`）再读；不要依赖新窗口 stdout。
- 启动/停止/快照：
  ```
  sudo.exe -N powershell.exe -NoProfile -Command "Start-VM -Name 'Win10 测试环境'"
  sudo.exe -N powershell.exe -NoProfile -Command "Checkpoint-VM -Name 'Win10 测试环境' -SnapshotName 'ls-test-<ts>'"
  sudo.exe -N powershell.exe -NoProfile -Command "Restore-VMSnapshot -VMName 'Win10 测试环境' -Name '安装完毕' -Confirm:$false"
  ```
- 访客账号/密码：用户已确认 **VM/WSL 都没有设置密码**。优先用以下无凭据路径：
  1. `Copy-VMFile`（Guest Service Interface，宿主→客机，不需要客机密码）；
  2. 若 VM 配置了自动登录，`vmconnect` 打开控制台直接操作（人工步骤由用户协助）；
  3. PowerShell Direct 若必须凭据：可尝试空密码 `PSCredential`（`New-Object SecureString` 空串），
     失败就**不要猜/爆破**，写 `⏳ BLOCKED: PowerShell Direct 需要可登录会话` 并跳下一项。
- **不要**修改 VM 的密码/账户策略；无密码不等于可以重置。
- 文件进 VM（按可用性二选一）：
  1. `Copy-VMFile`（需 Guest Service Interface/集成服务；失败则换 2）；
  2. 宿主机 `python -m http.server` + VM 浏览器下载（Default Switch NAT 通常可访问宿主 IP；
     用 VM 内 `ipconfig` 找网关/宿主地址，或先用 ping 验证）。

## 前置

- **等 75 完成并重建新 zip**（含 57–74/105/便携修复）；不要用旧的 `20261029` 包做验收。
- 开始前 `Restore-VMSnapshot ... '安装完毕'` 回到干净基线；结束后再次恢复，避免污染。
- 只做低负载：启动服务后**立即通过 API 停掉所有摄像头**，不跑事件检测/压测/24h；
  VM 里预期 `backend=cpu`（无 DML），这属正常，不要求帧率。

## 验收步骤

1. VM 内确认：无 Python/Node 安装、系统时间/网络正常；记录 OS 版本/用户是否管理员。
2. 把新便携 zip 拷入 VM，解压到 `C:\Users\<user>\Desktop\LabSentinel-test`（含空格/中文路径测试可选）。
3. **标准用户**双击 `LabSentinel.exe`：应无黑框启动、浏览器打开面板；记录从双击到 UI 可用的时间。
4. 检查：`/api/system` 200、`backend=cpu`、版本正确；`/` 页面正常；`scripts\doctor.py` FAIL 0。
5. 通过 API `POST /api/cameras/bulk`（若 111 未完成则逐路 stop）停掉所有摄像头，避免持续推理。
6. 安装器：`安装.bat`（或 `install_portable.py install`）→ 桌面/开始菜单快捷方式、自启（可选）；
   从开始菜单启动一次；再运行 `卸载.bat` 选“保留数据”，确认数据目录仍在；重装后数据保留。
7. 覆盖升级：装旧 zip → 装新 zip → 数据/事件仍在、版本更新；不要求迁移大版本。
8. 卸载：快捷方式/自启清理；`--purge-data` 分支可选（在快照内做）。
9. 重启 VM（可选）：自启生效则服务自动起来；不生效记录实际行为。
10. **恢复快照**，确认 VM 回到干净基线；报告附 VM 名/快照名/步骤结果/截图路径。

## 约束

- **禁止**：训练、压测、24h、批量推理、GPU/DML 测试、在 VM 里装 Python/Node 依赖；
- 不修改宿主机 Hyper-V 配置/交换机；不删除现有快照；
- 所有写操作在快照内，结束恢复；不把 VM 凭据写进仓库/报告；
- 失败/缺凭据写 BLOCKED，不要卡住整晚。

## 验收

- 安装/启动/停止/重装/卸载/自启六项有 PASS/FAIL 表；
- 新 zip 版本与 manifest 一致；doctor FAIL 0；
- VM 已恢复「安装完毕」快照；宿主机无残留进程/端口；
- 报告明确“VM 仅安装/UI 验收，不代表真实摄像头 24h/DML 性能”。

## 执行报告

**flash 2026-10-31 部分完成（用户协作手动验收；PS Direct 空密码被拒，转人工）**

### 通道探索（自动化受阻记录）
- `sudo.exe -N powershell` 提权通道 ✓（Get-VM/快照/防火墙均可用）；
- 快照「安装完毕」在（10-30 22:07）；**来宾服务接口=False** → Copy-VMFile 不可用；
- PowerShell Direct 空密码：`User`/`user`/`Administrator` 三用户全部"凭据无效"→ 规格授权路径尽，
  **未猜未爆破**；http.server+boot.bat 自动化通道搭成后 VM 内文件异常消失（疑似杀软/清理），
  用户决定转手动验收（用户指示：不恢复快照，当前即干净基线）。

### 手动验收结果（用户提供现场输出+日志，均已归档分析）
| # | 项 | 结果 | 证据 |
|---|---|---|---|
| 1 | 双击 exe 无黑框+浏览器弹面板 | ✅ | launcher 15:59:52→就绪 **10.3s**；UI 200/WS/资产全通 |
| 2 | backend | ✅ 预期 cpu | DML EP Error→回退 CPU（VM 无 GPU 属正常） |
| 3 | doctor（解压根） | ✅ FAIL 0 | 9 PASS/1 WARN（WARN=服务占用 8000 属预期）；`config=√` |
| 4 | UI 删摄像头 | ✅ | DELETE /api/cameras/1..8 全 200 |
| 5 | 安装.bat | ✅ | 版本 2026.10/快捷方式/数据目录外置；覆盖升级路径实测 |
| 6 | 开始菜单启动 | ✅ | 6.1s 就绪 |
| 7 | 数据外置连续性 | ✅ | 换安装位置事件 id 38→39 连续（同一库） |
| 8 | 卸载 | ❌→已热修 | `do_uninstall` 停服 spawn PermissionError 崩溃（exe 缺失+runtime
  PENDING_DELETE 状态）；**120 已修**（三级降级+taskkill 兜底），待 VM 用新包复测 |
| 9 | 控制面板卸载项 | ➡120 | 原版无；120 已加 HKCU Uninstall 键 |
| 10 | Web 停止 | ➡120 | 用户需求；POST /api/system/shutdown + UI 按钮已交付 |

### 119 修复的 VM 实证
- **go2rtc 路径**：桌面副本写 `<包>\tools\go2rtc.yaml`、安装副本写
  `...\Programs\LabSentinel\tools\go2rtc.yaml`——**均包根，app\tools 零写入** ✓
- 就绪窗口：10.3s/6.1s 远低于 60s ✓

### 发现（转建议）
- **seed-if-empty 复活**：用户删光摄像头→重启重播 4 路演示（设计如此但反直觉）；
  建议首启播种后落标记，只对全新库播种（P3）；
- **auto-backup 便携失效**：`[auto-backup] ✗ backup_data.py 不存在`（parents[2] 布局坑）——
  **发现即修**（backups.py+validate.py 双趟根推导，pytest 回归覆盖）；
- VM 内 `python=True`（PATH 有 python）——"无 Python"前提不成立，如实记录。

### 清理与遗留
- 宿主机：传输服务器已杀、防火墙规则 `LS118-temp-in-8188` 已删 ✓；
- VM：测试产物（Desktop 解压目录+安装目录+数据目录）待用户按需清理/恢复快照（用户自决）；
- 遗留：①新包（sha256 4e2ed006…）在 VM 复测完整安装/卸载/控制面板链；②升级链（旧 20261029→新）
  未跑（自动化中断后用户跳过）；③报告不含 24h/DML 声明——VM 仅安装/UI 验收。

## 复测记录补全（2026-11-04，主线确认"用户已复测"，flash 仅记录，不重跑）

- 主线复审（docs/72 §一）确认：**用户已用新包在 Hyper-V 完成复测**；本任务不再重跑，
  也不占用队列。据 10-31 手动验收 + 120/119 修复内容，VM 侧遗留三项的预期状态：
  1. 卸载崩溃 → 120-b 已修（停服三级降级 + taskkill 兜底 + 全子进程 CREATE_NO_WINDOW），
     本机隔离端到端 10/10 PASS（含卸载 2s、注册表清除、数据保留）；
  2. 控制面板卸载 → 120 已交付（HKCU Uninstall 九键）；
  3. Web 停止 → 120 已交付（`POST /api/system/shutdown` + Cameras 页「服务控制」卡）。
- 复测所用包：`LabSentinel-portable-20261103.zip`（sha256 `6073b88f…`，含 121/106-108）——
  122 已全链验收（smoke 8/8、包内 doctor 10/0/0、冷启动 20.0s）。
- 结论口径不变：**VM 仅安装/UI 验收，不代表真实摄像头 24h / DML 性能**；
  118 就此关闭，不阻塞后续任务。
