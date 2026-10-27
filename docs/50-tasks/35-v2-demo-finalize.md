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
