@echo off
rem LabSentinel 开发启动：Rust 服务端（托管 go2rtc）+ Web 开发服
rem 生产/发布包只需 LabSentinel.bat（Rust 直接托管 web/dist）；本脚本面向改代码调试。
setlocal enabledelayedexpansion
cd /d "%~dp0.."
if not exist logs mkdir logs

set "EXE=engine-rs\target\release\labsentinel-server.exe"
if not exist "%EXE%" set "EXE=engine-rs\target\debug\labsentinel-server.exe"
if defined CARGO_TARGET_DIR if exist "%CARGO_TARGET_DIR%\release\labsentinel-server.exe" set "EXE=%CARGO_TARGET_DIR%\release\labsentinel-server.exe"
if defined CARGO_TARGET_DIR if not exist "%EXE%" set "EXE=%CARGO_TARGET_DIR%\debug\labsentinel-server.exe"
if not exist "%EXE%" (
    echo   [缺失] Rust 服务端未构建：cd engine-rs ^&^& cargo build --release -p ls-server
    pause & exit /b 1
)
if not exist "web\node_modules" (
    echo   [缺失] web\node_modules 不存在：cd web ^&^& npm install
    pause & exit /b 1
)

set SERVER_SKIPPED=0
set WEB_SKIPPED=0
netstat -ano | findstr ":8000" | findstr "LISTENING" >nul 2>&1
if not errorlevel 1 ( set SERVER_SKIPPED=1 & echo   [跳过] 8000 已被占用 )
netstat -ano | findstr ":5173" | findstr "LISTENING" >nul 2>&1
if not errorlevel 1 ( set WEB_SKIPPED=1 & echo   [跳过] 5173 已被占用 )
if "%SERVER_SKIPPED%"=="0" (
    echo [1/2] 启动 Rust 服务端 ^(端口 8000^)...
    set "LS_INSTALL_DIR=%CD%"
    set "LS_WEB_DIST=%CD%\web\dist"
    set "LS_DATA_DIR=%CD%\.data"
    set "LS_DB_PATH=%CD%\.data\labsentinel.db"
    start "server" /D "%~dp0.." cmd /k ""%EXE%" >> logs\server.log 2>&1"
) else (
    echo [1/2] 服务端跳过
)

if "%WEB_SKIPPED%"=="0" (
    echo [2/2] 启动 Web 开发服 ^(端口 5173^)...
    start "web" /D "%~dp0..\web" cmd /k "npm run dev >> ..\logs\web.log 2>&1"
) else (
    echo [2/2] Web 跳过
)

echo.
echo 完成：Web http://localhost:5173  API http://localhost:8000
echo 停止：scripts\stop-all.bat
exit /b 0
endlocal
