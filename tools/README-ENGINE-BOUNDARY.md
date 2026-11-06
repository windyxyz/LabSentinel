# LabSentinel tools —— 与 Rust 引擎的边界（T-334）

`tools/` 是本仓库**唯一长期保留 Python 的部分**（AI 生态依赖），职责边界：

## 保留在 Python 的原因
| 工具 | 依赖 | 说明 |
|---|---|---|
| `train_ppe.py` / `export_onnx.py` / `quantize_npu.py` | ultralytics / torch / AMD Quark | 训练生态仅 Python |
| `frame_sampler.py` / `preannotate.py` / `export_dataset.py` / `eval_report.py` | torch/cv2/datasets | 数据飞轮与评测 |
| `download_datasets.py` / `check_model_ready.py` | 标准库 | 数据准备与就绪自检 |
| `camera_soak.py` / `e2e_smoke.py` / `golden_run.py` | 标准库/httpx | 压测与回归（Python 实现参照） |

## 交付契约（Python → Rust 引擎，唯一接口）
模型目录（`models/` 或 `m0-spike/`）中是**文件契约**，Rust `ls-infer` 只认这三件：

1. `<model>.onnx` —— 输入 `1x3x640x640`（NCHW，可 dynamic batch）、输出 YOLO11 `(1, 4+C, N)`
2. `<model>.classes.txt` —— 类别名，一行一类，**顺序敏感**（与训练 `data.yaml names` 一致）
3. `model.json`（可选，T-334/Q-14）—— 类别角色声明：
   ```json
   {"person_class": "Person", "wearing_classes": ["Lab Coat", "Gloves", "Mask"],
    "violation_class": "No Lab Coat", "conf": 0.4, "iou": 0.5,
    "train_data_license": "CC BY 4.0", "source": "Roboflow lab-ppe"}
   ```
   Rust 引擎按**角色**取类别（不按文件数量硬编码），合并模型/级联模型共用同一代码路径。

## 评测闭环
- `tools/eval_report.py --engine rust`（T-334 增强项）：用 `ls-analyzer` 离线评测子命令跑固定验证集，
  保证训练侧与线上推理口径一致；Python 与 Rust 输出 mAP 偏差 > 1% 视为不通过（76 §9.7）。

## 禁止事项
- 运行时代码不得回调 Python（`server/app` 为 legacy，L4 后删除；Rust 引擎不 import 任何 Python）。
- `tools/` 脚本不得成为生产路径的一部分（仅构建/训练/评测/压测时人工运行）。
