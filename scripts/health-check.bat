@echo off
rem LabSentinel 健康检查：三个端点状态表（API / go2rtc / Web dev）
setlocal enabledelayedexpansion

echo LabSentinel 健康检查 %date% %time%
echo ==============================================

curl -s -o nul --max-time 3 http://127.0.0.1:8000/api/system >nul 2>&1
if errorlevel 1 ( set S1=[DOWN] ) else ( set S1=[ OK ] )
curl -s -o nul --max-time 3 http://127.0.0.1:1984/api >nul 2>&1
if errorlevel 1 ( set S2=[DOWN] ) else ( set S2=[ OK ] )
rem vite dev 仅绑 IPv6 localhost（README 已知事项），用 [::1] 探测；-g 关闭 curl 的方括号通配
curl -g -s -o nul --max-time 3 "http://[::1]:5173" >nul 2>&1
if errorlevel 1 ( set S3=[DOWN] ) else ( set S3=[ OK ] )

echo !S1! API   http://127.0.0.1:8000/api/system
echo !S2! go2rtc http://127.0.0.1:1984/api
echo !S3! Web   http://localhost:5173 ^(IPv6 localhost^)
echo ==============================================
echo 提示：启动用 scripts\start-all.bat，停止用 scripts\stop-all.bat
endlocal
