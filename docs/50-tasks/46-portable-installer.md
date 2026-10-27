# 任务 46 · 安装/卸载与干净环境验收（无管理员）

> 背景：便携包双击可用后，普通用户还需要「装到固定位置 + 桌面/开始菜单快捷方式 + 可选开机自启 + 干净卸载」。本任务用脚本实现一个不依赖 Inno/NSIS、不需要管理员的轻量安装器，并做一次 PATH 剥离的干净环境验收。

## 目标

1. **安装脚本** `安装.bat` + `scripts/install_portable.py`（标准库）：
   - 把便携目录复制到 `%LOCALAPPDATA%\Programs\LabSentinel\`（若已存在，提示先卸载或覆盖升级，保留 `data`）；
   - 创建快捷方式（PowerShell `WScript.Shell`）：
     - 桌面：`LabSentinel.lnk` → `<install>\runtime\pythonw.exe "<install>\scripts\portable_launcher.py"`
     - 开始菜单：`%APPDATA%\Microsoft\Windows\Start Menu\Programs\LabSentinel\LabSentinel.lnk` + `卸载.lnk`
   - 可选开机自启：写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\LabSentinel`（默认关闭，安装时用 `--autostart` 开启；卸载时删除）。
   - 生成 `卸载.bat`：停止进程（调用 launcher --stop）→ 删除快捷方式与 Run 键 → 询问是否删除 `%LOCALAPPDATA%\LabSentinel` 数据（默认保留，`--purge-data` 删除）。
2. **升级语义**：`安装.bat` 对已安装目录做"覆盖程序文件、保留 data/config"；版本号写在 `app/VERSION`（若没有则用 `2026.10`）。
3. **干净环境验收**（关键）：
   - 把安装目录复制到 `D:\_clean_test\LabSentinel`（避免影响本机安装）；
   - 新开 cmd：`set PATH=C:\Windows\System32`（没有 Python/Node/ffmpeg），运行启动器；
   - 验证：UI 打开、`/api/system` 正常、添加文件摄像头→出事件（证明 onnxruntime/opencv/ffmpeg 全部来自包内）；停止；
   - 记录是否有杀软/防火墙弹窗（只记录，不处理）。
4. **文档** `docs/59-portable-install.md`：安装/升级/卸载/自启开关/数据位置/FAQ（端口冲突、杀软、无网络）。
5. **报告**：安装路径、快捷方式、自启注册表、卸载保留/删除数据验证、干净 PATH 验收过程。

## 约束

- 不请求管理员权限（全部写 HKCU/用户目录）；不写系统目录/系统服务。
- 不删除用户数据，除非显式 `--purge-data`；卸载默认保留数据并打印路径。
- 不改业务代码；不训练；测试完成后停止进程并清理临时安装目录（保留真实安装目录由用户决定）。
- 快捷方式/注册表操作只在 HKCU/用户目录；不要动其他软件。

## 验收标准

- 安装后桌面/开始菜单快捷方式可启动（PATH=System32 也能跑）；
- 二次安装=升级（data 不丢）；卸载默认保留数据，`--purge-data` 删除；
- 自启开关生效/可移除；
- 干净 PATH 下完成"UI + 摄像头 + 事件"验证；
- 文档与报告齐全。

## 执行报告

（待填：安装/升级/卸载验证 / 快捷方式与自启 / 干净 PATH 事件验证 / 弹窗记录 / 遗留问题）
