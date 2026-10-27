# 55 · 交接快照（2026-10-27 07:58 核验）

> 本文件是**上下文压缩的落点**：新会话/新协作者读这一份即可恢复全部关键状态。
> 细节依次见：`50-tasks/调度看板.md`（协作时间线）→ `50-tasks/README.md`（任务台账）→ `52-model-eval.md`（模型评估）→ `40-roadmap.md`（实施记录）。

## 一、总体状态：M1 全部完成，v2 模型已训练完成并接入

| 维度 | 状态 |
|---|---|
| 服务端 | ✅ 运行中（:8000，DirectML 后端），进程启动 07:45 **晚于** v2 模型导出 07:26 → 已加载 v2 |
| Web 端 | ✅ 运行中（:5173，WinUI 3 主题） |
| 模型 | ✅ **v2 已上线**：`m0-spike/yolo11n_ppe.onnx`（11 类，输出 15 通道）+ `yolo11n_ppe.classes.txt`（11 类）+ `yolo11n_ppe.pt`；v1 已备份为 `*_v1.*` |
| 测试 | ✅ **78 passed**（pytest 全量，07:58 复跑） |
| 任务 | ✅ 任务 01–34 全部交付，执行报告齐全（12–34 见各任务文档） |
| Git | ⚠️ 3 次提交（基线 1bce2da + 2 次素材清理）；**27 个未提交改动待 commit** |
| 协作 | flash（用户另开会话）+ 主线巡检（cron 每 5 分钟，automation-c43e2b44）双轮询 |

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

1. **提交 Git（27 个改动）**：任务 12–34 的全部交付 + v2 模型接入文档改动 → 建议一次 `feat: 任务12-34交付 + v2 模型接入` 提交（模型文件已被 gitignore）
2. **v2 效果对比收尾**：`效果演示/lab-检测效果_v2.mp4`、`实验有风险…-v2.mp4` 已生成（07:53）→ 与 v1 版并排核对后在看板勾掉任务 27
3. **e2e_smoke 实跑**（训练已结束，负载空闲）：`python tools/e2e_smoke.py`，确认 PASS 表
4. **GPU 冒烟**：确认 DML 后端下 v2 推理正常（服务端已 directml 运行，可用文件源回放验证事件语义）
5. **真实摄像头 24h 验证**（用户侧，M1.5 出口条件）→ 通过后可宣布 v0.1.0-alpha 可发布
6. 发布前：CI 本地验证（ruff/yaml 已校验）、发布公告（docs/发布公告.md 已写）

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

- 项目根 `D:\User\Desktop\TOOLS\监控识别`；Python venv `D:\LabSentinel\m0-venv`（py3.14）；NPU venv `D:\LabSentinel\npu-venv`（py3.12）
- 镜像：pip 清华源 / npm npmmirror / GitHub ghfast / HF hf-mirror；Roboflow 走代理 7890
- 铁律：DML 禁并发 run（走 `Engine._run`）；重负载一次只跑一个；supervision==0.25.1；onnxruntime 互杀；numpy 钉 1.26.4；Git Bash curl 中文 JSON 损坏
- 服务：`uvicorn app.main:app --port 8000`（server/）；`npm run dev`（web/）；go2rtc :1984
- 巡检 cron：每 5 分钟（变化驱动，含训练看门狗；automation-c43e2b44）
