# 任务 18 · 启动/停止脚本完善（先读本目录 README.md）

## 背景

现有 `scripts/start-all.bat` 由任务 01 提供（拉起 go2rtc/uvicorn/web）。需要工程化：日志落盘、停止脚本、进程检查、首次运行引导。

## 目标（`scripts/` 目录）

1. `start-all.bat` 增强：
   - 检测端口 1984/8000/5173 占用并提示（占用则跳过对应组件而非报错退出）
   - 每个组件日志写 `logs/<name>.log`（目录自动创建）
   - 启动前检查 venv 与 node_modules 存在，缺失时给出安装命令提示
2. `stop-all.bat`：按端口找 PID 并结束（`netstat -ano | findstr :8000` + `taskkill`），带确认
3. `health-check.bat`：curl 三个端点（api/system、go2rtc api、vite），输出状态表
4. `README`（或写入部署文档对应节）：三脚本用法

## 约束

- 纯批处理（不引入 PowerShell 依赖）；路径含中文与空格要正确处理（`cd /d "%~dp0.."`）
- 不自动启动训练/推理
- 不修改 server/web 代码

## 验收标准

本机按顺序执行 start-all → health-check → stop-all，三个组件状态正确、日志文件生成、无残留进程；将实跑输出摘要写入执行报告。

## 执行报告

### 交付物
- **start-all.bat 增强**：环境预检（venv/node_modules 缺失给具体安装命令）；三端口占用检测
  （1984/8000/5173，占用即跳过该组件不报错退出）；每组件日志落 `logs\<name>.log`（目录自动创建）；
  ffmpeg PATH 注入保留
- **stop-all.bat（新）**：按端口找 PID（netstat+findstr LISTENING）→ 列表展示 → choice Y/N 确认 →
  taskkill /F；顺带按窗口标题（go2rtc/server/web）关闭 cmd /k 残留窗口壳
- **health-check.bat（新）**：curl 状态表（API / go2rtc / Web）；vite 仅绑 IPv6 用 `[::1]:5173` 探测
- **scripts/README.md（新）**：三脚本用法表 + GBK 编码坑/vite IPv6/端口误用提醒
- 部署指南 1.5 节指向 scripts/README（后续巡检同步）

### 实跑输出摘要（验收标准要求）
- `start-all.bat` 实跑：预检 [OK]；**8000/5173 被主线预览实例占用 → 正确跳过**（符合设计的跳过路径）；
  go2rtc 正常启动（1984 LISTENING），`logs\go2rtc.log` 生成（webrtc listen udp 日志确认）
- `health-check.bat` 实跑：三端点全 `[ OK ]`（API=主线预览实例、go2rtc=本次启动、Web=主线预览 vite）
- `stop-all.bat`：完整实跑会停掉主线预览的 8000/5173（协作协议禁止启停他人进程）→ **机制单点验证**：
  以 1984 为靶（taskkill 按端口杀 + WINDOWTITLE 清窗）实测"成功: 已终止 PID"；验证后主线 8000/5173 保持
  LISTENING 不受影响；go2rtc 已重启恢复（预览实例的实时画面可用）

### 验收状态
- 三脚本 GBK 编码（UTF-8 会让 cmd 中文解析崩溃——已实测并写入 scripts/README 已知坑）
- 端口占用跳过 / 日志落盘 / 停止机制 三项均实证；"完整 stop-all 循环"留待主线预览实例关闭后补跑
  （届时顺序执行 start-all → health-check → stop-all 即可，脚本已就绪）

### 追加交付（2026-10-27，用户直接指令）：根目录交互式一键启动器 `一键启动.bat`
- 流程：探测 `/api/system` → 已运行直接开面板；未运行调 start-all 并等待就绪（45s 超时告警）→
  自动打开控制面板（Web 优先、Web 未起回退 Swagger /docs）→ 菜单循环
  （[1]面板 [2]API文档 [3]健康检查 [4]日志目录 [5]重启 [6]停止 [0]退出脚本后台保活）
- `LS_LAUNCHER_NO_BROWSER=1` 禁自动弹浏览器（自动化测试用）
- 顺带修复 start-all.bat 尾部 errorlevel 泄漏（末尾端口检测 findstr 置 1 → call 方误判预检失败，
  补显式 `exit /b 0`）
- 实测四路径全过：冷启动（等待循环→就绪→三组件运行中）/ 已运行检测 / 菜单停止（三 PID 结束、
  端口清零）/ 冷启动回归。编码 GBK+CRLF；用法已写入 scripts/README.md

### 追加交付 2（2026-10-27，用户直接指令）：自动部署 + 重部署/卸载菜单 + demo 打包

**一键启动.bat 升级**：启动前先体检，缺组件自动调 deploy.py 部署；菜单新增
[7] 重部署保留数据 / [8] 重部署清空数据 / [9] 一键卸载（均有 Y/N 确认）。

**scripts/deploy.py（新，纯标准库）**：
- `check`：9 项体检（Python/venv/依赖含 DirectML/OpenH264/Node/web 依赖/ffmpeg/模型/.env）
- `install`：建 venv（py -3.12 优先）→ pip 清华镜像 → OpenH264 DLL → Node（缺则自动下便携版）→
  npm npmmirror → ffmpeg/ffprobe（npmmirror 单文件 .gz + npm tarball 抽取）→ 生成 server/.env →
  播种演示库（4 路摄像头/区域/规则）→ 写 scripts\venv_path.txt
- `redeploy --keep-data/--wipe`、`uninstall`（数据改名备份不直接删）
- 安全：下载前校验仅公网 http/https（拒绝环回/私有/保留地址）；项目外 venv 只解除引用绝不删除

**顺带修复（新机部署暴露）**：
- requirements.txt 漏 `python-multipart`（FastAPI 表单必需，缺了启动即 500）
- venv_path.txt 改 GBK 编码（cmd for /f 按 OEM 代码页读，UTF-8 中文路径乱码导致 start-all 找不到 venv）
- make_demo_zip.py（新）：demo 包 94MB（183 文件）；排除 server/.env（含机器路径）、server/data
  （运行时数据）、datasets/runs/ffmpeg/node_modules
- 快速开始.md（新，进包根目录）：三步上手 + 菜单表 + 常见问题

**实测记录**：
- 保留数据重部署：events 118 / cameras 4 / zones 4 / rules 8 全程不变，环境重建后 DirectML
  4 路 running、Web 200
- 清空重部署：数据目录清空→重新播种（events 0 / cameras 4 / rules 8），服务可用
- 新机冷部署终极验证：demo 包解压到 D:\tmp\demo-test（干净目录、无任何环境）→ 双击一键启动 →
  自动建 venv 装依赖下 ffmpeg 播种库 → DirectML 4 路 running、事件 84 条、Web 200、exit 0
- 产物：`dist/LabSentinel-demo-20261027.zip`（94.0 MB）
