@echo off
rem LabSentinel stop (task 120: prefers installed copy registered in control panel; else this package)
rem 2026-11-13 P3: exe --stop 失败（rc!=0）时回退本包 runtime python 直调 launcher，
rem 不再恒 exit /b 0 掩盖失败；最终 rc 回传给调用方。
set "LS_INST="
for /f "tokens=2*" %%a in ('reg query HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\LabSentinel /v InstallLocation 2^>nul ^| findstr /i InstallLocation') do set "LS_INST=%%b"
if defined LS_INST if exist "%LS_INST%LabSentinel.exe" (
  "%LS_INST%LabSentinel.exe" --stop
  if not errorlevel 1 (
    pause
    exit /b 0
  )
  echo installed-copy --stop failed (rc=%errorlevel%), falling back to package runtime...
)
"%~dp0runtime\python.exe" "%~dp0scripts\portable_launcher.py" --stop
set "LS_RC=%errorlevel%"
pause
exit /b %LS_RC%
