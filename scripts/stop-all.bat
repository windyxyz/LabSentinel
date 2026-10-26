@echo off
rem LabSentinel 一键停止：按端口找 PID 结束 go2rtc(1984) / 服务端(8000) / vite(5173)
setlocal enabledelayedexpansion
cd /d "%~dp0.."

echo 将停止以下组件（按端口找 PID）：
set FOUND=0
for %%p in (1984 8000 5173) do (
    for /f "tokens=5" %%a in ('netstat -ano ^| findstr ":%%p " ^| findstr "LISTENING"') do (
        echo   端口 %%p -^> PID %%a
        set FOUND=1
    )
)
if "%FOUND%"=="0" (
    echo 没有发现 LabSentinel 相关进程（1984/8000/5173 均未监听）。
    pause & exit /b 0
)

choice /C YN /M "确认停止以上进程"
if errorlevel 2 ( echo 已取消 & exit /b 0 )

for %%p in (1984 8000 5173) do (
    for /f "tokens=5" %%a in ('netstat -ano ^| findstr ":%%p " ^| findstr "LISTENING"') do (
        taskkill /PID %%a /F >nul 2>&1 && echo   已结束 PID %%a（端口 %%p）
    )
)
rem 顺带关闭 start-all 留下的控制台窗口壳（按窗口标题）
taskkill /FI "WINDOWTITLE eq go2rtc*" /F >nul 2>&1
taskkill /FI "WINDOWTITLE eq server*" /F >nul 2>&1
taskkill /FI "WINDOWTITLE eq web*" /F >nul 2>&1
echo 停止完成。日志保留在 logs\。
endlocal
