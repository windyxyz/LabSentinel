# 50 · 任务包（子代理工作分发中心）

> 用途：每个 `NN-*.md` 是一份**自包含**的任务规格，可直接发给 AI 代理（含轻量模型）独立执行。
> 使用方法：把 `README.md`（本文件）+ 目标任务文档一起粘贴给代理。

## 领任务前必读：项目环境与铁律

### 项目概况

**LabSentinel/实验室哨兵**：开源实验室安全行为监控系统。服务端 FastAPI + ONNX Runtime 推理（人员检测/ByteTrack 跟踪/安全区规则/事件告警），Web 端 Vue3。项目愿景见 [../00-vision.md](../00-vision.md)，术语表见 [../../GLOSSARY.md](../../GLOSSARY.md)。

### 关键路径（Windows 机器）

| 项 | 路径/命令 |
|---|---|
| 项目根 | `D:\User\Desktop\TOOLS\监控识别` |
| 服务端 | `server/`（FastAPI，代码在 `server/app/`） |
| Web 端 | `web/`（Vue3+Vite，源码 `web/src/`） |
| Python 环境 | 项目内 `.venv\Scripts\python.exe`（`python scripts\deploy.py install` 创建，路径见 `scripts/venv_path.txt`）。**m0-venv（`D:\LabSentinel\m0-venv`）是旧装训练专用环境**（ultralytics/torch），仅训练/导出用，勿动其中包版本 |
| 启动服务端 | `cd server && ..\.venv\Scripts\python.exe -m uvicorn app.main:app --port 8000`（推荐直接跑 `一键启动.bat` 或 `scripts\start-all.bat`） |
| 启动 Web 开发服 | `cd web && npm run dev`（浏览器 http://localhost:5173；仅绑 IPv6 localhost，curl 测试用 `[::1]`） |
| 单元测试 | `cd server && D:\LabSentinel\m0-venv\Scripts\python.exe -m pytest tests/ -q`（pytest 装在训练环境 m0-venv；项目 `.venv` 未装 pytest，当前基线 91 passed，以最近执行报告为准） |
| 演示视频 | `m0-spike/data/lab-walk-24076.mp4`、`lab-equip-21453.mp4` |
| 模型文件 | `m0-spike/*.onnx`、`m0-spike/*.pt` |

### 网络铁律（这台机器直连部分境外源会失败）

- pip 一律加 `-i https://pypi.tuna.tsinghua.edu.cn/simple`
- npm 一律加 `--registry=https://registry.npmmirror.com`
- GitHub 下载用镜像前缀 `https://ghfast.top/https://github.com/...`；断点续传用 `curl -C -` 循环
- 需要 Roboflow/HuggingFace 等外网 API 时走本地代理：环境变量 `HTTPS_PROXY=http://127.0.0.1:7890`

### 技术禁忌（违反会破坏现有系统）

1. **🔴 推理并发铁律（2026-10-26 死机事故后新增）**：DirectML/VitisAI 会话**严禁跨线程并发 Run**——已两次导致整机死机（Kernel-Power 41）。所有推理调用必须走 `Engine._run()`（内部有全局串行锁）；**代码中出现 `sess.run` 直接视为不合格**（grep 审查项）。并发压测一次只跑一个进程，且不得与训练/其他压测同时进行。
2. **重负载一次只跑一个**：训练、压测、批量推理不得并发（CPU 争用曾与 DML 缺陷叠加触发死机）。训练必须 `torch.set_num_threads(6)` 限线程 + 过夜跑；**巡检任务可按《[训练安全规程](./训练安全规程.md)》受控启动训练**（前置条件/看门狗/完成链路全在规程里，参数不许自改）。
3. **`supervision` 必须钉在 0.25.1**——0.26+ 移除了 ByteTrack（License 是 MIT）。
4. **`onnxruntime`（CPU 版）与 `onnxruntime-directml` 不可同装**，会互相损坏（卸载要两个一起卸再重装 directml 版）。
5. **m0-venv 里 numpy 版本不可改动**（依赖二进制兼容）。
6. 服务端改 Python 依赖前必须先停掉正在运行的 uvicorn（DLL 锁）。
7. **Git Bash 的 curl 发含中文的 JSON 会被转码损坏**——测试 API 一律用 python httpx 写脚本。
8. 开源协议为 **AGPL-3.0**；新增第三方依赖必须核查协议并写进任务报告。
9. 不要改动 `m0-spike/` 与 `docs/20-research/`（历史实测记录）。
10. Roboflow 的 YOLO 导出可能是**分割多边形格式**（一行几十个坐标点），解析代码必须兼容多边形→外接框，否则大量标签会被静默丢弃（2026-10-26 实测 4/5 的框因此丢失）。

### 完成标准（每个任务通用）

- 单元测试全绿（`pytest tests/ -q`）；
- 涉及前端时 `npm run build` 通过；
- 新增代码符合现有风格（中文注释、模块化、不引重型框架）；
- 在任务文档末尾追加"## 执行报告"段落：改了哪些文件、验收结果、遗留问题。

### 并行协作边界（2026-10-26，主线与子代理同时开发时必读）

