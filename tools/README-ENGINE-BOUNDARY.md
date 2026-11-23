# LabSentinel tools —— 与 Rust 引擎的边界（T-334）

`tools/` 是本仓库**唯一长期保留 Python 的部分**（AI 生态依赖），职责边界：

## 保留在 Python 的原因
| 工具 | 依赖 | 说明 |
|---|---|---|
| `train_ppe.py` / `export_onnx.py` / `quantize_npu.py` | ultralytics / torch / AMD Quark | 训练生态仅 Python |
| `frame_sampler.py` / `preannotate.py` / `export_dataset.py` / `eval_report.py` | torch/cv2/datasets | 数据飞轮与评测 |
| `download_datasets.py` / `check_model_ready.py` | 标准库 | 数据准备与就绪自检 |
| `camera_soak.py` / `e2e_smoke.py` / `golden_run.py` | 标准库/httpx | 压测与回归（Python 实现参照） |

## 推理后端（T-325）
`ls-infer` 采用 **tract 纯 Rust ONNX 运行时**（feature=`tract`）与 **ort（load-dynamic）**
双后端：无需 libclang 的 tract 用于离线/CI，ort 用于发行包 GPU/CPU 推理。预处理（letterbox 114
灰边 + BGR→RGB + /255 + NCHW）与历史 Python 引擎逐位对齐（对齐结论已固化在
`testdata/vectors/*.json`），解码/NMS 复用 `post.rs` 的共享实现。
模型路径由 `LS_MODEL_FP32` 传入；缺失时 `ls-analyzer` 自动回退空检测（数据面/状态机照常）。

## 交付契约（Python → Rust 引擎，唯一接口）
模型目录（`models/` 或 `m0-spike/`）中的文件契约：

1. `<model>.onnx` 使用单张 RGB NCHW 输入。支持 YOLO 逐类置信度或带类别编号的端到端输出，也支持 Ultralytics RT-DETR 的两种输出。
2. `<model>.classes.txt` —— 类别名，一行一类，**顺序敏感**（与训练 `data.yaml names` 一致）
3. `model.json`（可选，T-334/Q-14）—— 类别角色声明：
   ```json
   {"person_class": "Person", "wearing_classes": ["Lab Coat", "Gloves", "Mask"],
    "violation_class": "No Lab Coat", "conf": 0.4, "iou": 0.5,
    "train_data_license": "CC BY 4.0", "source": "Roboflow lab-ppe"}
   ```
   Rust 引擎按**角色**取类别（不按文件数量硬编码），合并模型/级联模型共用同一代码路径。
4. `<model>.model.json` 可用 `output_format` 声明当前 ONNX 的格式。可选 `auto`、`yolo`、`rtdetr_scores`（归一化中心框及逐类置信度）、`rtdetr_topk`（归一化中心框、置信度及类别编号）。`export_onnx.py --arch rtdetr` 同时写入 ONNX 元数据与配置文件。若已有模型缺少元数据，可从「模型管理」指定实验服模型及 A/B 候选的格式。

Ultralytics RT-DETR 将图像直接缩放至模型输入尺寸，坐标分别按原图宽、高恢复。较早版本按每个查询的最高置信度分配类别，新版本由模型输出类别编号。 [Ultralytics 8.3.230](https://github.com/ultralytics/ultralytics/blob/v8.3.230/ultralytics/models/rtdetr/predict.py) · [Ultralytics 8.4.142](https://github.com/ultralytics/ultralytics/blob/v8.4.142/ultralytics/models/rtdetr/predict.py)

## 评测闭环
- `tools/eval_report` 类离线评估已退役；在线口径由 `ls-analyzer --golden` + Rust 契约测试保证。

## 禁止事项
- 运行时代码不得回调 Python（Python 服务端已退役；Rust 引擎不 import 任何 Python）。
- `tools/` 脚本不得成为生产路径的一部分（仅构建/训练/评测/压测时人工运行）。
