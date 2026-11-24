# 实验室监控项目与候选模型对照

2026-11-24。产品基线为本地 `v0.7.5`。本轮用项目合成图片、已有验收图及公开 LAB PPE 测试图片做 CPU 推理。下载用 Ghost Downloader。本机未训练，未接入生产相机或调用外部视觉 API。

## 先查当前实验服复核

从同一个 `v0.7.5` 暂存包启动回环服务，使用独立数据库和相同的两张高位广角图。第二组复制相同的 PPE 权重与类名表，强制使用相同阈值 `wearing=0.40`、`violation=0.50`。程序按随包路径启用末级 YOLOE，按自定义模型路径绕过末级复核。第一、二组人员模型相同。第三组只替换人员模型。

| 配置 | 白天，2 人 | 夜视，1 人 |
|---|---|---|
| 随包模型与复核 | 未知 2 | 未知 1 |
| 同一 PPE 权重，绕过末级复核 | 已穿 1、未知 1 | 已穿 1 |
| YOLO26n 人员检测，随包 PPE 与复核 | 未知 2 | 未知 1 |

合成白天图含一件实验服和一件浅色开衫，夜视图中的背影穿实验服。两次绕过末级所得阳性都对应实验服。另一个独立试验中，随包 PPE 在白天左侧人工归属框上给出 `0.755` 的已穿分数。当前 `apply_coat_confirmation` 会把缺乏胜出正证据的已穿改成未知。`LS-091` 应显示末级无命中、归属冲突、反证、推理失败等原因，`LS-093` 再对照调整。此前抽测中也有浅色衣物和负类误判，要用难负例复测误报后再调整默认。见[分析配置](../../engine-rs/crates/ls-server/src/worker.rs#L779)和[末级确认逻辑](../../engine-rs/crates/ls-analyzer/src/ppe.rs#L163)。

## 现成权重的取舍

| 候选 | 已核实的证据 | 路线图安排 |
|---|---|---|
| **Ultralytics YOLO26n** | 官方 `v8.4.0` FP32 ONNX 提供 COCO 类别，权重元数据标 AGPL-3.0。在独立的 `v0.7.5` Rust 照片入口实际查到 3 人，两张检查记录均正常。18 张图的另一组 CPU 测试中，人员推理与解码 P50 从 `52.39` 降到 `43.82 ms`，P95 从 `56.35` 降到 `44.43 ms`。公开测试集 15 个已标人框，现有 YOLO11n 匹配 `12/15`，YOLO26n 匹配 `13/15`，未匹配的输出框分别为 3 和 2。 [官方权重](https://github.com/ultralytics/assets/releases/tag/v8.4.0) · [模型说明](https://docs.ultralytics.com/models/yolo26/) | 可以用作本地人员检测候选。正式启用前复核小目标、辅助类别、视频逐帧表现及包内 DirectML / Vulkan。PPE 单独验收。 |
| **SaKaNa-hxy/LabSentinel** | `994b1c1` 的公开仓库有部署配置与报告，完整文件树和 Releases 中未找到 `.pt` 或 `.onnx`。作者的 256 像素分类器在真实衣物框裁出的 `1,040` 个样本上达到 `97.99%` 均衡准确率，作者另列完整链路评估。 [模型目录](https://github.com/SaKaNa-hxy/LabSentinel/tree/994b1c1c16d481b39495b1ec0351c3fad847477f/models/laboratory_safety/weights) · [实验报告](https://github.com/SaKaNa-hxy/LabSentinel/blob/994b1c1c16d481b39495b1ec0351c3fad847477f/asset/diagnostics/coat_binary_20261113/%E5%AE%9E%E9%AA%8C%E6%9C%8D%E4%BA%8C%E5%88%86%E7%B1%BB%E8%AF%84%E4%BC%B0.md) · [Releases](https://github.com/SaKaNa-hxy/LabSentinel/releases) | 参考上身裁剪、正负得分和逐人归属。若上游公开权重，优先核实模型许可和输入定义，再跑同一机位样本。 |
| **ELYRA** | `5a03662` 的 NCNN 权重可加载。其 `Compliance-Shirt` 表示普通合规上衣，另外有裤子和鞋类别。白天浅色开衫得分 `0.913`，高于作者 `0.8` 门槛。把上衣误映射为实验服，在公开 6 个未穿样本中会误确认 4 个（探索阈值 `0.25`）。代码仓库标 Apache-2.0，实际权重导出元数据标 AGPL-3.0。 [项目说明](https://github.com/lylim0417/ELYRA) · [权重元数据](https://github.com/lylim0417/ELYRA/blob/5a03662bf2c1aebecc4c142051775959ffadec76/ELYRA-model/Custom_ALAPPE_Detection-YOLO11s/best_ncnn_model/metadata.yaml) | 退出实验服候选。仅在需要普通上衣、鞋裤等拓展场景且类别适配时重新验证。 |
| **化学实验室 YOLO11** | KFU 项目提供作者 `best.pt` 下载，训练说明含 `labcoat`、`gloves`、`goggles`、`mask`。需要核验权重许可并取得兼容的 ONNX。 [作者仓库与权重入口](https://github.com/EAlnahari/GraduationProject#2--download-the-trained-model) · [数据说明](https://www.kaggle.com/datasets/shathaalhosayyen/dataset2) | 优先关注白大褂、口罩的正证据。拿到可安全加载的导出后做实测，未检出仍按未知处理。 |
| **PP-Human / PP-LCNet、LibreYOLO** | PP-LCNet 的 `LongCoat` 是通用衣长属性。通用人员权重可做运行速度与授权对照，需要按版本核对类别和导出。 [PaddleDetection 标签表](https://github.com/PaddlePaddle/PaddleDetection/blob/release/2.9/deploy/pipeline/docs/tutorials/pphuman_attribute.md) · [LibreYOLO](https://github.com/LibreYOLO/LibreYOLO) | 备用通用检测路线。白大褂仍需对应标签及独立验证。 |
| **Chemist Eye 与 LLaVA-Phi3** | RSC 论文报告 LLaVA-Phi3 的两种问法在其单项 PPE 试验上为 `74.0%` 与 `97.5%`。作者先取 400 张按 PPE 和姿态分组，另取 900 张含一至三人场景并作画面级标签。DataCite 显示软件和实验资产两个记录均为 CC BY 4.0。XTuner 已发布 LLaVA-Phi-3-Mini GGUF。 [论文](https://doi.org/10.1039/D6DD00062B) · [软件](https://doi.org/10.5281/zenodo.19508611) · [实验资产](https://doi.org/10.5281/zenodo.19206709) · [GGUF 权重](https://huggingface.co/xtuner/llava-phi-3-mini-gguf) | 画面级 VLM 和提示词的基准。与较小的 Qwen3-VL 用同图对照后再选离线档案，逐人归属需另有标注。留待离线运行环境验收时获取具体模型和投影文件。 |
| **临床穿戴与其他公开 PPE 权重** | `clinical-attire-nano` 的作者在模型卡中撤回固定机位推荐。`Pharma Manufacturing` 页面标 CC BY 4.0，有实验服、未穿实验服、口罩及未戴口罩标签，公开页提供托管试用。BioSafety PPE、UAEU Safety Lab、Qualcomm 和 CPPE-5 仍按真实标签、来源和具体权重许可核对。 [Clinical Attire 模型卡](https://huggingface.co/resoajoe/clinical-attire-nano) · [Pharma Manufacturing](https://universe.roboflow.com/mohammad-annus/pharma-manufacturing) · [Qualcomm 模型卡](https://huggingface.co/qualcomm/PPE-Detection) · [CPPE-5 作者仓库](https://github.com/Rishit-dagli/CPPE-Dataset) | Pharma Manufacturing 先列为有正负标签的候选来源，取得可本地使用的权重再对照。其余先作为数据或专项基线。 |

小样本检测对照统一使用 640 等比缩放补边、CPU 2 线程、阈值 `0.25`、IoU ≥ `0.45` 一对一匹配，每个模型先预热 3 次，再对每张图跑 7 次，共 126 个计时样本。计时范围为 ONNX Runtime 1.32.0 推理与输出解码。公开图片与随包 PPE 使用过的 LAB PPE 数据同源，独立现场验收采用不同来源。未匹配的输出框需人工复核是否为漏标目标。ELYRA 在 18 个裁剪上的 CPU P50 为 `137.85 ms`，随包 PPE 在 18 张整帧上的 P50 为 `31.68 ms`，分别衡量逐人和逐帧成本。

## 对其他项目的取用范围

| 范围 | 适合写入的验收 |
|---|---|
| 合规恢复、组合穿戴、硬件输出 | `Sentinel_AI` 与 `SafeLABS` 对照类别映射、持续合规与按机位的穿戴组合。`VLM-Guard` 对照硬件输出与并发门控。`LS-099` 用模拟设备验证断线和恢复，由本地确定性事件驱动声光。 [Sentinel_AI](https://github.com/realxpen/Sentinel_AI) · [SafeLABS](https://github.com/Thithira-Paranawithana/SafeLABS) · [VLM-Guard](https://github.com/Seeed-Projects/VLM-Guard) |
| 干净证据、抑制与溯源 | `visionguard`、Frigate 与 AmberSight 用来对照留原图、叠框、重复误报记忆、动静态筛选。`open-nvr` 参考每条告警锁定模型与规则版本，`LS-091` 让用户可查证据在哪一步被保留或转为未知。 [visionguard](https://github.com/Viral3899/visionguard) · [Frigate](https://github.com/blakeblackshear/frigate) · [AmberSight](https://github.com/HuskerMinion/AmberSight) · [open-nvr](https://github.com/open-nvr/open-nvr) |
| 多路与留存 | `lab-monitor` 对照按可见窗口分配预览和断线重放。`Construction-Hazard-Detection` 对照有界队列与回执。`Retina` 和 Moonfire 分别提供 RTSP、零转码录像参考。`LS-097` 验证原视频连接、事件存证、投递恢复三个环节，记录大华/乐橙对应通道的编码差异。 [lab-monitor](https://github.com/kelongyan/lab-monitor) · [Construction-Hazard-Detection](https://github.com/yihong1120/Construction-Hazard-Detection) · [Retina](https://github.com/scottlamb/retina) · [Moonfire](https://github.com/scottlamb/moonfire-nvr) |
| 机位规则与 AI 复核 | 用 `industrial-inspection`、VERA、Vad-R1、CoAT 对照门控、清晰证据、可复核输出。DeepCamera 提供规则包交互参考。`LabClaw` / LabOS 用于事件类别和证据门槛参考。`LS-092/094` 保留低把握结果供人工核对，现场确定性规则持续独立工作。 [industrial-inspection](https://github.com/xbs0325/industrial-inspection) · [VERA](https://openaccess.thecvf.com/content/CVPR2025/papers/Ye_VERA_Explainable_Video_Anomaly_Detection_via_Verbalized_Learning_of_Vision-Language_CVPR_2025_paper.pdf) · [Vad-R1](https://proceedings.neurips.cc/paper_files/paper/2025/hash/abccc325c84dedf23dbe8de3f686c733-Abstract-Conference.html) · [CoAT](https://arxiv.org/abs/2512.20417) · [DeepCamera](https://github.com/SharpAI/DeepCamera) · [LabClaw](https://github.com/wu-yc/LabClaw) · [LabOS](https://arxiv.org/abs/2510.14861) |
| 关键帧与研究数据 | NVIDIA VSS、EventVAD、HolmesVAU 用来对照多帧采样。MonitorVLM 的数据来自矿业，适合参考验证方法。GlovEgo-HOI 的第一人称视角、DINO-YOLO 的工地视角、SmokeBench 的野外烟雾、低比特研究的车载视角各自独立，转入实验室之前应有固定机位对照。`arXiv_daily` 作检索索引。 [VSS](https://github.com/NVIDIA-AI-Blueprints/video-search-and-summarization) · [EventVAD](https://github.com/YihuaJerry/EventVAD) · [HolmesVAU](https://github.com/pipixin321/HolmesVAU) · [MonitorVLM 论文](https://arxiv.org/abs/2510.03666) · [训练工具 fork](https://github.com/JiangWu0826/ms-swift) · [GlovEgo-HOI](https://arxiv.org/abs/2601.09528) · [DINO-YOLO](https://arxiv.org/abs/2510.25140) · [SmokeBench](https://arxiv.org/abs/2512.11215) · [量化研究](https://arxiv.org/abs/2603.28888) · [论文索引](https://github.com/advanced-cs/arXiv_daily) |
| 后端选型与权限 | ncnn、ONNX Runtime WebGPU、Burn 和 llama.cpp 保留为性能与兼容性对照。`lab-safety-system` 用于参考密钥存放与上传隔离。MCP 的读写权限、设备作用域与媒体访问以项目自身授权测试为准。 [ncnn SDK](https://github.com/Tencent/ncnn/releases/tag/20261020) · [ORT WebGPU](https://onnxruntime.ai/docs/execution-providers/WebGPU-ExecutionProvider.html) · [Burn ONNX](https://burn.dev/books/burn/onnx-import.html) · [llama.cpp](https://github.com/ggml-org/llama.cpp) · [lab-safety-system](https://github.com/LIghtJUNction/lab-safety-system) |

`popxoq/lab-monitoring-detection` 的公开页面只见视频入口。原 GitCode 与部分 CSDN / 技术栈文章未取得可核验正文，相关数值和实现细节无需作为验收依据。通用隐患与情境风险论文保留给后续机位规则对照。 [lab-monitoring-detection](https://github.com/popxoq/lab-monitoring-detection) · [通用隐患研究](https://arxiv.org/abs/2605.23304) · [情境风险分级](https://arxiv.org/abs/2603.28901)

## 本地复核记录

忽略目录 `Resources/dist/model-evaluation/` 内有 `comparison.json`、`folder-comparison.json` 和对应的一次性验证脚本。`compare_local.py` 使用 `Resources/runs/vulkan-spike/pydeps` 内现有的 CPU 运行依赖，`check_folder.py` 使用 `Resources/dist/rust/labsentinel-server`。YOLO26n ONNX 的输入为 FP32 `[1,3,640,640]`，输出为 `[1,300,6]`，模型元数据声明 `end2end=True`。Rust `vulkan_spike.exe cpu` 另对该权重跑过单会话与四会话推理，成功加载并输出检测框。

| 下载文件 | 字节数 | SHA-256 |
|---|---:|---|
| `yolo26-v8.4.0/yolo26n.onnx` | 9,941,957 | `2e947b787d9e787b93a16772a5f55b1d4d8c4d86f53146149c5d6a642442d6f7` |
| `elyra-5a03662/model.ncnn.param` | 22,848 | `a531cf03aaaeb976b97aa6e628d23b48420ac535ca341d9ab279df6189983008` |
| `elyra-5a03662/model.ncnn.bin` | 37,828,840 | `6750d05b4d256f8134b7df0e39201321288b68d0818709f4620cf073cf36de8e` |
| `elyra-5a03662/metadata.yaml` | 522 | `a2005c008ad0038af751165d64b5bda9a0e27ae6a76f6ceb3a5af712e32bd904` |