- **主线正在负责**：`web/src/theme.css`、`web/src/style.css`、`web/src/App.vue`、`web/src/main.js`（WinUI 主题换肤，已完成冻结）。**子代理不要修改这四个文件**；需要的局部样式写在自己组件的 `<style scoped>` 里。
- **子代理任务 12–15 可自由修改**：`server/app/*.py`、`server/tests/*.py`、`web/src/views/{Events,Dashboard,Reports,Face}.vue`（写前先读最新内容，只做任务范围内的增量）。
- **端口约定**：主线预览占用 8000（服务端）/5173（vite）；子代理测试如需启动服务，用 `--port 8001`，或在测试里直接构造 app（TestClient），不要抢占 8000。
- **文件冲突原则**：任何共享文件写入前重读；若发现他人刚改过，在最新内容上做增量，禁止整文件覆盖回旧版。
- **重负载纪律**：一次只跑一个重任务（训练/压测/批量推理），跑前确认对方空闲；训练必须限线程（铁律 2）。

## 任务清单（按优先级）

| 文档 | 任务 | 难度 | 适合模型 |
|---|---|---|---|
| [01-webrtc-live-view.md](./01-webrtc-live-view.md) | 实时画面（go2rtc/WebRTC 集成） | ★★★ | 主力模型 |
| [02-reports-page.md](./02-reports-page.md) | 统计报表（前后端） | ★★ | flash 可 |
| [03-alert-channels.md](./03-alert-channels.md) | 钉钉通知渠道 + 告警模板 | ★ | flash 可 |
| [04-video-evidence.md](./04-video-evidence.md) | 事件短视频存证 | ★★ | flash 可 |
| [05-face-module.md](./05-face-module.md) | 人脸比对模块（CompreFace，默认关） | ★★★ | 主力模型 |
| [06-training-toolchain.md](./06-training-toolchain.md) | 训练/量化可复现脚本 | ★★ | flash 可 |
| [07-multi-stream-test.md](./07-multi-stream-test.md) | 多路并发压测 | ★★ | flash 可 |
| [08-android-client.md](./08-android-client.md) | Flutter 安卓客户端骨架 | ★★★ | 主力模型 |
| [09-alert-polish.md](./09-alert-polish.md) | 告警通道打磨（异步化/H264/文案） | ★★ | flash 可 |
| [10-deploy-docs.md](./10-deploy-docs.md) | 部署文档与 .env.example | ★ | flash 可 |
| [11-annotation-toolchain.md](./11-annotation-toolchain.md) | 标注工具链（抽帧+预标注，数据飞轮首轮） | ★★ | flash 可 |
| [12-event-triage.md](./12-event-triage.md) | 事件闭环（确认/误报/忽略 + 误报回流标注池） | ★★ | flash 可 |
| [13-push-reliability.md](./13-push-reliability.md) | 推送可靠性（重试/死信/渠道自检） | ★★ | flash 可 |
| [14-data-retention.md](./14-data-retention.md) | 数据保留与清理（合规+磁盘保护） | ★★ | flash 可 |
| [15-report-export.md](./15-report-export.md) | 报表导出（CSV/PDF 月报） | ★★ | flash 可 |

### 近期批次（M1.5 / 发布收尾）

| 文档 | 任务 | 状态 |
|---|---|---|
| [56-usb-virtual-camera.md](./56-usb-virtual-camera.md) | USB/OBS 虚拟摄像头接入验证 | ✅ 2026-10-28 |
| [57-multi-rtsp-sim.md](./57-multi-rtsp-sim.md) | 多路模拟 RTSP 压测 | 执行中 |
| [58-release-manifest.md](./58-release-manifest.md) | 发布产物清单/SHA256 | 已派发 |
| [59-portable-smoke.md](./59-portable-smoke.md) | 便携包一键冒烟 | 已派发 |
| [60-usb-device-discovery.md](./60-usb-device-discovery.md) | USB 设备枚举 API + UI | 已派发 |
| [61-logging-diagnostics.md](./61-logging-diagnostics.md) | 日志轮转 + 诊断包 | 已派发 |
| [62-backup-restore.md](./62-backup-restore.md) | 备份/恢复 CLI | 已派发 |

> 完成情况（2026-10-26 晚）：01–11 全部交付（07 压测待锁修复后补测、08 apk 待 Flutter SDK）。
> 12–15 为 2026-10-26 新派发批次（框架期任务，与模型训练解耦，可立即开工）。
> 关联文档：架构 [../30-architecture.md](../30-architecture.md) · API 契约 [../35-api-reference.md](../35-api-reference.md) · 模型接入 [../45-model-onboarding.md](../45-model-onboarding.md) · 里程碑 [../60-milestones.md](../60-milestones.md)。
>
> 附加已完成：M1.5#2 词表同步（PpeEngine 读 `<model>.classes.txt`，换模型零改代码）；飞轮工具 `eval_report.py` / `export_dataset.py`；`make_demo.py` 参数化（--ppe/--suffix 支持 v1/v2 对比）；**v1 在 merged test 集基线 = Person P32% / Lab Coat P22%**（→ v2 重训依据，见 [../52-model-eval.md](../52-model-eval.md)）。
