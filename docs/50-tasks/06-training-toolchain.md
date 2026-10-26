# 任务 06 · 训练/量化可复现工具链（先读本目录 README.md）

## 背景

实验服模型 yolo11n_ppe 由一次性脚本微调而来（datasets/lab-ppe，Roboflow CC BY 4.0）。需要把"数据下载→训练→评估→导出 ONNX（含 NPU 量化）"固化成可复现脚本，让任何人（包括未来的我们）一条命令重跑。

## 目标

新建 `tools/` 目录：

1. `tools/download_datasets.py`——用 roboflow 包下载 LAB PPE（key 从环境变量 `RF_API_KEY` 读，**禁止写死**；走代理支持 `HTTPS_PROXY`；已存在则跳过）
2. `tools/train_ppe.py`——ultralytics 训练：yolo11n 起点、`datasets/lab-ppe/data.yaml`、epochs/patience 可配、结果复制 `best.pt` 到 `m0-spike/yolo11n_ppe.pt`
3. `tools/export_onnx.py`——best.pt 导出 FP32 ONNX 到 `m0-spike/yolo11n_ppe.onnx`（opset 13，imgsz 640），并打印 10 类别顺序（**类别顺序必须写进执行报告**，PPE 引擎按名称匹配依赖 data.yaml 的 names）
4. `tools/quantize_npu.py`——按 docs/40-roadmap.md M0 记录的 Quark XINT8 配方（含"剥掉 `_initialize_kernels()` 调用"的注入绕道，代码参考 roadmap 描述 + 本文件附录），用 `datasets/lab-ppe/valid/images` 抽 20 张做校准，产出 `yolo11n_ppe_xint8.onnx`；环境用 `D:\LabSentinel\npu-venv`（Python 3.12）
5. `tools/README.md`——每个脚本用法 + 各环境路径（m0-venv/npu-venv）+ 已知坑（README 铁律全部搬进来）

## 验收标准

- 三个 venv 命令均可在本机重跑成功（训练脚本允许 `--epochs 1` 冒烟）
- 导出的 ONNX 与现有 `m0-spike/yolo11n_ppe.onnx` 类别顺序一致
- 脚本有 `--help`；pytest 不受影响

## 附录：Quark 注入绕道（M0 实测有效，Py 代码骨架）

```python
import sys, types, os
import quark
src_path = os.path.join(os.path.dirname(quark.__file__), 'onnx', 'operators', 'custom_ops', 'build_custom_ops.py')
src = open(src_path, encoding='utf-8').read()
src = src[:src.rfind('_initialize_kernels()')]  # 剥掉末尾调用(保留 def)
pkg = 'quark.onnx.operators.custom_ops.build_custom_ops'
mod = types.ModuleType(pkg); mod.__file__ = src_path
mod.__package__ = 'quark.onnx.operators.custom_ops'
sys.modules[pkg] = mod
exec(compile(src, src_path, 'exec'), mod.__dict__)
import quark.onnx  # 现在可用
# QConfig(...)+ModelQuantizer 见 roadmap；exclude=检测头节点(/model.23 前缀)
```

## 执行报告（2026-10-25）

### 改动文件

**新建 `tools/`（本次全部交付物）**：

| 文件 | 说明 |
|---|---|
| `tools/download_datasets.py` | Roboflow 下载 LAB PPE → `datasets/lab-ppe/`；key 只从 `RF_API_KEY` 环境变量读（缺 key 报错并给三平台设置说明），走 `HTTPS_PROXY`，已存在（有 data.yaml）即跳过 |
| `tools/train_ppe.py` | ultralytics 训练（yolo11n 起点），epochs/patience/imgsz/batch/device/name 等全部 argparse 可配；训练完把 `best.pt` 复制到 `m0-spike/yolo11n_ppe.pt`；结果目录默认 `runs/detect/runs/ppe_v2`（与历史 ppe_v1 并列不覆盖） |
| `tools/export_onnx.py` | best.pt → FP32 ONNX（opset 13 / imgsz 640）→ `m0-spike/yolo11n_ppe.onnx`；打印类别顺序并与 data.yaml 的 names 交叉核对；用 onnxruntime 验证输入形状 |
| `tools/quantize_npu.py` | **必须 npu-venv 跑**：Quark 注入绕道 + XINT8 + 排除 `/model.23` 检测头 + valid 集等距抽 20 张校准 → `m0-spike/yolo11n_ppe_xint8.onnx`；自带量化前后节点数对比与检测头 FP32 验证 |
| `tools/README.md` | 四脚本用法、m0-venv/npu-venv 分工、铁律坑全集（含本次新踩的两个坑）、协议核查表 |

**新增模型产物**（任务要求的输出，非历史文件改动）：`m0-spike/yolo11n_ppe.pt`、`m0-spike/yolo11n_ppe.onnx`（server/.env 的 `LS_PPE_MODEL` 指向它）、`m0-spike/yolo11n_ppe_xint8.onnx`。未改动 server/、web/、m0-spike/ 既有文件、docs/20-research/。

