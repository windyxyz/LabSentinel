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
  #define LsAppVersion "0.5.0"
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
Compression=lzma2/max
SolidCompression=yes
CloseApplications=yes
UninstallDisplayName=LabSentinel {#LsAppVersion}
UninstallDisplayIcon={app}\labsentinel-server.exe

[Languages]
Name: "chinese"; MessagesFile: "compiler:Default.isl,Languages\ChineseSimplified.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
chinese.ServiceTask=安装为 Windows 服务（仅 RTSP/文件夹源；需管理员）
chinese.ServiceOptGroup=可选组件
chinese.ServiceMissing=未随包携带 winsw.exe：服务模式需手工安装（参见 deploy/windows/labsentinel-service.xml）。
chinese.ServiceFailed=服务安装失败（WinSW 执行失败）。可在安装目录手工运行 winsw install。
chinese.StartMenu=创建开始菜单快捷方式
chinese.Desktop=创建桌面快捷方式
chinese.ShortcutOptGroup=快捷方式
chinese.Verify=正在校验新版本…
chinese.VerifyFailed=安装已完成，但新版本健康自检（--selftest-http）未通过（退出码 %1）。如属升级覆盖，可在安装目录执行 labsentinel-server.exe --rollback 回滚旧版。
chinese.Archive=正在归档旧版本…
chinese.Launch=启动 LabSentinel
chinese.RemoveUserData=是否同时删除 LabSentinel 的用户数据？%1 包含数据库、事件截图/录像、备份与密钥。选择「否」保留数据，重新安装后可继续使用。
english.ServiceTask=Install as a Windows service (RTSP/folder sources only; requires admin)
english.ServiceOptGroup=Optional components
english.ServiceMissing=winsw.exe is not bundled: install the service manually (see deploy/windows/labsentinel-service.xml).
english.ServiceFailed=Service installation failed (WinSW error). Run "winsw install" manually in the install directory.
english.StartMenu=Create a Start Menu shortcut
english.Desktop=Create a desktop shortcut
english.ShortcutOptGroup=Shortcuts
english.Verify=Verifying the new version…
english.VerifyFailed=Installation finished, but --selftest-http failed (exit code %1). If upgrading, you may run "labsentinel-server.exe --rollback" in the install directory.
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
Name: "installservice"; Description: "{cm:ServiceTask}"; \
  GroupDescription: "{cm:ServiceOptGroup}"; Flags: unchecked;

[Files]
; Rust 发行包全树（labsentinel-server.exe、LabSentinel.bat、VERSION、web、models、tools）
Source: "{#LsSourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[UninstallDelete]
; CreateShellLink 生成的快捷方式不在 Inno 安装日志里，卸载需显式删除；
; {group} 随安装模式（仅我/所有用户）自动指向对应用户或公共开始菜单。
Type: files; Name: "{group}\LabSentinel.lnk"
Type: files; Name: "{group}\Uninstall LabSentinel.lnk"
Type: dirifempty; Name: "{group}"
Type: files; Name: "{userdesktop}\LabSentinel.lnk"
Type: files; Name: "{commondesktop}\LabSentinel.lnk"
; 0.3.1：卸载清理升级归档与旧布局遗留目录（此前 versions\<ver> 整树残留，
; 第三方卸载检查工具会报大量文件；server/data 是修复前的相对路径残留）。
Type: filesandordirs; Name: "{app}\versions"
Type: filesandordirs; Name: "{app}\server"
Type: filesandordirs; Name: "{app}\data"
Type: files; Name: "{app}\current.txt"
Type: files; Name: "{app}\.env"
; 用户数据（%LOCALAPPDATA%\LabSentinel）默认保留；卸载时可选择一并删除，
; 见 [Code] InitializeUninstall（静默卸载用 /REMOVEDATA=1 显式删除）。

[Run]
; 升级链路：
;   1) [Code] PrepareToInstall → 结束正在运行的 labsentinel-server.exe / go2rtc.exe
;   2) [Code] ssInstall → labsentinel-server --archive（归档旧版，见 CurStepChanged）
;   3) 文件复制（[Files]）
;   4) [Code] ssPostInstall → --selftest-http 校验退出码（失败在交互模式下弹窗提示 --rollback）
Filename: "{app}\labsentinel-server.exe"; \
  Parameters: "--launch --tray"; WorkingDir: "{app}"; \
  Description: "{cm:Launch}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; 卸载前必须结束正在运行的 labsentinel-server.exe（无参数会常驻起服务，
