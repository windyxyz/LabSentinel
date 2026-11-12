# tools/ · 训练/量化可复现工具链（任务 06）

LAB PPE 模型的完整复现流水线：**下载数据集 → 微调训练 → 导出 ONNX → NPU 量化**。
任务规格见历史任务包（已清理），
M0 实测背景见 [../docs/40-roadmap.md](../docs/40-roadmap.md) 的「实测环境与注意事项」。

## 一、两个 Python 环境（勿重建、勿改包版本）

| venv | 路径 | 版本 | 用途 |
|---|---|---|---|
| **m0-venv** | `D:\LabSentinel\m0-venv\Scripts\python.exe` | Python 3.14 | 数据下载、训练、导出 ONNX（ultralytics 8.4.188 / roboflow / torch CPU / onnxruntime-directml） |
| **npu-venv** | `D:\LabSentinel\npu-venv\Scripts\python.exe` | Python 3.12 | 仅量化（amd-quark 0.12 / onnxruntime-vitisai 1.23.2 / numpy 钉 1.26.4） |

快照：`m0-spike/npu-venv-freeze.txt`。**改 venv 包版本是禁忌**（会破坏现有 server 推理与量化环境）。

## 一.5 摄像头联调工具（任务 56）

| 脚本 | 用途 |
|---|---|
| `list_cameras.py` | 枚举 DirectShow 摄像头索引/分辨率/fps；`--snapshot-dir` 保存首帧确认画面 |
| `virtual_cam_feed.py` | 用 pyvirtualcam 把演示视频喂给 OBS Virtual Camera（`pip install pyvirtualcam`），供 `type=usb` 联调 |

用法与注意事项见 [../docs/guide/部署指南.md](../docs/guide/部署指南.md) §3.1 与 [../docs/64-usb-virtual-camera-report.md](../docs/64-usb-virtual-camera-report.md)。

## 二、流水线四步

```bash
PY=D:/LabSentinel/m0-venv/Scripts/python.exe
NPY=D:/LabSentinel/npu-venv/Scripts/python.exe
cd /d/User/Desktop/TOOLS/监控识别   # 路径都按项目根解析，从哪里跑都行

# 1) 下载数据集（key 走环境变量 RF_API_KEY，网络走本地代理）
export RF_API_KEY=rf_xxxxxxxx      # Roboflow 免费注册: app.roboflow.com -> Settings -> API Key
HTTPS_PROXY=http://127.0.0.1:7890 $PY tools/download_datasets.py
#   -> datasets/lab-ppe/（已存在 data.yaml 则跳过）

# 2) 训练（正式 100 epochs；冒烟 --epochs 1 --name runs/ppe_smoke）
$PY tools/train_ppe.py --epochs 100 --patience 50 --name runs/ppe_v2
#   -> runs/detect/<name>/，best.pt 自动复制为 m0-spike/yolo11n_ppe.pt

# 3) 导出 FP32 ONNX（opset 13 / imgsz 640，打印 10 类别顺序并验证输入 1x3x640x640）
$PY tools/export_onnx.py
#   -> m0-spike/yolo11n_ppe.onnx（server/.env 的 LS_PPE_MODEL 指向它）

# 4) NPU XINT8 量化（必须 npu-venv；检测头 /model.23 保留 FP32；valid 集抽 20 张校准）
$NPY tools/quantize_npu.py
#   -> m0-spike/yolo11n_ppe_xint8.onnx（NPU 推理参考 m0-spike/s2c_fps_npu.py）
```

### 各脚本参数速查（均有 `--help`）

| 脚本 | 关键参数 | 说明 |
|---|---|---|
| `download_datasets.py` | `--workspace/--project/--version/--format/--out` | key 从 `RF_API_KEY` 读，缺失时报错并给设置说明；数据集已存在则跳过 |
| `train_ppe.py` | `--model --data --epochs --patience --imgsz --batch --device --workers --project --name --exist-ok` | 默认 `runs/detect/runs/ppe_v2`，与历史 `runs/detect/runs/ppe_v1` 并列，不会覆盖 |
| `export_onnx.py` | `--weights --out --imgsz --opset --data --skip-verify` | 与 data.yaml 的 names 交叉核对类别顺序 |
| `quantize_npu.py` | `--input --output --calib-dir --num-calib --exclude-prefix --mode` | `--mode XINT8`（NPU 配方，默认）或 `A8W8`（通用 INT8） |

