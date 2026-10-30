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
- 访客账号/密码：**必须由用户提供**；不得猜测/爆破。没有凭据就在报告写 `⏳ BLOCKED: 需要 VM 登录凭据`。
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

（待填：VM/凭据来源 / 六项结果 / 截图 / 快照恢复 / 遗留）
