<div align="center">

# 🧪 LabSentinel · 实验室哨兵

**开源实验室安全行为智能监控系统**：接入现有摄像头（RTSP）、USB/视频文件，或外部软件持续截图的图片文件夹，自动识别进入安全区未穿实验服、统计人数，异常实时多渠道告警。全本地部署，隐私可控。

![Release](https://img.shields.io/badge/Release-0.2.0-blue) ![Python](https://img.shields.io/badge/Python-3.12-blue) ![Vue](https://img.shields.io/badge/Vue-3-42b883) ![FastAPI](https://img.shields.io/badge/FastAPI-latest-009688) ![Rust](https://img.shields.io/badge/Rust-控制面%20105%2F106%20路由-dea584) ![License](https://img.shields.io/badge/License-AGPL--3.0-red) ![Platform](https://img.shields.io/badge/Platform-Windows-lightgrey)

</div>

## ✨ 功能一览（30 秒了解）

- 🔍 **白大褂检测**：进入安全区自动判定穿着状态（证据分级判定，不把"看不清"当"违规"误报）
- 🖼️ **图片文件夹自动识别**：外部截图软件持续写入目录 → 自动识别关键图片（有人/违规）并按摄像头/时间汇报，原始截图只读
- 👥 **人数统计**：安全区在场人数统计与超限告警
- 🚨 **事件闭环**：告警 → 确认/误报/忽略 → 误报样本自动进标注池反哺模型
- 📢 **多渠道告警**：企业微信 / 钉钉 / 飞书 / 微信（Server酱·PushPlus）/ QQ（Qmsg）/ 邮件，失败重试+死信可查；网页端保存即生效，密钥加密存储
- 🖥️ **实时画面**：WebRTC 端到端约 1 秒延迟（go2rtc 网关），令牌鉴权子协议
- 📊 **统计报表**：日/小时趋势、CSV / PDF 月报导出（安全汇报直接用）
- 🎥 **视频存证**：事件前后各 2 秒 H.264 短视频 + 证据包 ZIP 导出
- 👤 **人脸比对**（默认关闭）：CompreFace 集成，合规红线齐备
- 🔧 **Rust 引擎灰度**：摄像头级切换 python↔rust 分析器（gRPC sidecar），Rust 控制面路由覆盖 105/106
- 🎛️ **双主题 UI**：WinUI 3 设计令牌，浅色/深色随系统

## 🖼️ 界面预览

| 仪表盘 | 事件查询与处置 |
|---|---|
| ![仪表盘](docs/assets/dashboard.png) | ![事件查询与详情抽屉](docs/assets/events-drawer.png) |
| **安全区配置** | **统计报表** |
| ![安全区配置](docs/assets/zones.png) | ![统计报表](docs/assets/reports.png) |

> 更多截图见 [docs/assets/](docs/assets/README.md)（含人脸管理页）。

## 🚀 快速开始

### 方式 A：安装包（推荐，向导式）

下载 **`LabSentinel-setup-<版本>.exe`** → 双击 → 选择语言（简体中文/English）→
选择安装模式（**仅为我** / **为所有用户**）与安装位置 → 完成页勾选开始菜单/桌面快捷方式（默认全勾）→
安装完成自动健康校验并启动。

- 支持从开始菜单/控制面板卸载；升级直接重跑安装包（自动归档旧版本，健康检查失败自动回滚）
- 可选安装为 Windows 服务（仅建议 RTSP/文件夹源）

### 方式 B：便携包（免安装）

下载 **`LabSentinel-portable-<版本>.zip`** 解压 → 双击 `安装.bat`（生成快捷方式）→
双击桌面 **LabSentinel**。**不需要安装 Python/Node**（包内自带嵌入式运行时），运行无需联网。

- 数据在 `%LOCALAPPDATA%\LabSentinel`；`LabSentinel.exe --stop` 停止服务
- 详见 [docs/guide/安装与卸载.md](docs/guide/安装与卸载.md)

### 方式 C：源码部署（开发模式）

```bash
# 1. 服务端（Python 3.12，哈希锁安装）
cd server
pip install --require-hashes --no-deps -r requirements-win.lock   # Linux 用 requirements-linux.lock
python -m uvicorn app.main:app --port 8000

# 2. Web 端
cd web
npm ci --registry=https://registry.npmmirror.com
npm run dev     # http://localhost:5173（API 已代理到 8000）
```

一键脚本（自动体检/部署/端口检测/健康检查）：见 [scripts/README.md](scripts/README.md)。
完整生产化指南（RTSP 模板、告警渠道申请、合规自查）：**[docs/guide/部署指南.md](docs/guide/部署指南.md)**。

## 🧭 文档索引（docs/ 已按用途分组）

| 分组 | 内容 |
|---|---|
| **[docs/guide/](docs/guide/)** — 使用与部署 | [部署指南](docs/guide/部署指南.md)（Windows/Linux 双平台）、[安装与卸载](docs/guide/安装与卸载.md)、[运维手册](docs/guide/运维手册.md)、[摄像头接入 Runbook](docs/guide/摄像头接入Runbook.md)、[图片文件夹接入](docs/guide/图片文件夹接入.md)、[部署 FAQ](docs/guide/部署FAQ.md)、[杀毒软件误报说明](docs/guide/杀毒软件误报说明.md) |
| **[docs/dev/](docs/dev/)** — 开发者 | [架构设计](docs/dev/架构设计.md)（双运行时+模块地图）、[API 参考](docs/dev/api-reference.md)（+openapi.json）、[MQTT 协议](docs/dev/mqtt-protocol.md)、[插件指南](docs/dev/plugin-guide.md)、[模型接入](docs/dev/model-onboarding.md)、[数据飞轮](docs/dev/data-flywheel.md)、[术语表](docs/dev/GLOSSARY.md) |
| **[docs/releases/](docs/releases/)** — 发布 | [0.2.0 发布说明](docs/releases/release-notes-v0.2.0.md)、[风险登记册](docs/releases/risk-register.md)、[性能基线](docs/releases/perf-baseline.md) |
| **[docs/adr/](docs/adr/)** — 架构决策 | 0010 安装包策略 · 0009 对齐与 GPU · 0006 ffmpeg 视频栈 · 0003 Flutter · 0002 服务端分析 · 0001 AGPL |
| **[docs/plan/](docs/plan/)** — 项目管理 | [状态台账](docs/plan/status.md)（持续更新）、二轮审查计划归档 |
| **[docs/archive/](docs/archive/)** — 历史留存 | M0 路线图、v0.1 发布草稿、S7 验证记录、预研报告 |

## 📁 仓库结构

```
server/          Python 主运行时（FastAPI 控制面 + 推理引擎/跟踪/规则/事件/告警）
engine-rs/       Rust 工作区：数据面（ingest→infer→track→dispatch）+ 控制面 ls-server + 分析器 sidecar
web/             Vue3 Web 客户端
deploy/          部署产物：windows/（Inno Setup 安装器、WinSW 服务）+ systemd/
scripts/         构建（便携包/setup.exe）/安装/升级回滚/发布检查/压测脚本
tools/           训练/量化/标注工具链 + go2rtc 网关 + ffmpeg
docs/            guide（使用）/ dev（开发者）/ releases / adr / plan / archive
android/         Flutter 客户端骨架（暂缓）
m0-spike/        M0 技术验证记录（历史保留）
Resources/       效果演示/插件示例/数据集/分发包（见 Resources/README.md）
```

## 🔒 环境变量（前缀 LS_）

完整清单见 [server/.env.example](server/.env.example)。常用：

| 变量 | 说明 | 默认 |
|---|---|---|
| LS_BACKEND_ORDER | 推理后端降级顺序 | openvino,tensorrt,cuda,cpu |
| LS_ANALYSIS_FPS | 每路分析帧率上限 | 5.0 |
| LS_HOST | 监听地址（单一来源；非回环无令牌拒绝启动） | 127.0.0.1 |
| LS_ADMIN_TOKEN | 管理令牌（首启自动生成） | 自动 |
| LS_RETENTION_EVENTS_DAYS | 事件/截图/视频保留天数 | 180 |
| LS_FACE_ENABLED | 人脸比对开关 | **false** |

告警渠道推荐在网页端配置（保存即生效、密钥加密存储），无需改 .env。

## ⚖️ 开源协议与合规

AGPL-3.0（与 Ultralytics 生态联动决策见 [docs/adr/0001](docs/adr/0001-agpl-and-ultralytics.md)）。模型与数据集不随库分发。
人脸比对模块**默认关闭**；开启前必读部署指南合规章节。部署摄像头请设置显著提示标识，遵守 PIPL 与相关法规。

## 🛡️ 已知限制（0.2.0）

- 实验服检测 PPE 模型需在网页端确认许可后按需下载（当前资产源待维护者上传，见[发布说明](docs/releases/release-notes-v0.2.0.md)）
- Docker 部署已终止（2026-11-11）；Linux 走 systemd 原生部署
- Rust 控制面 `ls-server` 独立模式无分析 worker（分析走 Python 主运行时或 sidecar）；签名证书办理中（未签名预览版 SmartScreen 可能提示）