## 三、类别顺序（10 类，data.yaml names）

**0 Glasses / 1 Gloves / 2 Lab Coat / 3 Mask / 4 No Glasses / 5 No Gloves / 6 No Lab Coat / 7 No Mask / 8 Person / 9 Shoes**

PPE 引擎按**类别名**匹配（`LS_PPE_CLASS_NAMES`），names 顺序变了引擎才会失效，因此导出脚本会打印并核对顺序。

## 四、已知坑（搬自历史任务包铁律，与本工具链相关的全部）

1. **网络**：pip 一律加 `-i https://pypi.tuna.tsinghua.edu.cn/simple`；npm 加 `--registry=https://registry.npmmirror.com`；GitHub 下载权重用镜像前缀 `https://ghfast.top/https://github.com/...`，断点续传 `curl -C -`；Roboflow/HuggingFace 等外网 API 走本地代理 `HTTPS_PROXY=http://127.0.0.1:7890`（roboflow 包自动遵循该环境变量）。
2. **onnxruntime 家族互斥**：CPU 版与 `onnxruntime-directml` 不可同装（互装会损坏）；`npu-venv` 里 CPU 版与 `onnxruntime-vitisai` 同理互杀。各 venv 已装对，**不要动**。⚠️ 实测坑：ultralytics 导出 ONNX 时会 AutoUpdate 自动 pip 装 CPU 版 onnxruntime（它不认 directml 变体）——`export_onnx.py` 已内置 `YOLO_AUTOINSTALL=false` 防御；若 venv 已被污染，恢复方法：`pip uninstall -y onnxruntime onnxruntime-directml` 后 `pip install onnxruntime-directml==1.24.4 -i https://pypi.tuna.tsinghua.edu.cn/simple`（两个一起卸再重装 directml 版）。
3. **numpy 钉死**：m0-venv numpy 不可改（二进制兼容）；npu-venv 必须钉 1.26.4（AMD wheel 按 numpy 1.x 编译），pandas 2.2.3。
4. **Quark 量化绕道**：本机无 Visual Studio MSVC，Quark 导入时要 JIT 编译 C++ 扩展会失败——`quantize_npu.py` 内置"注入绕道"（内存中剥掉 `build_custom_ops.py` 末尾 `_initialize_kernels()` 调用后手工加载模块），不修改 npu-venv 文件。正式环境建议装 VS Build Tools 走官方路径。Quark 0.13 的 download.amd.com 直链已失效（0.12 可用）。
5. **量化必须排除检测头**：`/model.23` 前缀子图（84 节点）保留 FP32——全图量化的两版（通用 QDQ、Quark XINT8 全图）类别输出全部归零；排除后精度恢复（M0 实测 NPU 40 FPS，person conf 0.80 与 FP32 同档）。
6. **Roboflow key 禁止写死**：只从 `RF_API_KEY` 环境变量读；也不要把 key 写进任何文档/命令历史。
7. **不要改历史记录**：`m0-spike/`、`docs/20-research/` 是历史实测记录，禁止改动；训练结果目录新开 `--name`，别覆盖已有 runs。
8. **服务端依赖**：`supervision` 钉 0.25.1（0.26+ 移除 ByteTrack）；服务端改 Python 依赖前先停 uvicorn（DLL 锁）。
9. **Git Bash curl 发含中文 JSON 会转码损坏**——测试 API 用 python httpx。

## 五、协议核查

| 组件 | 协议 | 说明 |
|---|---|---|
| LAB PPE 数据集（Roboflow Universe） | CC BY 4.0 | 见 `datasets/lab-ppe/README.dataset.txt`；项目开源协议 AGPL-3.0，需署名 |
| ultralytics | AGPL-3.0 | 训练工具链独立使用，服务端只用导出的 ONNX 推理 |
| roboflow（pip 包，1.7.2 实测元数据） | Apache-2.0 | 仅数据下载客户端 |
| AMD Quark（amd-quark） | MIT | 仅量化工具 |
| onnxruntime-vitisai | MIT | NPU 推理运行时 |

## 六、常见问题

