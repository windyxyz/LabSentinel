<div align="center">

# 🧪 LabSentinel · 实验室哨兵

**开源实验室安全行为智能监控系统**：接入现有摄像头（RTSP）、USB/视频文件，或外部软件持续截图的图片文件夹，自动识别进入安全区未穿实验服、统计人数，异常实时多渠道告警。全本地部署，隐私可控。

![Release](https://img.shields.io/badge/Release-0.2.0-blue) ![Rust](https://img.shields.io/badge/Rust-单二进制-dea584) ![Vue](https://img.shields.io/badge/Vue-3-42b883) ![License](https://img.shields.io/badge/License-AGPL--3.0-red) ![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux-lightgrey)

</div>

## ✨ 功能一览（30 秒了解）

- 🔍 **白大褂检测**：进入安全区自动判定穿着状态（证据分级判定，不把“看不清”当“违规”误报）
- 🖼️ **图片文件夹自动识别**：外部截图软件持续写入目录 → 自动识别关键图片（有人/违规）并按摄像头/时间汇报，原始截图只读
- 👥 **人数统计**：安全区在场人数统计与超限告警
- 🚨 **事件闭环**：告警 → 确认/误报/忽略 → 误报样本自动进标注池反哺模型
- 📢 **多渠道告警**：企业微信 / 钉钉 / 飞书 / 微信（Server酱·PushPlus）/ QQ（Qmsg）/ 邮件，失败重试+死信可查；网页端保存即生效，密钥加密存储
- 🖥️ **实时画面**：WebRTC 端到端约 1 秒延迟（go2rtc 网关），令牌鉴权子协议
- 📊 **统计报表**：日/小时趋势、CSV / PDF 月报导出（安全汇报直接用）
- 🎥 **视频存证**：事件前后各 2 秒 H.264 短视频 + 证据包 ZIP 导出
- 🤖 **模型模块化**：包内 ONNX 本地模型 / 用户自带 ONNX / 外部联网视觉模型（OpenAI 兼容、Ollama、Gemini）按摄像头切换
- 👤 **人脸比对**（默认关闭）：CompreFace 集成，合规红线齐备
- 🎛️ **双主题 UI**：WinUI 3 设计令牌，浅色/深色随系统，首次打开有配置向导

## 🖼️ 界面预览

| 仪表盘 | 事件查询与处置 |
|---|---|
| ![仪表盘](docs/assets/dashboard.png) | ![事件查询与详情抽屉](docs/assets/events-drawer.png) |
| **安全区配置** | **统计报表** |
| ![安全区配置](docs/assets/zones.png) | ![统计报表](docs/assets/reports.png) |

> 更多截图见 [docs/assets/](docs/assets/README.md)（含人脸管理页）。

## 🚀 快速开始

### 方式 A：发布包（推荐，免安装）

下载 **`labsentinel-rust-<版本>-<日期>.zip`**（[Releases](https://github.com/windyxyz/LabSentinel/releases)）→
解压到可写目录 → 双击 **`LabSentinel.bat`** → 浏览器自动打开控制台。

- 包内自带模型、ffmpeg、onnxruntime 与前端，**不需要安装 Python / Node**，运行无需联网
- 数据默认在 `%LOCALAPPDATA%\LabSentinel\data`；换端口/局域网访问见
  [安装与卸载](docs/guide/安装与卸载.md)
- 升级/回滚：`labsentinel-server.exe --archive .` / `--rollback .`（详见同上文档）

### 方式 B：源码运行（开发模式）

```bash
# 1) 前端
cd web
npm ci --registry=https://registry.npmmirror.com
npm run build

# 2) 资产（模型/ffmpeg/onnxruntime；已存在则跳过；需要 Python 3.12 作为构建工具）
python scripts/prepare_rust_assets.py

# 3) 服务端（Rust）
cd engine-rs
cargo run -p ls-server -- --launch
# 或打包：python scripts/build_rust.py --strict --zip
```

开发时前端热更新：`cd web && npm run dev`（Vite 代理到 8000），服务端照常用
`cargo run -p ls-server`；用 `LS_WEB_DIST=web/dist` 让 Rust 直接托管构建产物。

> 仓库内的 Python 仅用于**训练/标注工具链（`tools/`）与验收驱动（`scripts/*.py`）**，
> 不是运行时依赖。原 Python 服务端已在 ADR-0011 过渡期结束后退役。

## 🧭 文档索引（docs/ 已按用途分组）

| 分组 | 内容 |
|---|---|
| **[docs/guide/](docs/guide/)** — 使用与部署 | [安装与卸载](docs/guide/安装与卸载.md)、[部署指南](docs/guide/部署指南.md)、[运维手册](docs/guide/运维手册.md)、[摄像头接入 Runbook](docs/guide/摄像头接入Runbook.md)、[图片文件夹接入](docs/guide/图片文件夹接入.md)、[部署 FAQ](docs/guide/部署FAQ.md)、[杀毒软件误报说明](docs/guide/杀毒软件误报说明.md) |
| **[docs/dev/](docs/dev/)** — 开发者 | [架构设计](docs/dev/架构设计.md)、[API 参考](docs/dev/api-reference.md)（+openapi.json）、[MQTT 协议](docs/dev/mqtt-protocol.md)、[插件指南](docs/dev/plugin-guide.md)、[模型接入](docs/dev/model-onboarding.md)、[数据飞轮](docs/dev/data-flywheel.md)、[术语表](docs/dev/GLOSSARY.md) |
| **[docs/releases/](docs/releases/)** — 发布 | [0.2.0 发布说明](docs/releases/release-notes-v0.2.0.md)、[风险登记册](docs/releases/risk-register.md)、[性能基线](docs/releases/perf-baseline.md) |
| **[docs/adr/](docs/adr/)** — 架构决策 | 0011 Rust-first 全面重构（含退役记录）· 0010 安装包策略 · 0009 对齐与 GPU · 0006 ffmpeg 视频栈 · 0003 Flutter · 0002 服务端分析 · 0001 AGPL |
| **[docs/plan/](docs/plan/)** — 项目管理 | [状态台账](docs/plan/status.md)（持续更新） |
| **[docs/archive/](docs/archive/)** — 历史留存 | M0 路线图、v0.1 发布草稿、S7 验证记录、预研报告 |

## 📁 仓库结构

```
engine-rs/       Rust 工作区：控制面（axum 单二进制）+ 分析/跟踪/告警/存储/取流
web/             Vue3 Web 客户端（构建产物由 Rust 直接托管）
scripts/         构建（Rust 单包）/资产准备/冒烟/端到端验收（Python 仅作开发工具）
tools/           训练/量化/标注工具链 + go2rtc 网关 + ffmpeg（开发资产）
deploy/          部署参考：systemd / Windows 服务
docs/            guide（使用）/ dev（开发者）/ releases / adr / plan / archive
android/         Flutter 客户端骨架（暂缓）
m0-spike/        M0 技术验证记录与模型资产（历史保留）
Resources/       效果演示/插件示例/数据集/构建产物（见 Resources/README.md）
```

## 🔒 环境变量（前缀 LS_）

常用项（全部可省略，网页端能配的优先在网页配）：

| 变量 | 说明 | 默认 |
|---|---|---|
| LS_PORT | 监听端口 | 8000 |
| LS_HOST | 监听地址（非回环必须配 `LS_ADMIN_TOKEN`） | 127.0.0.1 |
| LS_ADMIN_TOKEN | 管理令牌（局域网部署必设） | 空（本机免鉴权） |
| LS_DATA_DIR / LS_DB_PATH | 数据目录 / 数据库路径 | `%LOCALAPPDATA%\LabSentinel\data` |
| LS_MODEL_PATH | 本地检测模型（ONNX） | 包内 `models/yolo11n.onnx`（由启动器注入） |
| LS_PPE_MODEL | 实验服 PPE 模型 | 包内 `models/yolo11n_ppe.onnx`（存在时） |
| LS_VLM_URL / LS_VLM_KEY / LS_VLM_MODEL | 外部联网视觉模型（OpenAI 兼容端点） | 未配置 |
| LS_ANALYSIS_FPS | 每路分析帧率上限 | 5.0 |
| LS_FACE_ENABLED | 人脸比对开关 | **false** |

告警渠道推荐在网页端配置（保存即生效、密钥加密存储），无需改环境变量。

## ⚖️ 开源协议与合规

AGPL-3.0（与 Ultralytics 生态联动决策见 [docs/adr/0001](docs/adr/0001-agpl-and-ultralytics.md)）。模型与数据集不随库分发。
人脸比对模块**默认关闭**；开启前必读部署指南合规章节。部署摄像头请设置显著提示标识，遵守 PIPL 与相关法规。

## 🛡️ 已知限制

- 未签名预览版：SmartScreen 可能提示「未知发布者」，请先校验 `SHA256SUMS-rust.txt`
- USB 设备自动枚举暂未实现（可手动填写设备索引；RTSP/视频文件/图片文件夹不受影响）
- 外部联网模型的质量取决于所选服务商；离线环境请使用包内/自带的 ONNX 模型
- Linux 以源码构建运行（systemd 示例见 `deploy/systemd/`），发布包目前面向 Windows
