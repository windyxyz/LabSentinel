@echo off
rem LabSentinel 一键启动：go2rtc（实时画面网关）+ 服务端 + Web 开发服（任务18 工程化版）
rem 特性：端口占用检测（占用则跳过该组件）、日志落盘 logs\、环境预检（venv/node_modules）
setlocal enabledelayedexpansion

rem 路径含中文与空格：一律 cd /d "%~dp0.." 再用相对路径
cd /d "%~dp0.."
if not exist logs mkdir logs

echo [预检] 环境...
if not exist "D:\LabSentinel\m0-venv\Scripts\python.exe" (
    echo   [缺失] Python venv 不存在，先执行：
    echo     python -m venv D:\LabSentinel\m0-venv ^&^& D:\LabSentinel\m0-venv\Scripts\python.exe -m pip install -r server\requirements.txt -i https://pypi.tuna.tsinghua.edu.cn/simple
    pause & exit /b 1
)
if not exist "web\node_modules" (
    echo   [缺失] web\node_modules 不存在，先执行：
    echo     cd web ^&^& npm install --registry=https://registry.npmmirror.com
    pause & exit /b 1
)
echo   [OK] venv 与 node_modules 就绪

rem ---- 端口占用检测：占用则跳过对应组件 ----
set GO2RTC_SKIPPED=0
set SERVER_SKIPPED=0
set WEB_SKIPPED=0
netstat -ano | findstr ":1984" | findstr "LISTENING" >nul 2>&1
if not errorlevel 1 ( set GO2RTC_SKIPPED=1 & echo   [跳过] 1984 已被占用（go2rtc 可能已在跑） )
netstat -ano | findstr ":8000" | findstr "LISTENING" >nul 2>&1
if not errorlevel 1 ( set SERVER_SKIPPED=1 & echo   [跳过] 8000 已被占用（服务端可能已在跑） )
netstat -ano | findstr ":5173" | findstr "LISTENING" >nul 2>&1
if not errorlevel 1 ( set WEB_SKIPPED=1 & echo   [跳过] 5173 已被占用（vite 可能已在跑） )

rem ---- go2rtc ----
if "%GO2RTC_SKIPPED%"=="0" (
    echo [1/3] 启动 go2rtc ^(端口 1984，日志 logs\go2rtc.log^)...
    rem go2rtc 的 ffmpeg: 文件源依赖 ffmpeg.exe（tools\ffmpeg），加入 PATH
    start "go2rtc" /D "%~dp0.." cmd /k "set PATH=%~dp0..\tools\ffmpeg;%PATH% && tools\go2rtc.exe --config tools\go2rtc.yaml >> logs\go2rtc.log 2>&1"
) else (
    echo [1/3] go2rtc 跳过（端口已占用）
)

rem ---- 服务端 ----
if "%SERVER_SKIPPED%"=="0" (
    echo [2/3] 启动服务端 ^(端口 8000，日志 logs\server.log^)...
    start "server" /D "%~dp0..\server" cmd /k "D:\LabSentinel\m0-venv\Scripts\python.exe -m uvicorn app.main:app --port 8000 >> ..\logs\server.log 2>&1"
) else (
    echo [2/3] 服务端跳过（端口已占用）
)

rem ---- Web 开发服 ----
if "%WEB_SKIPPED%"=="0" (
    echo [3/3] 启动 Web 开发服 ^(端口 5173，日志 logs\web.log^)...
    start "web" /D "%~dp0..\web" cmd /k "npm run dev >> ..\logs\web.log 2>&1"
) else (
    echo [3/3] Web 跳过（端口已占用）
)

echo.
echo 全部完成：Web http://localhost:5173  API http://localhost:8000  go2rtc http://localhost:1984
echo 日志目录：logs\（go2rtc.log / server.log / web.log）；停止请运行 scripts\stop-all.bat
endlocal