- **训练报 data.yaml 不存在** → 先跑 `download_datasets.py`（需 `RF_API_KEY` + 代理）。
- **export 后类别顺序与 yaml 不一致** → 脚本会 `[warn]`，检查是否用错了起点权重；PPE 引擎按名称匹配，顺序不一致时优先修 `LS_PPE_CLASS_NAMES`/重训。
- **quantize 报 `_initialize_kernels`/C++ 编译错误** → 没走注入绕道或换动了 npu-venv；确认用 `npu-venv` 的 python 跑本目录的 `quantize_npu.py`。
- **NPU 推理验证** → 参考 `m0-spike/s2c_fps_npu.py`（onnxruntime-vitisai + VitisAIExecutionProvider，注意不同后端置信度分布有差异，生产按后端校准阈值）。

## 七、标注工具链（数据飞轮首轮，任务 11）

两个脚本配合使用：从现场视频抽帧 → 部署模型预标注 → 人工在 Roboflow 修正 → 合并进训练集（飞轮闭环）。

### 7.1 抽帧 `frame_sampler.py`

```bash
# 单视频：每 2 秒抽 1 帧，最多 300 帧；帧差去重（静态画面自动丢弃）
python tools/frame_sampler.py --video m0-spike/data/lab-walk-24076.mp4 --out datasets/frames

# 目录批量（--interval 越小抽得越密；--diff-thresh 越小保留越多）
python tools/frame_sampler.py --video-dir D:/lab-videos --out datasets/frames --interval 2.0 --max 300 --diff-thresh 8.0
```

输出 `{视频名}_{秒数:06.1f}.jpg` + `manifest.csv`（source,timestamp,frame_file,resolution）。
建议间隔：每路摄像头 1-5 秒抽 1 帧；一天录像按 5 秒抽约有 17000 帧，先按月抽少量再扩充。

### 7.2 预标注 `preannotate.py`

```bash
python tools/preannotate.py --images datasets/frames --out datasets/labels \
    --model m0-spike/yolo11n_ppe.onnx --classes-file datasets/lab-ppe/data.yaml
```

- `--classes-file` 支持一行一个类名，或直接给 `data.yaml`（自动解析 `names:` 列表）；**类名顺序必须与模型输出一致**
- 输出与图片同名的 YOLO txt（空检测写空文件）+ `preview/` 带框目检图
- 推理复用 `server/app/engine.py` 的 Engine（`Engine._run` 全局串行锁）；**禁止改成直接 sess.run**
- 跑完打印每类框数与平均置信度，用于估算人工修正工作量

### 7.3 上传 Roboflow 修正并合入训练集

1. Roboflow → Create New Project（Object Detection，贴 11 类词表）
2. Upload → 选 "Upload images with annotations" → 图片与同名 `.txt` 一起拖入（YOLO 格式自动配对）
3. Preprocessing 保持 "Resize to 640x640 - Stretch"（与现有数据集一致）→ Generate 生成版本
4. 每轮把新版本导出 YOLO 格式，与 datasets/lab-ppe、datasets/merged 一起按 `datasets/README` 的合并脚本（如被删则重建）重跑训练
5. 注意：预标注只是起点，**必须人工过一遍**（尤其 No Lab Coat 与 Lab Coat 的混淆样本）

### 7.4 评估 `eval_report.py`（飞轮④）

```bash
# v1 基线（当前 10 类模型在 merged test 集，491 图，已落盘历史模型评估记录（已清理））
python tools/eval_report.py --model m0-spike/yolo11n_ppe.onnx \
    --data datasets/merged/data.yaml --classes m0-spike/yolo11n_ppe.classes.txt \
    --label "v1" --report docs/model-eval-v1.md

# v2 训完后对比（同一 test 集再跑一次，追加到同一文件）
python tools/eval_report.py --model m0-spike/yolo11n_ppe_v2.onnx \
    --data datasets/merged/data.yaml --classes m0-spike/yolo11n_ppe_v2.classes.txt \
    --label "v2(11类,merged)" --report docs/model-eval-v1.md
```

逐类 GT/TP/FP/检出率/平均置信度表；多版本追加同一文件即成对比报告。`--limit N` 冒烟、`--split valid` 换划分。
口径：IoU≥0.5 贪心匹配；"检出率"列实为精确率 P=TP/(TP+FP)；FP 偏高 = 该类误报多。
推理走 `Engine._run`（全局串行锁），符合并发铁律。

### 7.5 版本打包 `export_dataset.py`（飞轮③→发布前置）

```bash
python tools/export_dataset.py --images datasets/frames --labels datasets/labels \
    --version 1 --note "首轮实拍 300 帧" --license "CC BY 4.0"
```

