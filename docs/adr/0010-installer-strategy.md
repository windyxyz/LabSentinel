# ADR-0010：Windows 安装包策略（Inno Setup 双形态）

- **状态**：Accepted（2026-11-12，维护者决策）
- **背景**：0.2.0 前只有便携 zip（`scripts/build_portable.py` 产出 `dist/portable/LabSentinel/`
  经 `make_zip` 打包）+ 一个 Python 安装器（`scripts/install_portable.py`：装到
  LOCALAPPDATA、快捷方式、HKCU Uninstall、版本归档 + 健康检查 + 失败自动回滚）。
  用户要求「学习开源软件的打包方式，做成安装包」。
- **决策**：**同一 staging 产物同时出便携 zip 与 setup.exe 双形态**（mpv.net/Tautulli
  先例），安装器用 **Inno Setup 6**（`.iss` 脚本入库）；**保留** Python 安装器的
  版本归档/健康检查/自动回滚能力作为 `finalize` 子命令由安装器调用；
  Windows 服务（WinSW）做成安装器上的**可选勾选项**，默认不装。

## 调研结论（2026-11-12）

- **"flash s player next"**：未找到同名开源项目。覆盖排查：
  `debugly/fsplayer`（FSPlayer，ijkplayer 继任，无 Windows 版）、
  `AimesSoft/NipaPlay-Reload`（Flutter 播放器，Windows exe 安装器 + MSIX Store）、
  `ruffle-rs/ruffle`（Rust Flash 模拟器，nightly zip + Store）、
  `anilbeesetti/NextPlayer`（Android）。若用户本意指具体仓库，需提供链接再补。
- **同类「本地服务 + Web UI」开源项目的 Windows 打包**：
  | 项目 | 形态 | 安装器 | 关键实践 |
  |---|---|---|---|
  | Jellyfin Server | .NET 服务 + Web UI | NSIS | 「安装为服务」勾选（默认），NSSM 包装；托盘/服务二选一 |
  | Tautulli（Python+Web UI，与本形态最像） | Python 服务 | NSIS | 脚本安装器直接分发 Python runtime；快捷方式/静默参数 |
  | mpv.net | .NET 播放器 | Inno Setup | **同一产物出 setup.exe + 便携 zip 双形态**（本 ADR 直接样板） |
  | Navidrome | Go 单二进制 + Web UI | 无（zip） | 内置 `service install` 子命令或 NSSM/WinSW 手工装服务 |
  | Jenkins agent | Java 服务 | 安装器 + WinSW | 与现有 `labsentinel-service.xml` 同款模式 |
  | MPC-BE / SMPlayer | C++/Qt 播放器 | Inno Setup / NSIS | 常规快捷方式/文件关联/卸载 |
  | Immich / Paperless-ngx | 自托管服务 | 无原生 Windows 包 | 仅 Docker Compose（本项目已终止 Docker 路线） |
- **工具对比**（面向「Python runtime + C# 启动器 exe」组合）：
  - **Inno Setup**：维护活跃、Pascal/INI 脚本最易上手；`PrivilegesRequired=lowest`
    + `PrivilegesRequiredOverridesAllowed` 天然对齐按用户安装；`UsePreviousAppDir`
    自动覆盖升级对齐现有安装器语义；`SignTool=` 一次签好安装器与卸载器；
    无原生服务安装（用 `[Run]`/`[UninstallRun]` 调 `sc.exe` 或 WinSW，需管理员）。
  - **NSIS**：能力等价（SimpleSC 插件装服务），但 2024-25 社区普遍视其较老化
    （NVDA 迁移评估）。
  - **WiX/MSI**：企业 GPO/事务回滚场景才值；`cargo-wix` 低维护、MSI 组件规则与
    「胖 runtime 目录 + 自定义回滚」摩擦大；WiX bundle 有杀软误报前科。
  - **MSIX/Store**：Windows 服务需 restricted capability，不能走 App Installer，
    且工作目录不受控（uvicorn 依赖 cwd）——与形态冲突；仅未来 Store 分发再评估。
  - **cargo-packager / Tauri bundler**：默认 NSIS 模板无服务支持，sidecar 升级
    替换有已知 bug；主程序并非 Rust GUI，无增益。
  - **electron-builder**：Windows 默认 target = NSIS（oneClick + 可选 portable），
    佐证「脚本安装器为默认解」的跨生态共识。

## 设计要点

1. 安装位置 `{localappdata}\Programs\LabSentinel`（与 `install_portable.py` 一致）；
   数据仍留在 `{localappdata}\LabSentinel\`（卸载不删用户数据）。
2. `UsePreviousAppDir=yes`（同 AppId 覆盖升级）、`CloseApplications=yes`（处理占用
   中的 `LabSentinel.exe`）。
3. 升级归档/健康检查/自动回滚由安装器 `[Run]` 调
   `runtime\python.exe scripts\install_portable.py finalize --install-dir {app}`
   （`install_portable.py` 新增非交互子命令）复用既有实现，不重写。
4. 服务模式 = 自定义勾选页（默认不勾）；勾选后提权调
   `tools\winsw.exe install labsentinel-service.xml`（需随包带 winsw，或改为
   `sc.exe create`；本版先按 XML 说明保留手工/可选路径，不强制随包）。
5. 数字签名（T-403 人工事项）：证书到位后 `.iss` 启用 `SignTool=`，一次签齐
   setup.exe 与卸载器；未签名草稿期发布说明标注「未签名预览版」。
6. zip 便携形态继续保留（用户可直接解压双击 `LabSentinel.exe`）。

## 后果

- 正：用户有「双击 setup.exe 安装 → 开始菜单/卸载项」的标准软件体验；
  CI 一条命令可同时出双产物；现有回滚能力不丢。
- 负：CI 需装 Inno Setup（`choco install innosetup -y` 或便携 ISCC.EXE）；
  本机未装时 `--installer` 警告跳过（zip-only 仍成功）；服务勾选页需带
  winsw.exe 随包或明确"仅 RTSP/文件夹源"限制。
- 二进制接口不变：`smoke_portable.py`（zip 形态）8/8 仍是主冒烟门禁。