### 每步实测结果

1. **下载**：`RF_API_KEY` 当前未设置——已验证参数解析（--workspace/--project/--version/--format/--out）、缺 key 时清晰报错（exit 1）并给设置说明、数据集已存在时打印 skip（exit 0）。真实下载需先 `export RF_API_KEY=...` + `HTTPS_PROXY=http://127.0.0.1:7890`。
2. **类别顺序（10 类，与 data.yaml names 完全一致，导出脚本已自动核对）**：
   `0 Glasses / 1 Gloves / 2 Lab Coat / 3 Mask / 4 No Glasses / 5 No Gloves / 6 No Lab Coat / 7 No Mask / 8 Person / 9 Shoes`
3. **导出**（m0-venv，用现有 `runs/detect/runs/ppe_v1/weights/best.pt` 真跑）：`m0-spike/yolo11n_ppe.onnx` 10.67 MB；onnxruntime 加载成功，输入 `images (1,3,640,640)`，输出 `(1,14,8400)`（4 box + 10 类）。
4. **训练冒烟**（m0-venv，`--epochs 1 --name runs/ppe_smoke`）：37 batch 跑完，约 2 分 20 秒，mAP50=0.216（1 epoch 冒烟水平属正常）；结果在 `runs/detect/runs/ppe_smoke/`，`best.pt` 复制到 `m0-spike/yolo11n_ppe.pt` 的路径已实测走通。冒烟后已把 `m0-spike/yolo11n_ppe.pt` 回填为 ppe_v1 正式权重（与 yolo11n_ppe.onnx 同源）。
5. **量化**（npu-venv 真跑）：注入绕道生效（剥离 `build_custom_ops.py` 的 `_initialize_kernels()` 后 quark.onnx 导入成功）；校准 `datasets/lab-ppe/valid/images` 等距抽 20/40 张；产出 `m0-spike/yolo11n_ppe_xint8.onnx` 4.51 MB。
   **节点数对比：FP32 320 节点 → XINT8 1156 节点**（量化插入量化/反量化节点）；检测头 `/model.23` 前缀节点 **84 个**（与 roadmap 记录一致），其中 25 个检测头 Conv 权重保持 FLOAT（88 个 Conv 共 63 个被量化，与 M0 的 yolo11n_xint8_hd 配方特征表完全吻合：Conv Weights INT8(63)）。
6. **--help**：四个脚本均正常（quantize_npu.py 用 npu-venv 验证）。
7. **pytest**：`server/tests` 全绿（22 passed；README 写的 6/6 是旧数字，套件已扩充，无回归）。

### 协议核查

| 组件 | 协议 | 依据 |
|---|---|---|
| LAB PPE 数据集 | CC BY 4.0 | `datasets/lab-ppe/README.dataset.txt` |
| ultralytics 8.4.188 | AGPL-3.0 | pip 元数据 |
| roboflow 1.7.2（pip 包） | **Apache-2.0** | dist-info LICENSE/Classifier |
| amd-quark 0.12 | MIT | pip 元数据 |
| onnxruntime-vitisai 1.23.2 | MIT | pip 元数据 |

### 本次踩到并已解决的坑（已写进 tools/README.md）

1. **ultralytics 导出会自动 pip 装 CPU 版 onnxruntime**（它不认 onnxruntime-directml 变体），与 m0-venv 的 directml 互斥（铁律 2）。发生即恢复：两个一起卸 → 重装 `onnxruntime-directml==1.24.4`（已照做，DmlExecutionProvider 验证正常，numpy 等其余包未动）。预防：`export_onnx.py` 已内置 `YOLO_AUTOINSTALL=false`（必须在 import ultralytics 前设置）+ 依赖预检。
2. **cv2.imread 读不了项目路径里的中文**（`监控识别`），npu-venv 下 20 张校准图全部读取失败——`quantize_npu.py` 已改为 `np.fromfile + cv2.imdecode` 兜底（与 ultralytics 的 imread 补丁同理）。

### 遗留问题

- `RF_API_KEY` 未配置，下载脚本无法对真实 API 完整实测（按任务要求验证到参数解析与报错路径）；真实重下需 key + 代理。
- 未重训正式模型：`m0-spike/yolo11n_ppe.pt/.onnx` 来自 ppe_v1（约 epoch27，mAP50≈0.68）；需要更好精度时用 `tools/train_ppe.py` 重训并重新导出/量化。
- 新量化模型的 NPU 实测 FPS 未跑（配方与 M0 已验证的 yolo11n_xint8_hd 完全一致）；验证命令参考 `m0-spike/s2c_fps_npu.py`。
- 数据集里少量训练图标注越界（ultralytics 自动忽略，如 `019_0582_020149_...jpg` 等），属数据集本身质量问题，不影响训练。
- ultralytics 导出会在 `runs/detect/runs/ppe_v1/weights/` 留一个中间产物 `best.onnx`（runs 目录为训练输出区，无碍）。
