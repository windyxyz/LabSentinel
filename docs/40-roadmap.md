# 40 · 路线图与 M0 技术验证清单

> v0.1（2026-10-25）· 里程碑总表见 [10-requirements.md](./10-requirements.md)；本文档是 M0 的执行手册

## M0 目标

用最小代价回答三个问题：①这台机器（CPU/GPU）能跑多快的检测；②白大褂检测在现成数据集模型上的真实效果；③RTSP→浏览器 WebRTC 观看的延迟。**产出是一张实测表和 go/no-go 结论**，代码全部是一次性 spike 脚本，不进正式仓库结构。

## 前置准备

- [ ] P1 环境：Python 3.11+ venv（Windows）；`pip install ultralytics opencv-python`（GPU 存在则装 CUDA 版 torch，否则 CPU 版）
- [ ] P2 视频源：优先 1 路 RTSP IP 摄像头（子码流地址）；没有的话：手机装"IP 摄像头"类 App 获取 RTSP 地址，或用 2–3 段实验室场景视频文件代替
- [ ] P3 工具：下载 go2rtc Windows 版单文件可执行

## Spike 清单（每个都是独立小脚本）

### S1 取流与解码
- [ ] `cv2.VideoCapture("rtsp://...")` 连续读帧 60s，记录：实际帧率、CPU 占用、断流重连表现
- [ ] 对比主码流(1080p) vs 子码流(720p/480p) 的解码开销

### S2 检测帧率（关键）
- [ ] YOLO11n 与 YOLO11s，COCO `person` 类，分别在 CPU / GPU（如有）跑 S1 的流
- [ ] 记录 FPS；**通过线：CPU ≥ 5 FPS、GPU ≥ 15 FPS（720p 子码流）**
- [ ] 结论回填 10-requirements 的"模型规模"指标

