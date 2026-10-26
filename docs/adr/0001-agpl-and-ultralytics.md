# 代码仓库采用 AGPL-3.0，检测模型用 Ultralytics YOLO 生态

---
status: accepted
date: 2026-10-25
---

Ultralytics YOLO 连模型权重都是 AGPL-3.0（闭源商用需企业授权），而 RF-DETR/YOLOX 等 Apache-2.0 替代品的训练管线需要自己拼装。LabSentinel 是单人开发的开源学习项目，AGPL 的传染性义务（网络交互也需开源）对我们几乎无感，换来的是最完整的检测/训练/导出/跟踪生态。因此：**代码仓库采用 AGPL-3.0，检测直接用 Ultralytics**。

## Considered Options

- **Apache-2.0 + RF-DETR/YOLOX**（否）：协议更宽松、且 RF-DETR 精度超 YOLO11 同级，但训练管线、文档、数据集格式生态都弱一档，单人项目时间花在拼管线上不划算。若未来需要换道，分析管线与模型解耦（ONNX 推理），迁移成本可控。
- **InsightFace 模型**（否，人脸识别另见 synthesis）：模型仅限非商业研究用途，不可随开源项目再分发。

## Consequences

- 模型权重与 Roboflow CC 数据集**不直接入仓库**：由安装脚本下载到本地并展示各自信任的协议声明。
- 想闭源商用本项目代码的人被 AGPL 挡住——对本开源项目是保护而非损失。
