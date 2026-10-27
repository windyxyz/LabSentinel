# 55 · 交接快照（2026-10-28 核验）

> 本文件是**上下文压缩的落点**：新会话/新协作者读这一份即可恢复全部关键状态。
> 细节依次见：`50-tasks/调度看板.md`（协作时间线）→ `50-tasks/README.md`（任务台账）→ `52-model-eval.md`（模型评估）→ `40-roadmap.md`（实施记录）。

## 一、总体状态：M1 全部完成，v2 模型已训练完成并接入

| 维度 | 状态 |
|---|---|
| 服务端 | ⏸ 已停止（用户要求）；上次运行 :8000 DirectML + v2 正常，事件/截图/视频全通 |
| Web 端 | ⏸ 已停止（:5173） |
| 模型 | ✅ **v2 已上线**：`m0-spike/yolo11n_ppe.onnx`（11 类，输出 15 通道）+ `yolo11n_ppe.classes.txt` + `yolo11n_ppe.pt`；v1 备份 `*_v1.*` |
| 测试 | ✅ **78 passed**（pytest 全量）；`npm run build` 通过（gzip ~42.7KB） |
| 部署 | ✅ 新机一键部署链路完成：`scripts/deploy.py` + `一键启动.bat`；沙箱解压 demo 包到中文路径从零 install 成功（8/8），start-all 三件套健康 |
| 任务 | ✅ 01–34 完成；新增 **35（已派发）→36→37→38**（M1.5 收尾/RC） |
| Git | ✅ 工作区干净；最近提交 `e82b7c7`（部署链路）；分发包 `dist/LabSentinel-demo-20261027.zip`（94MB，已忽略） |
| 协作 | flash（用户另开会话）+ 巡检 cron（每 5 分钟，`automation-af6a6031`；**30 分钟无进展自动停派发并删定时**） |

## 二、v2 模型事实（关键）

- 训练：merged 数据集 11420 图 / 26068 框，全量 40 epoch（用户授权全核，batch16），约 19h
- 最佳：**mAP50 = 0.723 / mAP50-95 = 0.477（epoch 28）**
- test 集（491 图）v1 → v2：
  - Person 检出率 P：32.1% → **51%**
  - Lab Coat P：22.3% → **49%**
  - **No Lab Coat P：11.0% → 77%**（违规告警的关键指标）
  - HairNet（新增类）P：82%
- 提示：Person P 51% 仍偏低（merged test 含多机位/遮挡场景），真实画面验收仍是最终标准（用户侧摄像头）
- 事故记录：完成链路途中 m0-venv 的 onnxruntime 再次被污染（pybind11 1.31-cpu 与 1.24.4-directml 混装 → run() 全崩），已按标准流程修复并双 provider 验证。**教训：安装/升级任何会引入 onnxruntime 的包后，先跑一次 CPU 推理冒烟**

## 三、待办（按优先级）

1. **任务队列**（看板为准）：35 v2 演示收尾（已派发）→ 36 e2e_smoke/DML 冒烟 → 37 移动端事件表格 → 38 RC 复核
2. **真实摄像头 24h 验证**（用户侧，M1.5 出口条件/发布硬阻塞）→ 通过后才可宣布 v0.1.0-alpha 可发布
3. 发布前：CI 本地验证（ruff/yaml 已校验）、发布公告（docs/发布公告.md 已写）——由任务 38 统一复核
4. 无障碍遗留：B 动态键盘/读屏走查、C Zones/Reports 硬编码色令牌化（A 对比度已于 10-28 修复）
5. 训练与模型迭代：下一轮建议收集真实摄像头困难负样本（白大褂远景误判），见 `标注与模型迭代手册.md`

## 四、关键文档地图

| 想知道什么 | 看哪里 |
|---|---|
| 协作时间线 / 谁做了什么 | `50-tasks/调度看板.md` |
| 任务清单与验收 | `50-tasks/README.md` + 各任务文档执行报告 |
| 模型评估数字 | `52-model-eval.md` |
| 实测性能（CPU/GPU/NPU） | `40-roadmap.md` M0 段 |
| 换模型怎么接入 | `45-model-onboarding.md` |
| 部署 | `部署指南.md` + `部署FAQ.md` |
| 模型训练/迭代 | `标注与模型迭代手册.md` + `90-data-flywheel.md` |
| 发布 | `70-release-plan.md` + `发布核对报告.md` + `发布公告.md` |
| 风险与事故复盘 | `80-risk-register.md`（A1 死机事故） |
| 架构与 API | `30-architecture.md` / `35-api-reference.md` |

## 五、环境要点（新会话必读）

- 项目根 `D:\User\Desktop\TOOLS\监控识别`；**首选 venv 为项目内 `.venv`（py3.12，deploy 创建，`scripts\venv_path.txt` 存相对路径）**；兼容旧 `D:\LabSentinel\m0-venv`（py3.14）；NPU venv `D:\LabSentinel\npu-venv`（py3.12）
- 启动：双击 `一键启动.bat`（体检→自动部署→拉起→面板）或 `scripts\start-all.bat`；停止 `scripts\stop-all.bat`
- 镜像：pip 清华/阿里云/官方自动回退；npm npmmirror→官方；GitHub ghfast；HF hf-mirror；Roboflow 走代理 7890
- 铁律：DML 禁并发 run（走 `Engine._run`）；重负载一次只跑一个；supervision==0.25.1；onnxruntime 互杀；numpy 钉 1.26.4；Git Bash curl 中文 JSON 损坏
- 服务：`uvicorn app.main:app --port 8000`（server/）；`npm run dev`（web/）；go2rtc :1984
- 巡检 cron：每 5 分钟（`automation-af6a6031`，状态机 `docs/50-tasks/派发状态.json`；30 分钟无进展自动停派发并删除定时）