### S3 白大褂模型可行性
- [ ] 从 Roboflow Universe 下载 LAB PPE / Lab Coat 预训练模型（如 [LAB PPE](https://universe.roboflow.com/gavriels-workspace-e9azv/lab-ppe-p4qe4)，注册免费账号可导出）
- [ ] 跑几段实验室视频，目测：能否分清"穿了/没穿实验服"的人；逆光、侧身、白墙背景的表现
- [ ] 通过标准：典型场景可用即算过（M1 前会用自建数据微调）

### S4 区域判定
- [ ] 用 `cv2.pointPolygonTest` 做目标底边中点与多边形安全区的包含判定，配一张静态图画多边形自测

### S5 WebRTC 观看
- [ ] go2rtc 配置 RTSP 源，浏览器打开页面看实时画面
- [ ] 记录端到端延迟；**通过线：≤ 2s**

### S6 最小闭环
- [ ] 单文件脚本（≤200 行）：取流 → person 检测 → 进入安全区且无 labcoat → 控制台打印事件 + 存截图
- [ ] 这就是 M1 的种子代码，验证事件链路的逻辑骨架

## M0 退出条件

| 项 | 标准 | 结果（2026-10-25 实测） |
|---|---|---|
| CPU 帧率 | ≥5 FPS @720p | ✅ **yolo11n 32–39 FPS，yolo11s 22–23 FPS**（首次运行有冷启动假象 11.6，复跑稳定） |
| GPU 帧率 | ≥15 FPS @720p | ✅ **DirectML @ Radeon 880M：64 FPS（p50 15.4ms）**，约为 CPU 的 2 倍 |
| WebRTC 延迟 | ≤2s | ⏳ 未测——需真实 RTSP 源，随首路摄像头接入时验证（S5） |
| 白大褂检测 | 典型场景可分穿/未穿 | ◐ 人员检测达标（主目标 conf 0.82–0.9，100% 帧覆盖）；**实验服模型待 Roboflow 账号导出**（免费注册即可，导出脚本已就绪） |
| Windows 部署 | 上述全部在 Windows 完成 | ✅ 全部在 Windows + Git Bash 完成 |

### 实测环境与注意事项

- 机器：AMD Ryzen AI 9 365（12 核）+ Radeon 880M 核显 + 31GB 内存；Python 3.14.3，venv 在 `D:\LabSentinel\m0-venv`（`D:\Program Files` 无写权限，改用 D 盘根下目录）。
- **NPU（XDNA2）实测——最终跑通（2026-10-25）**：硬件在位（NPU Compute Accelerator Device + 驱动 OK）。**最终成绩：XINT8 量化（检测头保留 FP32）后 NPU 实测 40.0 FPS（p50 24.8ms），人员检测正常（均 2.36 人/帧，conf 0.80 与 FP32 同档）**，日志确认 aiecompiler 真实编译并在 AIE 阵列执行。
  - 三档对比（yolo11n @640）：**GPU DirectML 64 FPS > NPU XINT8 40 FPS > CPU FP32 32–39 FPS**。NPU 跑赢 CPU、输给核显，但对小模型而言 NPU 的价值是**把 CPU 和 GPU 同时解放出来**（多路并发时：分析走 NPU、解码/WebRTC 走 GPU、Web 走 CPU）。
  - 量化配方（关键工程结论）：**必须排除检测头**（`/model.23` 子图 84 个节点）——全图量化的两版（通用 QDQ、Quark XINT8）类别输出全部归零；排除后精度恢复。Quark XINT8（pypi.amd.com 的 onnxruntime-vitisai + download.amd.com 的 amd_quark 0.12 zip）。
  - **本机绕道方案（未装 MSVC 时可用）**：Quark 导入时要 JIT 编译 C++ 扩展，本机无 Visual Studio——把 `quark/onnx/operators/custom_ops/build_custom_ops.py` 末尾的 `_initialize_kernels()` 调用剥掉后手工注入模块即可量化（m0-spike 里有完整代码）。正式环境建议装 VS Build Tools 走官方路径。
  - venv：`D:\LabSentinel\npu-venv`（py3.12），快照 `m0-spike/npu-venv-freeze.txt`。
  - **M1 推理架构定论**：三档后端统一走 **ONNX Runtime**（providers 自动降级：DirectML → VitisAI → CPU），训练用 Ultralytics、部署用 ORT——与 ADR-0001 一致并细化。量化版模型注意：不同后端置信度分布略有差异，生产需按后端校准阈值。
  - **环境坑（NPU venv）**：`onnxruntime`（CPU 版）与 `onnxruntime-vitisai` 同装互杀；`numpy` 必须钉 1.26.4（AMD wheel 按 numpy 1.x 编译）；pandas 需 2.2.3（兼容 numpy1）；pytorch.org 官方源本网络不可用（用 PyPI 装 CPU torch）；Quark 0.13 的 download.amd.com 直链已失效（0.12 可用，需断点续传）。
- **坑 1**：GitHub 下载模型权重 SSL 失败（国内网络），需 curl 手动下载或镜像；安装脚本要内置镜像回退逻辑。
- **坑 2**：`onnxruntime`（CPU）与 `onnxruntime-directml` 同装会互相残杀，Dockerfile/requirements 必须只装其一。
- **发现 1**：简易 IOU 跟踪器 ID 频繁跳变导致事件重复（15 个进入事件中大量为同一人）——**M1 必须用 ByteTrack**（Ultralytics 内置 `model.track`）。
- **发现 2**：背景/倒影会产生低置信度小框误检（0.3–0.44），生产需"置信度阈值 + 最小框面积"双重过滤（事件判定 conf 建议 ≥0.4）。
- **结论**：默认推理配置定为 **yolo11n @ 640 + ONNX Runtime DirectML，CPU 回退**；一台这样的机器按"每路分析 5 FPS"估算，GPU 档可支撑 8–12 路，CPU 档 4–6 路（多路实测在 M3 验证）。

任一项不达标的应对：CPU 不够 → 分析降 480p 子码流/降帧抽样（每 2 帧分析 1 帧）；延迟超标 → 换 MediaMTX 试 WebRTC/WHIP；模型效果差 → M1 前集中做数据集微调（本来就是计划内）。

## M1 概要（M0 过关后展开细化）

服务端（Python/FastAPI）：流接入模块 → 分析管线（S6 骨架产品化）→ 事件存储（SQLite 起步）→ 告警通道（应用内 WebSocket + 企微 webhook + SMTP）。Web 前端（Vue3）：摄像头管理、安全区绘制、实时画面、事件流与检索。打包：Docker Compose + Windows 原生降级路径。

## M1 实施记录（2026-10-25）

- 🎨 **Web UI 升级为 WinUI 3 视觉语言（2026-10-27）**：参考用户提供的 drcom 项目（`D:\User\Desktop\TOOLS\drcom项目`，其 `CRX/design-tokens.css` 是 WinUI 3 官方设计令牌的完整复刻）与 WinUIonWeb（Vue 控件库，令牌基准来源）：新增 `web/src/theme.css`（深浅双主题/动效曲线/圆角阴影/材质/L1-L3 语义色），重写 `style.css`（NavigationView 骨架+线性 SVG 图标、WinUI 按钮/输入三态、卡片、徽章、无障碍焦点、响应式）。浏览器实测通过（仪表盘/事件查询页截图）。协作边界已写入 50-tasks/README（theme/style/App/main 归主线）。
- ✅ 服务端垂直切片全绿：FastAPI + ORT 引擎（后端自动降级）+ ByteTrack（supervision **0.25.1**，注意 0.26+ 移除了跟踪器，勿升级）+ 规则引擎（时段/冷却/级别）+ SQLite + WebSocket/企微/邮件告警 + 截图存证。实测 4 人 4 事件（对比简易跟踪 15 条重复）。单测 6/6。
- ✅ Vue3 Web 四页面（仪表盘/摄像头/安全区画布绘制/事件查询），vite 构建通过；`npm run dev` :5173 → :8000。
- ✅ Docker Compose（Linux 容器走 CPU 版 onnxruntime）+ README。
- ✅ **实验服（PPE）管线已就位待模型**：`PpeEngine`（自定义类别）+ 进入安全区时人框↔实验服框重叠判定（wearing/no/unknown 三态）+ `LS_PPE_MODEL`/`LS_PPE_CLASS_NAMES`/`LS_PPE_CONF` 配置。当前无模型走 "unknown" 占位路径（enter_no_ppe 事件注明"检测未启用"）。
- ✅ **Roboflow 打通（2026-10-25 晚）**：key 走本地代理 7890；LAB PPE 数据集已下载 `datasets/lab-ppe/`（588 训练/40 验证图，CC BY 4.0，10 类含 Lab Coat/No Lab Coat/Person）；yolo11n 微调进行中（runs/detect/runs/ppe_v1，epoch27 mAP50=0.68）；训练完导出 ONNX 到 m0-spike 并由 .env 的 LS_PPE_MODEL 自动加载。
- 🤖 **flash 子代理任务包体系（2026-10-25 建）**：[docs/50-tasks/](./50-tasks/README.md)——8 份自包含任务规格（含环境铁律/验收标准/禁忌）；02/03/04 已由子代理执行合入（见下）。分发给子代理时把 README+目标任务两份一起粘贴。
- 🎉 **实验服检测上线（2026-10-25 深夜）**：微调 50 epoch 完成——**Lab Coat AP50=0.977 / No Lab Coat=0.808 / Person=0.995**（test split）；`m0-spike/yolo11n_ppe.onnx(+.pt)` 上线。判定采用**证据分级**：No Lab Coat 显式检测(conf≥0.30)→违规告警；Lab Coat 低阈值(0.20)→合规证据；两者皆无→"不确定"（绝不把证据不足当违规，防误报）。mixkit 域外视频置信度保守属预期，真实验收待实验室摄像头画面。
- 🚀 **数据扩充 v2（2026-10-26，重训待跑）**：真实视频评测显示白大褂/便服区分不足 → 收编 5 个公开数据集合并训练（`datasets/merged/`：**11420 图 / 26068 框**，统一 11 类词表含 HairNet）：
  - 新收编：**ENSI PPE**（8040 图，Labcoat/Person/Gloves/HairNet，CC BY 4.0）、**lcd3**（1988 图，person_with/without_labcoat 分割标注→外接框）、whitecoat（624）、aremean（180）
  - **关键坑：Roboflow 导出的 YOLO 格式可能是分割多边形（一行几十个坐标），不是检测框**——合并脚本必须两者兼容（多边形取外接矩形），否则 4/5 的框会被静默丢弃
  - v2 训练因 **CPU 争用导致死机**被暂停（训练满核 + flash 子代理 + 服务端三线并行）。**重启训练前必须**：确认子代理空闲；用限线程版：`torch.set_num_threads(6)` + batch=8 + fraction=0.5 + epochs=24 + patience=8（预计 4-6h 过夜跑）。训完：test split 评估 → 导出 ONNX → 同步 11 类词表到服务端（PpeEngine 的 names 列表 + classes.txt）→ 用户视频效果对比。
- 📋 **flash 交付审查（2026-10-26）**：任务 02/03/04 全部通过，质量优秀（详见 docs/50-tasks/审查报告-2026-10-26.md）；已派生任务 09（告警打磨）/10（部署文档）。flash 疑似正在执行任务 06（出现 ppe_smoke 产物）。
- 🔧 **M3 前置实测（2026-10-26，压测进行中）**：1 路 @5fps 达标（进程 CPU ~105%、0 异常）；**2 路暴露严重 bug——DirectML 会话并发 Run 使 worker 线程全灭**，强压时挂死 GPU 驱动导致整机硬挂起（3× Kernel-Power 41，无蓝屏/转储；详见 [51-loadtest-report.md](./51-loadtest-report.md)）。已修复：`engine.py` 全局推理串行锁（accel 后端，CPU 后端不加锁）+ worker 单帧异常自愈 + 串行性单测。修复后 2/4 路补测待整机稳定后执行（8 路按用户要求跳过）；整机侧另见报告第 3 节（BIOS/驱动/虚拟显示器）。**补测完成前多路容量结论待定；单路能力已验证。**
- ✅ **M1.5#2 词表同步 + 飞轮工具（2026-10-26）**：PpeEngine 优先读 `<model>.classes.txt` 同名词表文件（换模型零改代码，v1 词表已就位）；新增 `tools/eval_report.py`（test 集逐类评估 → Markdown 对比表）与 `tools/export_dataset.py`（数据版本打包含脱敏确认清单）；`make_demo.py` 参数化（`--ppe/--suffix`，修掉重复绘制行）。**v1 在 merged test 集（491 图）基线：Person P=32.1% / Lab Coat P=22.3% / No Lab Coat P=11.0%**（docs/52-model-eval.md）——域差距量化确认，v2 重训是刚需。v2 待用户确认过夜空闲窗口后执行（限线程参数见 60-milestones M1.5#1）。
- 🔧 已知遗留：Git Bash curl 发中文 JSON 会转码损坏（统一用 python httpx 测试）；vite dev 仅绑 IPv6 localhost；WebRTC 实时画面未做（M2，见任务包 01）；摄像头 autostart=false 解决配置竞态。
