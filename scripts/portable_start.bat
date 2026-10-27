@echo off
rem LabSentinel portable launcher (no console window; data in %LOCALAPPDATA%\LabSentinel)
set "PYW=%~dp0runtime\pythonw.exe"
if not exist "%PYW%" set "PYW=%~dp0runtime\python.exe"
start "" "%PYW%" "%~dp0scripts\portable_launcher.py"
