@echo off
rem LabSentinel 一键启动（Rust 单二进制 · 源码/开发模式）
rem 发布包用户请直接双击解压目录里的 LabSentinel.bat（无需本脚本）
setlocal
cd /d "%~dp0"

set "EXE=engine-rs\target\release\labsentinel-server.exe"
if not exist "%EXE%" set "EXE=engine-rs\target\debug\labsentinel-server.exe"
if not exist "%EXE%" (
    echo [缺少] 未找到 Rust 服务端产物，请先构建：
    echo     cd engine-rs ^&^& cargo build --release -p ls-server
    echo   或直接使用发布包里的 LabSentinel.bat
    pause
    exit /b 1
)
if not exist "web\dist\index.html" (
    echo [提示] web\dist 不存在，请先执行：
    echo     cd web ^&^& npm ci ^&^& npm run build
    pause
    exit /b 1
)

set "LS_WEB_DIST=%~dp0web\dist"
set "LS_DATA_DIR=%~dp0.data"
set "LS_DB_PATH=%~dp0.data\labsentinel.db"
if not exist "%~dp0.data" mkdir "%~dp0.data"

rem 本地模型（存在则注入；不存在时由网页「模型管理」选择）
if not defined LS_MODEL_PATH if exist "%~dp0models\yolo11n.onnx" set "LS_MODEL_PATH=%~dp0models\yolo11n.onnx"
if not defined LS_MODEL_PATH if exist "%~dp0m0-spike\yolo11n.onnx" set "LS_MODEL_PATH=%~dp0m0-spike\yolo11n.onnx"
if not defined LS_PPE_MODEL if exist "%~dp0models\yolo11n_ppe.onnx" set "LS_PPE_MODEL=%~dp0models\yolo11n_ppe.onnx"
if not defined LS_PPE_MODEL if exist "%~dp0m0-spike\yolo11n_ppe.onnx" set "LS_PPE_MODEL=%~dp0m0-spike\yolo11n_ppe.onnx"
if not defined ORT_DYLIB_PATH if exist "%~dp0tools\onnxruntime\onnxruntime.dll" set "ORT_DYLIB_PATH=%~dp0tools\onnxruntime\onnxruntime.dll"

"%EXE%" --launch
pause
endlocal
