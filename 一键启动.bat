@echo off
rem LabSentinel 一键启动器（交互式菜单版）
rem 流程：Python 探测 -> 环境体检 -> 缺啥自动部署（venv/pip/npm/ffmpeg/配置） -> 探测服务 -> 未启动则拉起 -> 等就绪 -> 自动打开控制面板 -> 菜单
rem 自动化测试可设置环境变量 LS_LAUNCHER_NO_BROWSER=1 禁止自动弹浏览器

setlocal enabledelayedexpansion
cd /d "%~dp0"
title LabSentinel 启动器
if not exist logs mkdir logs

rem ---- Python 解释器探测：py 启动器优先，其次 PATH 里的 python ----
rem 不做管理员提权：本流程只写项目目录；提权会切换到管理员环境，新机上反而可能丢 PATH 里的 python
set "PYCMD="
if not defined PYCMD (
    py -3 --version >nul 2>&1
    if not errorlevel 1 set "PYCMD=py -3"
)
if not defined PYCMD (
    python --version >nul 2>&1
    if not errorlevel 1 set "PYCMD=python"
)
if not defined PYCMD (
    if exist "dist\portable\LabSentinel\启动.bat" (
        echo [兜底] 未找到 Python，但检测到便携包，改为启动便携版...
        start "" "dist\portable\LabSentinel\启动.bat"
        exit /b 0
    )
    echo [失败] 未找到 Python 3.10+（首次自动部署需要它）。
    echo        请从 https://www.python.org/downloads/ 安装，勾选 "Add python.exe to PATH"，
    echo        装好后重新双击本脚本；或在已装 Python 的终端里执行：
    echo          py -3 scripts\deploy.py install
    pause
    exit /b 1
)

rem 便携包兜底：普通用户已有 dist\portable\LabSentinel 时优先提示/启动便携版
if exist "dist\portable\LabSentinel\启动.bat" (
    echo [提示] 检测到便携包：普通用户建议直接运行 dist\portable\LabSentinel\启动.bat（无需 Python/Node）。
)

echo [体检] 检查运行环境...
%PYCMD% scripts\deploy.py check --quiet
if not errorlevel 1 (
    echo   [OK] 环境就绪，无需部署。
    goto CHECK_SVC
)
echo.
echo [部署] 环境不完整，开始自动部署（首次约 5-10 分钟，组件齐全时秒过）...
%PYCMD% scripts\deploy.py install
if errorlevel 1 (
    echo.
    echo [失败] 自动部署未完成，请按上方提示处理后重新运行本脚本。
    echo        也可手动执行：%PYCMD% scripts\deploy.py check
    pause
    exit /b 1
)
echo.

:CHECK_SVC
call :CHECK_ALL
if "%API_OK%"=="1" (
    echo [OK] 检测到服务已在运行（API 端口 8000 健康），无需重复启动。
    goto PANEL
)
netstat -ano | findstr ":8000 " | findstr "LISTENING" >nul 2>&1
if not errorlevel 1 (
    echo [警告] 端口 8000 被其他程序占用且健康检查未通过，服务端可能无法启动。
)
goto DO_START

:DO_START
echo.
echo [启动] 拉起组件：go2rtc(1984) / 服务端(8000) / Web(5173)，已在运行的自动跳过...
call scripts\start-all.bat
if errorlevel 1 (
    echo [失败] 启动预检未通过（见上方提示）。
    pause
    goto MENU
)
echo [等待] 服务端与 Web 就绪中（最多 60 秒）...
set /a TRIES=0
:WAIT_LOOP
call :CHECK_ALL
if "%API_OK%%WEB_OK%"=="11" goto WAIT_DONE
set /a TRIES+=1
if %TRIES% geq 60 (
    echo [警告] 等待超时：请查看 logs\server.log 与 logs\web.log
    goto PANEL
)
ping -n 2 127.0.0.1 >nul
goto WAIT_LOOP
:WAIT_DONE
echo [OK] 服务端已就绪。

:PANEL
call :OPEN_PANEL