产出 `datasets/lab-ppe-real/v{N}/`（images/labels/VERSION.md，已存在版本拒绝覆盖）。
VERSION.md 含来源清单、类别统计与**脱敏确认清单**——对外发布前必须人工逐项勾完，未勾完不得发布。

### 7.6 换模型零改代码（M1.5#2 词表同步）

服务端 PPE 词表优先读模型同名文件：`yolo11n_ppe_v2.onnx → yolo11n_ppe_v2.classes.txt`
（一行一个类名，顺序=输出通道顺序，# 注释与空行忽略）；无该文件才回退 `LS_PPE_CLASS_NAMES`。
v2 上线步骤：导出 ONNX → 把 11 类词表存为 `<onnx>.classes.txt` 放同目录 → `.env` 改
`LS_PPE_MODEL` 路径 → 重启服务端（日志会打印 `[engine] ppe classes from ...`）。
v1 的词表文件 `m0-spike/yolo11n_ppe.classes.txt` 已就位。

## 八、GPU 训练环境（gpu-venv，DirectML 训练路线）

**用途**：ultralytics 训练走 AMD 核显（Radeon 880M）加速，替代纯 CPU 训练。**与推理的 DirectML 是两回事**：
推理用 onnxruntime-directml（m0-venv），训练用 torch-directml（gpu-venv），互不相干。

| 项 | 值 |
|---|---|
| venv | `D:\LabSentinel\gpu-venv`（**Python 3.12**，来自 `C:\Users\User\AppData\Local\Programs\Python\Python312`） |
| 关键包 | torch 2.4.1 + torch-directml 0.2.5.dev240914 + ultralytics 8.4.189 + opencv 5.0.0.93 |
| 设备参数 | `--device dml`（ultralytics 会调 torch_directml.device()） |

### 已知坑（2026-10-26 实测）

1. **torch-directml 只发到 cp310/cp311 wheel**（PyPI 官方源与清华镜像一致，最高 0.2.5.dev240914）——
   本机默认 Python 3.14 装不上（报 "No matching distribution"），**必须用 Python 3.12 建 venv**（机器上有 py -V:3.12）。
2. torch-directml 捆绑自己的 torch 2.4.1+cpu（约 2.5GB 下载）；不要先装别的 torch 再装它（版本会被强改）。
3. **训练时禁止并行跑 DML 推理/压测**（并发铁律同样适用于 torch-directml 训练——同为 DML 设备）。
4. 未验证项（首轮 GPU 冒烟时确认）：batch 16 @640 在 4GB 共享显存下是否 OOM（OOM 就降 batch 8）、
   torch 2.4.1 与 ultralytics 8.4.x 的训练兼容性、实际加速比（预期 1.5-2x）。

### 用法（等 CPU 训练空闲后冒烟）

```bash
D:/LabSentinel/gpu-venv/Scripts/python.exe tools/train_ppe.py \
    --data datasets/merged/data.yaml --epochs 1 --name runs/ppe_gpu_smoke --device dml
```

## 九、黄金语料归档工具（S7 验收留档，非日常流水线）

以下 4 个脚本服务于 2026-11-06 的 `docs/验证记录-S7-L1黄金语料对齐.md`
（Python ↔ Rust 检测/跟踪逐帧对齐验证），**不参与生产运行、不在 CI 中调用**，
仅保留用于复现历史验收结论。日常训练/推理/评估请用上文一至八节的工具。

| 脚本 | 用途 | 复现入口 |
|---|---|---|
| `golden_diff.py` | 比对两份 gold jsonl 事件流，输出一致率与差异明细 | 验证记录 §逐帧对齐 |
| `detect_compare.py` | Python 与 Rust 检测框 IoU / 置信度差值对比（58/58 ≥ 0.99） | 验证记录 §检测对齐 |
| `export_track_vectors.py` | 从标注语料导出跟踪向量（合成语料用） | 验证记录 §跟踪向量 |
| `export_real_track_vector.py` | 从真实视频导出跟踪向量（大语料 track-agnostic 85%） | 验证记录 §真实语料 |

> 归档口径：R2-308 计划补齐 ≥10 段场景（遮挡/夜间/断流等）时，这些脚本会重新
> 成为主力；当前保留原地，不删除、不纳入常规 README 主流程。
