; LabSentinel Windows 安装器（RA4.2 Rust 形态版，2026-11-14）
; 包装「Rust 单二进制 zip」为 setup.exe：解压安装 + 快捷方式 + 卸载项 + 可选服务。
; 由 ISCC.exe 编译（本地 Inno Setup 7；CI 由 release.yml 安装 Inno 7.1.1 后
; scripts/build_setup_exe.py 调用）；版本由 /DLsAppVersion= 传入。
;
; 多语言/双模式（沿用 2026-11-13 版设计）：
; - 首页语言选择：简体中文 / English（.isl 随仓库 deploy/windows/languages/）
; - 双安装模式（PrivilegesRequiredOverridesAllowed=dialog commandline）：
;   · 仅为我安装（默认，lowest，无需管理员）——目录 {localappdata}\Programs\LabSentinel
;   · 为所有用户安装（提权）——目录 {autopf}\LabSentinel
; - 快捷方式选项：「选择附加任务」页（开始菜单 + 桌面，默认全勾）
; - 升级：同 AppId 覆盖安装；升级前由安装器调 labsentinel-server --archive 归档旧版
;
; 与 Python 版差异：无 runtime\python.exe——归档/回滚直接用 labsentinel-server.exe
;   --archive / --rollback；健康检查用 --selftest-http。

#ifndef LsAppVersion
  #define LsAppVersion "0.8.5"
#endif
#ifndef LsSourceDir
  ; 指向解压后的 Rust 发行包目录（labsentinel-rust-<ver>-<date>.zip 解包结果）
  #define LsSourceDir "..\..\Resources\dist\rust\labsentinel-server"
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
DefaultDirName={code:GetDefaultDirName}
DefaultGroupName=LabSentinel
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog commandline
UsePreviousAppDir=yes
UsePreviousGroup=yes
DisableDirPage=no
DisableProgramGroupPage=yes
ShowLanguageDialog=yes
OutputDir={#LsOutputDir}
OutputBaseFilename=LabSentinel-setup-{#LsAppVersion}
SetupIconFile=..\..\engine-rs\crates\ls-server\app-icon.ico
Compression=lzma2/max
SolidCompression=yes
CloseApplications=yes
UninstallDisplayName=LabSentinel {#LsAppVersion}
UninstallDisplayIcon={app}\labsentinel-server.exe

[Languages]
Name: "chinese"; MessagesFile: "compiler:Default.isl,Languages\ChineseSimplified.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
chinese.StartMenu=创建开始菜单快捷方式
chinese.Desktop=创建桌面快捷方式
chinese.ShortcutOptGroup=快捷方式
chinese.Verify=正在校验新版本…
chinese.VerifyFailed=安装已完成，但新版本健康自检（--selftest-http）未通过（退出码 %1）。如属升级覆盖，可在安装目录执行 labsentinel-server.exe --rollback 回滚旧版。
chinese.ArchiveFailed=无法归档现有安装。请检查安装目录的写入权限与磁盘空间后重试。
chinese.Archive=正在归档旧版本…
chinese.Launch=启动 LabSentinel
chinese.RemoveUserData=是否同时删除 LabSentinel 的用户数据？%1 包含数据库、事件截图/录像、备份与密钥。选择「否」保留数据，重新安装后可继续使用。
english.StartMenu=Create a Start Menu shortcut
english.Desktop=Create a desktop shortcut
english.ShortcutOptGroup=Shortcuts
english.Verify=Verifying the new version…
english.VerifyFailed=Installation finished, but --selftest-http failed (exit code %1). If upgrading, you may run "labsentinel-server.exe --rollback" in the install directory.
english.ArchiveFailed=The existing installation could not be archived. Check disk space and write permissions before retrying.
english.Archive=Archiving the previous version…
english.Launch=Launch LabSentinel
english.RemoveUserData=Also delete LabSentinel user data? %1 contains the database, event snapshots/videos, backups and keys. Choose No to keep data for a future reinstall.

[Tasks]
; 快捷方式选项放在标准「选择附加任务」页（Inno 原生控件，随 DPI 自适应）；
; 不再在完成页手工注入复选框（会被绘制在 ReadyMemo/RunList 之上，文字重叠）。
Name: "createshortcut"; Description: "{cm:StartMenu}"; \
  GroupDescription: "{cm:ShortcutOptGroup}"
Name: "createdesktop"; Description: "{cm:Desktop}"; \
  GroupDescription: "{cm:ShortcutOptGroup}"

[Files]
; Rust 发行包全树（labsentinel-server.exe、LabSentinel.bat、VERSION、web、models、tools）
Source: "{#LsSourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[UninstallDelete]
; 0.3.1：卸载清理升级归档与旧布局遗留目录（此前 versions\<ver> 整树残留，
; 第三方卸载检查工具会报大量文件）。旧版 server 中的用户数据在 [Code] 保留后再清理。
Type: filesandordirs; Name: "{app}\versions"
Type: files; Name: "{app}\current.txt"
Type: files; Name: "{app}\.env"
; 用户数据（%LOCALAPPDATA%\LabSentinel）默认保留；卸载时可选择一并删除，
; 见 [Code] InitializeUninstall（静默卸载用 /REMOVEDATA=1 显式删除）。

[Run]
; 升级链路：
;   1) [Code] PrepareToInstall → 结束 labsentinel-server.exe，Job Object 回收其 go2rtc
;   2) [Code] PrepareToInstall → labsentinel-server --archive（归档旧版）
;   3) 文件复制（[Files]）
;   4) [Code] ssPostInstall → --selftest-http 校验退出码（失败在交互模式下弹窗提示 --rollback）
Filename: "{app}\labsentinel-server.exe"; \
  Parameters: "--launch --tray"; WorkingDir: "{app}"; \
  Description: "{cm:Launch}"; Flags: nowait postinstall skipifsilent

