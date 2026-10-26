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
