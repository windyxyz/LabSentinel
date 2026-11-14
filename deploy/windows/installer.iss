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
; - 完成页双复选框：开始菜单 + 桌面快捷方式（默认全勾）
; - 升级：同 AppId 覆盖安装；升级前由安装器调 labsentinel-server --archive 归档旧版
;
; 与 Python 版差异：无 runtime\python.exe——归档/回滚直接用 labsentinel-server.exe
;   --archive / --rollback；健康检查用 --selftest-http。

#ifndef LsAppVersion
  #define LsAppVersion "0.2.0"
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
chinese.Verify=正在校验新版本…
chinese.Archive=正在归档旧版本…
chinese.Launch=启动 LabSentinel
english.ServiceTask=Install as a Windows service (RTSP/folder sources only; requires admin)
english.ServiceOptGroup=Optional components
english.ServiceMissing=winsw.exe is not bundled: install the service manually (see deploy/windows/labsentinel-service.xml).
english.ServiceFailed=Service installation failed (WinSW error). Run "winsw install" manually in the install directory.
english.StartMenu=Create a Start Menu shortcut
english.Desktop=Create a desktop shortcut
english.Verify=Verifying the new version…
english.Archive=Archiving the previous version…
english.Launch=Launch LabSentinel

[Tasks]
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
; 用户数据（%LOCALAPPDATA%\LabSentinel）与 versions/ 归档不随卸载删除；
; 需清理时手工删除：数据 %LOCALAPPDATA%\LabSentinel\，归档 {app}\versions\

[Run]
; 升级链路：
;   1) [Code] ssInstall → labsentinel-server --archive（归档旧版，见 CurStepChanged）
;   2) 文件复制（[Files]）
;   3) 此处 --selftest-http：真实启动新版本 → /api/health/ready → 停止；
;      失败可用 --rollback 恢复（安装器不自动回滚，提示用户手工执行）。
Filename: "{app}\labsentinel-server.exe"; \
  Parameters: "--selftest-http"; WorkingDir: "{app}"; Flags: runhidden; \
  StatusMsg: "{cm:Verify}"
Filename: "{app}\LabSentinel.bat"; Description: "{cm:Launch}"; \
  Flags: nowait postinstall skipifsilent

[UninstallRun]
; 卸载前必须结束正在运行的 labsentinel-server.exe（无参数会常驻起服务，
; 直接 Exec 会挂住卸载器且文件被锁）。taskkill /F：进程不存在返回 1，Inno 忽略。
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM labsentinel-server.exe"; \
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

// ---- 完成页注入双复选框（默认全勾）----
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
    StartMenuCb.Top := WizardForm.RunList.Top - StartMenuCb.Height - ScaleY(28);
    DesktopCb.Top := StartMenuCb.Top + StartMenuCb.Height + ScaleY(6);
    StartMenuCb.Visible := True;
    DesktopCb.Visible := True;
  end;
end;

// ---- 快捷方式生成：按完成页勾选；卸载项始终生成 ----
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
  WantMenu := (StartMenuCb = nil) or StartMenuCb.Checked;
  WantDesktop := (DesktopCb = nil) or DesktopCb.Checked;
  ForceDirectories(GroupDir);
  CreateShellLink(GroupDir + '\Uninstall LabSentinel.lnk',
    'Uninstall LabSentinel', ExpandConstant('{uninstallexe}'), '', '', '', 0, SW_SHOWNORMAL);
  if WantMenu then
    CreateShellLink(GroupDir + '\LabSentinel.lnk',
      CustomMessage('Launch'), ExpandConstant('{app}\LabSentinel.bat'),
      '', ExpandConstant('{app}'), '', 0, SW_SHOWNORMAL);
  if WantDesktop then
    CreateShellLink(DesktopDir + '\LabSentinel.lnk',
      CustomMessage('Launch'), ExpandConstant('{app}\LabSentinel.bat'),
      '', ExpandConstant('{app}'), '', 0, SW_SHOWNORMAL);
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
