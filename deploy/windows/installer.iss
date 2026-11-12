; LabSentinel Windows 安装器（ADR-0010 / 批次 B，2026-11-12）
; 同一 staging（Resources/dist/portable/LabSentinel/）出 setup.exe + 便携 zip 双形态。
; 由 scripts/build_portable.py --installer 调用 ISCC.exe 编译；版本由 /DLsAppVersion= 传入。
; 参考样板：mpv.net（Inno + 同源 zip）、Tautulli（Python 服务 NSIS）、Jellyfin（可选服务）。

#ifndef LsAppVersion
  #define LsAppVersion "0.2.0"
#endif
#ifndef LsSourceDir
  #define LsSourceDir "..\\..\\Resources\\dist\\portable\\LabSentinel"
#endif
#ifndef LsOutputDir
  #define LsOutputDir "..\\..\\Resources\\dist"
#endif

[Setup]
AppId={{9C7B3A41-5E2D-4F86-A1C9-2B7D6E4F8A10}
AppName=LabSentinel
AppVersion={#LsAppVersion}
AppVerName=LabSentinel {#LsAppVersion}
AppPublisher=LabSentinel Project
AppPublisherURL=https://github.com/Windy-LabSentinel/LabSentinel
DefaultDirName={localappdata}\Programs\LabSentinel
DefaultGroupName=LabSentinel
; 按用户安装（与 install_portable.py 的 HKCU 设计一致），允许命令行 /ALLUSERS 提权
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog commandline
; 同 AppId 覆盖升级：自动装回原目录并原地覆盖文件
UsePreviousAppDir=yes
UsePreviousGroup=yes
DisableProgramGroupPage=yes
DisableDirPage=auto
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
Name: "chinesesimp"; MessagesFile: "compiler:Default.isl"
; 中文语言包若不可用，回退英文（ISCC 的 Default.isl 即英文）

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "快捷方式:";
Name: "installservice"; Description: "安装为 Windows 服务（仅 RTSP/文件夹源；需管理员）"; \
  GroupDescription: "可选:"; Flags: unchecked;

[Files]
; staging 全树（含 runtime、app、web、models、tools、scripts、LICENSE/NOTICE）
Source: "{#LsSourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\LabSentinel"; Filename: "{app}\LabSentinel.exe"; WorkingDir: "{app}"
Name: "{group}\卸载 LabSentinel"; Filename: "{uninstallexe}"
Name: "{userdesktop}\LabSentinel"; Filename: "{app}\LabSentinel.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Registry]
; 控制面板「卸载程序」条目（HKCU，无需管理员）；Inno 自带卸载注册表键，此处只补显示信息
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Uninstall\LabSentinel"; \
  ValueType: string; ValueName: "DisplayName"; ValueData: "LabSentinel {#LsAppVersion}"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Uninstall\LabSentinel"; \
  ValueType: string; ValueName: "DisplayVersion"; ValueData: "{#LsAppVersion}"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Uninstall\LabSentinel"; \
  ValueType: string; ValueName: "Publisher"; ValueData: "LabSentinel Project"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Uninstall\LabSentinel"; \
  ValueType: string; ValueName: "InstallLocation"; ValueData: "{app}"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Uninstall\LabSentinel"; \
  ValueType: string; ValueName: "HelpLink"; ValueData: "docs\部署FAQ.md"; Flags: uninsdeletekey

[Run]
; 升级健康检查（R2-402 复用）：真实启动新版本 uvicorn → /api/health/ready → 停止；
; 失败 NonZero 退出码 → Inno 中止安装并回滚。skipifsilent 让静默 CI 安装跳过交互自检。
Filename: "{app}\runtime\python.exe"; \
  Parameters: "scripts\install_portable.py finalize --install-dir ""{app}"" --silent"; \
  WorkingDir: "{app}"; Flags: runhidden; StatusMsg: "正在校验新版本…"; \
  Check: not WizardSilent
Filename: "{app}\LabSentinel.exe"; Description: "立即启动 LabSentinel"; \
  Flags: nowait postinstall skipifsilent unchecked

[UninstallRun]
; 卸载前先停服务（仅当装过服务时；WinSW XML 随包，服务名见 labsentinel-service.xml）
Filename: "{app}\LabSentinel.exe"; Parameters: "--stop"; WorkingDir: "{app}"; \
  Flags: runhidden; RunOnceId: "StopLabSentinel"

[Code]
// 可选服务安装：勾选后提权调用 WinSW（若随包带 winsw.exe）；否则给出手工指引。
// 默认不勾选——Session 0 下 USB 摄像头不可用，服务模式仅建议 RTSP/文件夹源
// （见 deploy/windows/labsentinel-service.xml 头部说明）。
procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
  WinSW: String;
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('installservice') then
  begin
    WinSW := ExpandConstant('{app}\tools\winsw.exe');
    if FileExists(WinSW) then
    begin
      if not Exec(WinSW, 'install labsentinel-service.xml',
                  ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ResultCode) then
        MsgBox('服务安装失败（WinSW 执行失败）。可在安装目录手工运行 winsw install。',
               mbInformation, MB_OK);
    end
    else
      MsgBox('未随包携带 winsw.exe：服务模式需手工安装（参见 deploy/windows/labsentinel-service.xml）。',
             mbInformation, MB_OK);
  end;
end;
