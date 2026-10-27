# 任务 35 · v2 效果演示收尾与 v1/v2 对比（先读本目录 README.md）

> 背景：v2 模型已接入（`m0-spike/yolo11n_ppe.onnx`，11 类），两段 `效果演示/*_v2.mp4` 已于 10-27 生成，但任务 27 一直没有正式收尾：缺可复现命令、缺 v1/v2 对比报告、看板未勾销。

## 目标

1. **`效果演示/make_demo.py` 增加 `--device` 参数**（`directml` 默认 / `cpu`）：
   - person 引擎用 `Engine(..., prefer=args.device)`；
   - PPE 引擎 `PpeEngine` 不支持 prefer，需在构造前设置 `settings.backend_order = args.device`（`from app.config import settings`）；
   - 帮助文本注明：CPU 模式用于避免与其它推理抢 GPU，速度慢但结果一致。
2. **重新生成/验证 v2 演示**（优先 CPU，避免 DML 并发风险）：
   ```bat
   cd /d D:\User\Desktop\TOOLS\监控识别
   D:\LabSentinel\m0-venv\Scripts\python.exe 效果演示\make_demo.py --device cpu --suffix _v2
   ```
   产物：`效果演示/lab-检测效果_v2.mp4`、`效果演示/实验有风险，防护需穿戴-检测效果_v2.mp4`。
   若已有文件可正常播放且与新命令结果一致，可只做校验并说明（不要重复覆盖也行）。
3. **v1 对比**（可选，若耗时可控）：用备份模型生成 v1 版本供对照：
   ```bat
   ... make_demo.py --device cpu --ppe m0-spike\yolo11n_ppe_v1.onnx --suffix _v1
   ```
   若 CPU 处理第二段长视频超过 10 分钟，可只生成第一段或引用历史结论，不强行跑完。
4. **写 `docs/53-v2-demo-report.md`**：
   - 产物清单（文件/大小/时长/编码）、复现命令与耗时；
   - 3 组关键帧对比（v1 漏框/误判 vs v2 框出 Lab Coat / No Lab Coat），可用 ffmpeg 抽帧存 `docs/assets/v2-compare/`（3–6 张 PNG，标注时间点）；
   - 引用 `docs/52-model-eval.md` 的 test 集指标（v1→v2）；
   - 已知局限：远景小目标仍可能把白大褂判成 No Lab Coat / 证据不足（与事件详情「不确定」一致）。
5. **看板收尾**：在 `调度看板.md` 把旧任务 27 标记完成（指向本任务与报告）。

## 约束

- 不启动服务端/不占 8000/5173；不训练；不改 `m0-spike/` 下模型与视频（只读）；
- 同一时间只跑一个重任务；`--device cpu` 优先；
- 只改 `效果演示/make_demo.py`、新增 docs/截图；不动主线专属样式文件。

## 验收标准

- `make_demo.py --help` 出现 `--device`；
- 两段 v2 mp4 可播放（h264，≥10s；长视频≥30s），文件非 0 字节；
- `docs/53-v2-demo-report.md` 含复现命令、对比帧、指标与局限；
- 看板任务 27 已勾销；执行报告写明实际命令与耗时。

## 执行报告

（待填：改了哪些文件 / 实际命令与耗时 / 产物清单 / 对比结论 / 遗留问题）

### 执行报告（flash，2026-10-28 00:22 交付）

- **改动文件**：`效果演示/make_demo.py`（新增 `--device {directml,cpu}`；person 引擎 `prefer=args.device`，PPE 引擎经 `settings.backend_order=args.device` 切换，帮助文本注明 CPU 用途）；新增 `docs/53-v2-demo-report.md`、`docs/assets/v2-compare/` 4 张对比图；重生成/转码 `效果演示/` 两段 v2 mp4 + 对比用 v1 mp4。
- **实际命令与耗时**：见 53 报告 §2。v2 短视频 CPU 生成 ≈90s（508 帧），v1 同 ≈90s，x264 转码 ≈10s，逐帧差异扫描（0.5s×2 源×2 模型）≈3min。
- **产物清单**：lab-检测效果_v2.mp4（h264/16.95s/4.2MB）、实验有风险…_v2.mp4（h264/64.23s/8.5MB）、lab-检测效果_v1.mp4（mpeg4/16.95s/12.7MB，对比素材）、v2-compare/ 4 张 PNG。
- **对比结论**：v2 核心收益=误报收敛（lab-equip 蓝大褂场景 v1 误报 8 个时间点、v2 零误报；与 test 集 No Lab Coat P 11%→77% 一致）；检出一致时 v2 置信度更高（0.79→0.91）；单帧存在 lab coat/no lab coat 摇摆（walk 4.3–5.8s），服务端事件层聚合可吸收。
- **过程要点**：发现旧 lab 文件实为 lab-equip 源（stem 截断覆盖）、lab-walk 仅 7.84s 不满足 ≥10s、长视频源已缺失——均已写入 53 报告 §1 勘误与 §6 遗留建议（L1–L4）。
- **遗留问题**：L1 stem 截断修复（涉及资产命名，留主线决策）；L2 演示绘制类别扩展；L3 长视频补源；L4 抽样 jpg 不随发布。
- **合规自查**：全程 CPU 推理（--device cpu）未与其他任务并发；未占 8000/5173；未改 m0-spike/ 与主线专属文件；无新依赖。