:MENU
cls
echo.
echo ==============================================================
echo                 LabSentinel 控制面板启动器
echo ==============================================================
call :CHECK_ALL
set S1=[已停止]
set S2=[已停止]
set S3=[已停止]
if "%API_OK%"=="1" set S1=[运行中]
if "%WEB_OK%"=="1" set S2=[运行中]
if "%GO_OK%"=="1" set S3=[运行中]
echo   当前状态：服务端 !S1!   Web !S2!   go2rtc !S3!
echo --------------------------------------------------------------
echo   [1] 打开控制面板（Web 界面）
echo   [2] 打开 API 文档（Swagger）
echo   [3] 健康检查（三组件状态表）
echo   [4] 打开日志目录（logs\）
echo   [5] 重启服务（先停止再启动）
echo   [6] 停止服务（退出全部组件）
echo   [7] 重新部署（保留数据）——重装环境与依赖，事件/摄像头配置不动
echo   [8] 重新部署（清空数据）——重装环境并重置为演示初始状态
echo   [9] 一键卸载——停止服务并删除环境与组件（数据备份保留）
echo   [0] 退出启动脚本（服务继续在后台运行）
echo --------------------------------------------------------------
choice /C 1234567890 /N /M "请选择: "
if errorlevel 10 goto QUIT
if errorlevel 9 goto UNINSTALL
if errorlevel 8 goto REDEPLOY_WIPE
if errorlevel 7 goto REDEPLOY_KEEP
if errorlevel 6 goto STOP
if errorlevel 5 goto RESTART
if errorlevel 4 goto LOGS
if errorlevel 3 goto HEALTH
if errorlevel 2 goto DOCS
if errorlevel 1 goto PANEL

:DOCS
call :CHECK_ALL
if "%API_OK%"=="0" (
    echo [提示] 服务端未运行，请先选择 [5] 重启服务。
    pause
    goto MENU
)
echo [文档] http://127.0.0.1:8000/docs
if not defined LS_LAUNCHER_NO_BROWSER start "" http://127.0.0.1:8000/docs
goto MENU

:HEALTH
call scripts\health-check.bat
echo.
pause
goto MENU

:LOGS
if not exist logs mkdir logs
explorer "%~dp0logs"
goto MENU

:STOP
call scripts\stop-all.bat
echo.
pause
goto MENU

:RESTART
echo.
echo [重启] 先停止现有组件...
call scripts\stop-all.bat
echo.
echo [重启] 重新拉起组件...
goto DO_START

:REDEPLOY_KEEP
echo.
echo [重部署] 将重装运行环境（venv/依赖/下载组件/配置），server\data 数据全部保留。
choice /C YN /M "确认继续"
if errorlevel 2 goto MENU
echo.
%PYCMD% scripts\deploy.py redeploy --keep-data
echo.
pause
goto MENU

:REDEPLOY_WIPE
echo.
echo [重部署] 将重装运行环境并【清空全部数据】（事件/截图/视频/摄像头配置），重置为演示初始状态。
choice /C YN /M "确认清空数据并继续"
if errorlevel 2 goto MENU
echo.
%PYCMD% scripts\deploy.py redeploy --wipe
echo.
pause
goto MENU

:UNINSTALL
echo.
echo [卸载] 将停止服务并删除：venv、npm 依赖、下载的 Node/ffmpeg、配置与日志。
echo        server\data 数据会改名备份（不直接删除）。
choice /C YN /M "确认卸载"
if errorlevel 2 goto MENU
echo.
%PYCMD% scripts\deploy.py uninstall
echo.
pause
goto MENU

:QUIT
echo.
echo 已退出启动脚本，服务保持在后台运行：
echo   Web 控制面板 http://localhost:5173
echo   API 文档     http://127.0.0.1:8000/docs
echo   go2rtc       http://localhost:1984
echo 提示：再次双击本脚本可重新打开控制面板；停止服务请选菜单 [6] 或 scripts\stop-all.bat
pause
endlocal
exit /b 0

:OPEN_PANEL
call :CHECK_ALL
if "%API_OK%"=="0" (
    echo [提示] 服务端未运行，暂不打开面板。
    goto :eof
)
if "%WEB_OK%"=="1" (
    echo [面板] 打开 Web 控制面板：http://localhost:5173
    if not defined LS_LAUNCHER_NO_BROWSER start "" http://localhost:5173
) else (
    echo [面板] Web 未运行，改开 API 文档：http://127.0.0.1:8000/docs
    if not defined LS_LAUNCHER_NO_BROWSER start "" http://127.0.0.1:8000/docs
)
goto :eof

:CHECK_ALL
set API_OK=0
set WEB_OK=0
set GO_OK=0
curl -s -o nul --max-time 3 http://127.0.0.1:8000/api/system >nul 2>&1
if not errorlevel 1 set API_OK=1
rem vite 默认只绑 IPv6 localhost；关闭了 IPv6 的机器会绑 127.0.0.1，两个都探
curl -g -s -o nul --max-time 3 "http://[::1]:5173" >nul 2>&1
if errorlevel 1 curl -s -o nul --max-time 3 "http://127.0.0.1:5173" >nul 2>&1
if not errorlevel 1 set WEB_OK=1
curl -s -o nul --max-time 3 http://127.0.0.1:1984/api >nul 2>&1
if not errorlevel 1 set GO_OK=1
goto :eof
