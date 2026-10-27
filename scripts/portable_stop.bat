@echo off
rem LabSentinel portable stop (kills uvicorn by lock pid)
"%~dp0runtime\python.exe" "%~dp0scripts\portable_launcher.py" --stop
pause
