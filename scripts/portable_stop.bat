@echo off
rem LabSentinel stop (task 120: prefers installed copy registered in control panel; else this package)
set "LS_INST="
for /f "tokens=2*" %%a in ('reg query HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\LabSentinel /v InstallLocation 2^>nul ^| findstr /i InstallLocation') do set "LS_INST=%%b"
if defined LS_INST if exist "%LS_INST%LabSentinel.exe" (
  "%LS_INST%LabSentinel.exe" --stop
  pause
  exit /b 0
)
"%~dp0runtime\python.exe" "%~dp0scripts\portable_launcher.py" --stop
pause
