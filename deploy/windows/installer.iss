; LabSentinel Windows 安装器（ADR-0010 / 批次 B，2026-11-12；多语言+双模式版 2026-11-13）
; 同一 staging（Resources/dist/portable/LabSentinel/）出 setup.exe + 便携 zip 双形态。
; 由 scripts/build_portable.py --installer 调用 ISCC.exe 编译；版本由 /DLsAppVersion= 传入。
; 参考样板：mpv.net（Inno + 同源 zip）、Tautulli（Python 服务 NSIS）、Jellyfin（可选服务）。
;
; 多语言/双模式/交互（2026-11-13）：
; - 首页语言选择：简体中文 / English（IS7 自带 Languages\ChineseSimplified.isl）
; - 双安装模式（PrivilegesRequiredOverridesAllowed=dialog commandline）：
;   · 仅为我安装（默认，lowest，无需管理员）——目录 {localappdata}\Programs\LabSentinel，
;     开始菜单 {userprograms}，桌面 {userdesktop}，卸载项 HKCU
;   · 为所有用户安装（向导可选/命令行 /ALLUSERS，提权）——目录 {autopf}\LabSentinel，
;     开始菜单 {commonprograms}（所有用户可见），桌面 {commondesktop}，卸载项 HKLM
;   Inno 的 {userprograms}/{commonprograms} 等常量随 IsAdminInstallMode 自动切换，
;   [Registry] 用 HKLM/HKCU 分支 + [Code] AdminInstallMode 判定；数据目录两种模式
;   都落在各用户 %LOCALAPPDATA%\LabSentinel（服务模式例外，见 XML）。
; - 完成页双复选框：开始菜单 + 桌面快捷方式（默认全勾）；「立即启动」沿用内建 RunList
; - 快捷方式生成在 ssPostInstall（CreateShellLink），卸载项始终生成
;
; 注意：为所有用户安装时数据目录仍是安装用户的 LOCALAPPDATA——多用户各自跑
; `runtime\python.exe scripts\install_portable.py finalize` 会生成各自数据；
; 服务模式（WinSW）才是共享数据形态（见 deploy/windows/labsentinel-service.xml）。

#ifndef LsAppVersion
  #define LsAppVersion "0.2.0"
#endif
#ifndef LsSourceDir
  #define LsSourceDir "..\..\Resources\dist\portable\LabSentinel"
#endif
#ifndef LsOutputDir
  #define LsOutputDir "..\..\Resources\dist"
#endif