[Code]
// 双模式默认目录：管理员（为所有用户）→ {autopf}\LabSentinel；仅为我 → {localappdata}\Programs
function GetDefaultDirName(Param: String): String;
begin
  if IsAdminInstallMode then
    Result := ExpandConstant('{autopf}\LabSentinel')
  else
    Result := ExpandConstant('{localappdata}\Programs\LabSentinel');
end;

procedure StopInstalledServer;
var
  Locator, Wmi, Processes, Process: Variant;
  I, ExitCode: Integer;
  ExePath: String;
begin
  ExePath := ExpandConstant('{app}\labsentinel-server.exe');
  try
    Locator := CreateOleObject('WbemScripting.SWbemLocator');
    Wmi := Locator.ConnectServer('.', 'root\CIMV2');
    Processes := Wmi.ExecQuery('SELECT ProcessId, ExecutablePath FROM Win32_Process WHERE Name="labsentinel-server.exe"');
    for I := 0 to Processes.Count - 1 do
    begin
      Process := Processes.ItemIndex(I);
      if not VarIsNull(Process.ExecutablePath) then
        if CompareText(Process.ExecutablePath, ExePath) = 0 then
          Exec(ExpandConstant('{sys}\taskkill.exe'),
               '/F /T /PID ' + IntToStr(Process.ProcessId), '', SW_HIDE,
               ewWaitUntilTerminated, ExitCode);
    end;
  except
    Log('Unable to enumerate running LabSentinel processes: ' + GetExceptionMessage);
  end;
end;

// ---- 升级/覆盖安装前自动停服（防止 labsentinel-server.exe / go2rtc.exe 占用文件）----
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  Result := '';
  StopInstalledServer;
  if FileExists(ExpandConstant('{app}\labsentinel-server.exe')) then
  begin
    if (not Exec(ExpandConstant('{app}\labsentinel-server.exe'), '--archive',
                 ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ResultCode))
      or (ResultCode <> 0) then
      Result := CustomMessage('ArchiveFailed');
  end;
end;

// ---- 快捷方式生成：严格按「选择附加任务」页勾选 ----
// 静默安装（/VERYSILENT）时任务按默认选中处理（[Tasks] 未标 unchecked）。
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
  WantMenu := WizardIsTaskSelected('createshortcut');
  WantDesktop := WizardIsTaskSelected('createdesktop');
  // UX3：快捷方式直指 exe（--launch --tray）——无黑框；bat 保留一个版本周期兼容
  if WantMenu then
  begin
    ForceDirectories(GroupDir);
    CreateShellLink(GroupDir + '\Uninstall LabSentinel.lnk',
      'Uninstall LabSentinel', ExpandConstant('{uninstallexe}'), '', '', '', 0, SW_SHOWNORMAL);
    CreateShellLink(GroupDir + '\LabSentinel.lnk',
      CustomMessage('Launch'), ExpandConstant('{app}\labsentinel-server.exe'),
      '--launch --tray', ExpandConstant('{app}'), '', 0, SW_SHOWNORMAL);
  end;
  if WantDesktop then
    CreateShellLink(DesktopDir + '\LabSentinel.lnk',
      CustomMessage('Launch'), ExpandConstant('{app}\labsentinel-server.exe'),
      '--launch --tray', ExpandConstant('{app}'), '', 0, SW_SHOWNORMAL);
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
begin
  if CurStep = ssPostInstall then
  begin
    CreateShortcuts;

    // LS-045：真实启动新版本自检（--selftest-http），校验退出码并在交互模式下明确提示
    if FileExists(ExpandConstant('{app}\labsentinel-server.exe')) then
    begin
      if (not Exec(ExpandConstant('{app}\labsentinel-server.exe'),
                   '--selftest-http', ExpandConstant('{app}'), SW_HIDE,
                   ewWaitUntilTerminated, ResultCode)) or (ResultCode <> 0) then
      begin
        if not WizardSilent then
          MsgBox(FmtMessage(CustomMessage('VerifyFailed'), [IntToStr(ResultCode)]),
                 mbError, MB_OK);
      end;
    end;
  end;