; 直接 Exec 会挂住卸载器且文件被锁）。taskkill /F：进程不存在返回 1，Inno 忽略。
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM labsentinel-server.exe"; \
  Flags: runhidden; RunOnceId: "StopLabSentinel"
; LS-053：清理网页端开启的 HKCU 开机自启（免管理员写入的 Run 值），保证卸载零残留
Filename: "{sys}\reg.exe"; \
  Parameters: "delete ""HKCU\Software\Microsoft\Windows\CurrentVersion\Run"" /v LabSentinel /f"; \
  Flags: runhidden; RunOnceId: "CleanAutostart"

[Code]
// 双模式默认目录：管理员（为所有用户）→ {autopf}\LabSentinel；仅为我 → {localappdata}\Programs
function GetDefaultDirName(Param: String): String;
begin
  if IsAdminInstallMode then
    Result := ExpandConstant('{autopf}\LabSentinel')
  else
    Result := ExpandConstant('{localappdata}\Programs\LabSentinel');
end;

// ---- 升级/覆盖安装前自动停服（防止 labsentinel-server.exe / go2rtc.exe 占用文件）----
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  Result := '';
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM labsentinel-server.exe',
       '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM go2rtc.exe',
       '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
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
  WinSW: String;
begin
  // 升级归档：文件复制前归档当前旧版到 versions/<ver>/（回滚点）。
  // Rust 形态直接用 labsentinel-server --archive（--rollback 可手工恢复）。
  if CurStep = ssInstall then
  begin
    if FileExists(ExpandConstant('{app}\labsentinel-server.exe')) then
    begin
      Exec(ExpandConstant('{app}\labsentinel-server.exe'),
           '--archive', ExpandConstant('{app}'), SW_HIDE,
           ewWaitUntilTerminated, ResultCode);
    end;
  end;

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

    // 可选服务（Tasks 勾选后；仅 ALLUSERS 模式有意义）
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

// ---- 卸载优化（0.3.1）----
// 1) versions/ 升级归档、旧布局 server/、data/ 由 [UninstallDelete] 清理；
// 2) 用户数据（%LOCALAPPDATA%\LabSentinel）默认保留，交互卸载时询问是否一并删除
//    （静默卸载用 /REMOVEDATA=1 显式删除）；
// 3) 尽力清理 Prefetch 缓存（系统托管；仅管理员卸载时可删，失败不阻断）。
var
  RemoveUserData: Boolean;

function InitializeUninstall(): Boolean;
var
  DataDir: String;
begin
  Result := True;
  DataDir := ExpandConstant('{localappdata}\LabSentinel');
  if UninstallSilent then
    RemoveUserData := ExpandConstant('{param:REMOVEDATA|0}') = '1'
  else
    RemoveUserData := MsgBox(
      FmtMessage(CustomMessage('RemoveUserData'), [DataDir]),
      mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
end;

procedure DeletePrefetchMask(const Mask: String);
var
  FindRec: TFindRec;
  Dir: String;
begin
  Dir := ExpandConstant('{win}\Prefetch');
  try
    if FindFirst(Dir + '\' + Mask, FindRec) then
    begin
      try
        repeat
          DeleteFile(Dir + '\' + FindRec.Name);
        until not FindNext(FindRec);
      finally
        FindClose(FindRec);
      end;
    end;
  except
    // Prefetch 由系统维护，清理失败忽略
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    // 旧布局启动器锁文件（数据目录上一级 logs/）
    DeleteFile(ExpandConstant('{localappdata}\LabSentinel\logs\launcher.lock'));
    if RemoveUserData then
      DelTree(ExpandConstant('{localappdata}\LabSentinel'), True, True, True);
    DeletePrefetchMask('LABSENTINEL-SERVER.EXE-*.pf');
    DeletePrefetchMask('GO2RTC.EXE-*.pf');
    DeletePrefetchMask('FFMPEG.EXE-*.pf');
  end;
end;