[Setup]
AppId={{9C7B3A41-5E2D-4F86-A1C9-2B7D6E4F8A10}
AppName=LabSentinel
AppVersion={#LsAppVersion}
AppVerName=LabSentinel {#LsAppVersion}
AppPublisher=LabSentinel Project
AppPublisherURL=https://github.com/windyxyz/LabSentinel
; 双模式默认目录：IsAdminInstallMode 为假 → {localappdata}\Programs，真 → {autopf}
DefaultDirName={code:GetDefaultDirName}
DefaultGroupName=LabSentinel
; 向导「安装类型」页提供 仅为我 / 为所有用户 选择；命令行 /ALLUSERS /CURRENTUSER 亦可
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog commandline
; 同 AppId 覆盖升级：自动装回原目录并原地覆盖文件
UsePreviousAppDir=yes
UsePreviousGroup=yes
; 安装位置页：允许用户自选目录（升级沿用 UsePreviousAppDir）
DisableDirPage=no
DisableProgramGroupPage=yes
; 语言选择页置于最前（auto=跟随系统语言预选，可手动改）
ShowLanguageDialog=yes
OutputDir={#LsOutputDir}
OutputBaseFilename=LabSentinel-setup-{#LsAppVersion}
Compression=lzma2/max
SolidCompression=yes
CloseApplications=yes
UninstallDisplayName=LabSentinel {#LsAppVersion}
UninstallDisplayIcon={app}\LabSentinel.exe
; 签名（T-403 证书到位后启用）：
; SignTool=signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /n "LabSentinel Project" $f
; SignedUninstaller=yes
; 卸载不删用户数据（数据在 %LOCALAPPDATA%\LabSentinel，独立于 {app}）

[Languages]
Name: "chinese"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
; 双语自定义文案（key 相同，{cm:xxx} 按 ActiveLanguage 取值）
chinese.ServiceTask=安装为 Windows 服务（仅 RTSP/文件夹源；需管理员）
chinese.ServiceOptGroup=可选组件
chinese.ServiceMissing=未随包携带 winsw.exe：服务模式需手工安装（参见 deploy/windows/labsentinel-service.xml）。
chinese.ServiceFailed=服务安装失败（WinSW 执行失败）。可在安装目录手工运行 winsw install。
chinese.StartMenu=创建开始菜单快捷方式
chinese.Desktop=创建桌面快捷方式
chinese.Verify=正在校验新版本…
chinese.Launch=启动 LabSentinel
english.ServiceTask=Install as a Windows service (RTSP/folder sources only; requires admin)
english.ServiceOptGroup=Optional components
english.ServiceMissing=winsw.exe is not bundled: install the service manually (see deploy/windows/labsentinel-service.xml).
english.ServiceFailed=Service installation failed (WinSW error). Run "winsw install" manually in the install directory.
english.StartMenu=Create a Start Menu shortcut
english.Desktop=Create a desktop shortcut
english.Verify=Verifying the new version…
english.Launch=Launch LabSentinel

[Tasks]
; 唯一保留的 Tasks 页项：可选服务（默认不勾）
Name: "installservice"; Description: "{cm:ServiceTask}"; \
  GroupDescription: "{cm:ServiceOptGroup}"; Flags: unchecked;

[Files]
; staging 全树（含 runtime、app、web、models、tools、scripts、LICENSE/NOTICE）
Source: "{#LsSourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Registry]
; 控制面板「卸载程序」条目由 Inno 原生维护（{AppId}_is1 键，随安装模式自动落
; HKCU / HKLM(32 位视图)，DisplayVersion/Publisher/InstallLocation 齐全）——
; 2026-11-13 ALLUSERS 实测确认后移除旧的手写补充键（曾造成控制面板双条目）。
; 数据目录说明见 [UninstallDelete]。

[UninstallDelete]
; 用户数据（%LOCALAPPDATA%\LabSentinel）与 versions/ 归档**不随卸载删除**——
; {app} 之外的目录本就不在卸载范围，此处仅作说明；需清理时手工删除：
;   数据：%LOCALAPPDATA%\LabSentinel\（DB/截图/视频/标注池/日志/FIRST_RUN_TOKEN）
;   版本归档：{app}\versions\（回滚点，卸载时保留便于排查后重装）
;   服务模式共享数据：C:\ProgramData\LabSentinel\（WinSW XML 的 LS_DATA_DIR）
; Type: files; Name: "{app}\versions\*"  ← 刻意不写：保留回滚点

[Run]
; 升级链路（T-404/ADR-0010）：
;   1) [Code] ssInstall → finalize --archive（归档旧版，见 CurStepChanged）
;   2) 文件复制（[Files]）
;   3) 此处 finalize --verify：真实启动新版本 uvicorn → /api/health/ready → 停止；
;      失败自动 rollback() 到 versions/ 归档并以非 0 退出码让 Inno 中止安装。
;   服务模式（ALLUSERS）下健康检查以当前管理员用户身份跑（数据目录为其 LOCALAPPDATA），
;   与服务实际运行账户（LocalService）不同——仅验证程序本身可启动。
Filename: "{app}\runtime\python.exe"; \
  Parameters: "scripts\install_portable.py finalize --verify --install-dir ""{app}"" --silent"; \
  WorkingDir: "{app}"; Flags: runhidden; StatusMsg: "{cm:Verify}"
Filename: "{app}\LabSentinel.exe"; Description: "{cm:Launch}"; \
  Flags: nowait postinstall skipifsilent

[UninstallRun]
; 卸载前先停服务（仅当装过服务时；WinSW XML 随包，服务名见 labsentinel-service.xml）
Filename: "{app}\LabSentinel.exe"; Parameters: "--stop"; WorkingDir: "{app}"; \
  Flags: runhidden; RunOnceId: "StopLabSentinel"

[Code]
var
  StartMenuCb: TNewCheckBox;
  DesktopCb: TNewCheckBox;

// 双模式默认目录：管理员（为所有用户）→ {autopf}\LabSentinel；仅为我 → {localappdata}\Programs
function GetDefaultDirName(Param: String): String;
begin
  if IsAdminInstallMode then
    Result := ExpandConstant('{autopf}\LabSentinel')
  else
    Result := ExpandConstant('{localappdata}\Programs\LabSentinel');
end;

// ---- 完成页注入双复选框（默认全勾）：开始菜单 + 桌面快捷方式 ----
procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = wpFinished then
  begin
    if StartMenuCb = nil then
    begin
      StartMenuCb := TNewCheckBox.Create(WizardForm);
      StartMenuCb.Parent := WizardForm;
      StartMenuCb.Left := WizardForm.RunList.Left;
      StartMenuCb.Width := WizardForm.RunList.Width;
      StartMenuCb.Caption := CustomMessage('StartMenu');
      StartMenuCb.Checked := True;

      DesktopCb := TNewCheckBox.Create(WizardForm);
      DesktopCb.Parent := WizardForm;
      DesktopCb.Left := WizardForm.RunList.Left;
      DesktopCb.Width := WizardForm.RunList.Width;
      DesktopCb.Caption := CustomMessage('Desktop');
      DesktopCb.Checked := True;
    end;
    // 垂直排布：两个复选框位于 RunList 上方
    StartMenuCb.Top := WizardForm.RunList.Top - StartMenuCb.Height - ScaleY(28);
    DesktopCb.Top := StartMenuCb.Top + StartMenuCb.Height + ScaleY(6);
    StartMenuCb.Visible := True;
    DesktopCb.Visible := True;
  end;
end;

// ---- 快捷方式生成（文件复制后）：按完成页勾选；卸载项始终生成 ----
// 开始菜单位置随模式：管理员 → {commonprograms}（所有用户开始菜单）；
// 仅为我 → {userprograms}。桌面同理 {commondesktop}/{userdesktop}。
procedure CreateShortcuts;
var
  GroupDir, DesktopDir: String;
  WantMenu, WantDesktop: Boolean;
begin
  if IsAdminInstallMode then
  begin
    GroupDir := ExpandConstant('{commonprograms}\LabSentinel');
    DesktopDir := ExpandConstant('{commondesktop}');
  end
  else
  begin
    GroupDir := ExpandConstant('{userprograms}\LabSentinel');
    DesktopDir := ExpandConstant('{userdesktop}');
  end;
  // 静默安装（/VERYSILENT）不触发 wpFinished → 复选框为 nil → 默认全生成
  // （交互安装下复选框默认也是全勾，语义一致）
  WantMenu := (StartMenuCb = nil) or StartMenuCb.Checked;
  WantDesktop := (DesktopCb = nil) or DesktopCb.Checked;
  // 卸载快捷方式始终生成（保证可从开始菜单卸载）
  ForceDirectories(GroupDir);
  CreateShellLink(GroupDir + '\Uninstall LabSentinel.lnk',
    'Uninstall LabSentinel', ExpandConstant('{uninstallexe}'), '', '', '', 0, SW_SHOWNORMAL);
  if WantMenu then
    CreateShellLink(GroupDir + '\LabSentinel.lnk',
      CustomMessage('Launch'), ExpandConstant('{app}\LabSentinel.exe'),
      '', ExpandConstant('{app}'), '', 0, SW_SHOWNORMAL);
  if WantDesktop then
    CreateShellLink(DesktopDir + '\LabSentinel.lnk',
      CustomMessage('Launch'), ExpandConstant('{app}\LabSentinel.exe'),
      '', ExpandConstant('{app}'), '', 0, SW_SHOWNORMAL);
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
  WinSW: String;
begin
  // 升级归档：文件复制**前**（ssInstall）归档当前旧版到 versions/<ver>/（T-404 回滚点）
  if CurStep = ssInstall then
  begin
    // 旧版归档（首次安装无旧版时 install_portable 内部跳过）；失败不阻断安装，
    // 仅失去回滚点（与 do_upgrade 的 T-404 语义一致）
    Exec(ExpandConstant('{app}\runtime\python.exe'),
         'scripts\install_portable.py finalize --archive --install-dir "{app}"',
         ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ResultCode);
  end;

  if CurStep = ssPostInstall then
  begin
    CreateShortcuts;

    // 可选服务（Tasks 勾选后；仅 ALLUSERS 模式有意义——LocalService + ProgramData）
    if WizardIsTaskSelected('installservice') then
    begin
      if not IsAdminInstallMode then
        MsgBox(CustomMessage('ServiceMissing'), mbInformation, MB_OK)
      else
      begin
        WinSW := ExpandConstant('{app}\tools\winsw.exe');
        if FileExists(WinSW) then
        begin
          if not Exec(WinSW, 'install labsentinel-service.xml',
                      ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ResultCode) then
            MsgBox(CustomMessage('ServiceFailed'), mbError, MB_OK);
        end
        else
          MsgBox(CustomMessage('ServiceMissing'), mbInformation, MB_OK);
      end;
    end;
  end;
end;