end;

// 用户数据默认保留。静默卸载用 /REMOVEDATA=1 显式删除。
var
  RemoveUserData: Boolean;

function InitializeUninstall(): Boolean;
var
  DataDir: String;
begin
  Result := True;
  DataDir := ExpandConstant('{localappdata}\LabSentinel') + #13#10 +
             ExpandConstant('{app}\data');
  if UninstallSilent then
    RemoveUserData := ExpandConstant('{param:REMOVEDATA|0}') = '1'
  else
    RemoveUserData := MsgBox(
      FmtMessage(CustomMessage('RemoveUserData'), [DataDir]),
      mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
end;

procedure CleanAutostart;
var
  RunKey, Value, ExePath: String;
begin
  RunKey := 'Software\Microsoft\Windows\CurrentVersion\Run';
  ExePath := '"' + ExpandConstant('{app}\labsentinel-server.exe') + '"';
  if RegQueryStringValue(HKCU, RunKey, 'LabSentinel', Value) and
     ((CompareText(Value, ExePath) = 0) or
      (CompareText(Copy(Value, 1, Length(ExePath) + 1), ExePath + ' ') = 0)) then
    RegDeleteValue(HKCU, RunKey, 'LabSentinel');
end;

procedure CleanShortcut(const Link: String);
var
  Shell, Shortcut: Variant;
  Target: String;
begin
  if not FileExists(Link) then
    Exit;
  try
    Shell := CreateOleObject('WScript.Shell');
    Shortcut := Shell.CreateShortcut(Link);
    Target := Shortcut.TargetPath;
    if (CompareText(Target, ExpandConstant('{app}\labsentinel-server.exe')) = 0) or
       (CompareText(Target, ExpandConstant('{app}\LabSentinel.bat')) = 0) or
       (CompareText(Target, ExpandConstant('{uninstallexe}')) = 0) then
      DeleteFile(Link);
  except
    Log('Unable to inspect LabSentinel shortcut: ' + Link);
  end;
end;

procedure CleanShortcutsIn(const GroupDir, DesktopDir: String);
begin
  CleanShortcut(GroupDir + '\LabSentinel.lnk');
  CleanShortcut(GroupDir + '\Uninstall LabSentinel.lnk');
  CleanShortcut(DesktopDir + '\LabSentinel.lnk');
  RemoveDir(GroupDir);
end;

procedure CleanLegacyServer;
var
  Previous, Preserved, DataDir: String;
  Suffix: Integer;
begin
  Previous := ExpandConstant('{app}\server\data');
  if not RemoveUserData and DirExists(Previous) then
  begin
    DataDir := ExpandConstant('{app}\data');
    Preserved := DataDir + '\legacy-server-data';
    Suffix := 0;
    while FileExists(Preserved) or DirExists(Preserved) do
    begin
      Suffix := Suffix + 1;
      Preserved := DataDir + '\legacy-server-data-' + IntToStr(Suffix);
    end;
    if not ForceDirectories(DataDir) or not RenameFile(Previous, Preserved) then
    begin
      Log('Preserving legacy LabSentinel data at ' + Previous);
      Exit;
    end;
  end;
  DelTree(ExpandConstant('{app}\server'), True, True, True);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    StopInstalledServer;
    CleanAutostart;
    CleanShortcutsIn(ExpandConstant('{userprograms}\LabSentinel'), ExpandConstant('{userdesktop}'));
    if IsAdminInstallMode then
      CleanShortcutsIn(ExpandConstant('{commonprograms}\LabSentinel'), ExpandConstant('{commondesktop}'));
  end;
  if CurUninstallStep = usPostUninstall then
  begin
    CleanLegacyServer;
    if RemoveUserData then
    begin
      DelTree(ExpandConstant('{localappdata}\LabSentinel'), True, True, True);
      DelTree(ExpandConstant('{app}\data'), True, True, True);
    end;
    RemoveDir(ExpandConstant('{app}'));
  end;
end;
