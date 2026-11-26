# 识别模型和权重

本目录保存 Rust 服务和训练工具使用的本地权重、类别表及模型元数据。打包时由资产准备脚本按锁清单检查，再将随包模型复制到发行包的 `models/`。

- 随库保存的实验服权重为 `yolo11n_ppe.onnx`、`yolo11n_ppe.classes.txt` 和 `labcoat-confirm.onnx`。
- 其余下载权重、训练检查点、校准素材和实验输出按本地资源保留。
- 早期 CPU、DirectML 和 NPU 对照脚本已归入 [历史模型实验](../tools/legacy/model-experiments/)。

模型版本和哈希见 [资产锁定清单](../scripts/assets.lock.json)，来源及许可见 [NOTICE](../NOTICE)。
