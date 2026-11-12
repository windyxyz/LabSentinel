# ADR-0009：黄金语料「逐位一致」代价与 GPU / 模型来源恢复计划

- 状态：已接受（2026-11-09，R2-303 / R2-004）
- 相关：T-328（方案 A）、T-321（黄金语料）、N-20、N-09、docs/plan/78 号审阅 §9.1

## 背景

T-328 方案 A 为了让 Python 与 Rust 检测「逐位一致」，把 Windows 推理从
`onnxruntime-directml`（GPU/DML EP）统一成 `onnxruntime==1.31.1` 的 CPU EP；
Rust 默认后端 tract 也是纯 CPU。对齐目标达成（L1 轨迹 322/322 帧逐位一致），
但产品在 Windows 上失去了 GPU 加速，多路吞吐明显下降（N-20），且当时没有
性能基线数据（T-307 未做）来量化代价。

同时 `m0-spike/yolo11n.onnx` 被 `.gitignore` 排除、release 流水线在干净检出时
缺模型（N-09）。R2-004 让 CI 通过 `scripts/fetch_assets.py` 下载
Ultralytics 官方 release 的 `yolo11n.onnx`（sha256 `634279b4…`）——它与本地
自行导出的模型（sha256 `4758511e…`）**不是同一个文件**。

## 决策

### 1. 验收标准从「逐位一致」改为分层容差

- 同后端（Python-ORT CPU ↔ Rust-ort 同版本）：保留逐位一致作为**附加回归门禁**
  （`docs/archive/验证记录-S7-L1黄金语料对齐.md` 的 322/322 用例继续跑）。
- 跨后端 / 跨模型：检测框 IoU ≥ 0.99 且 |Δconf| ≤ 0.02（同后端）/ ≤ 0.05
  （跨后端）；事件一致率 ≥ 95%（track-agnostic）。
- 这样 DML、CUDA、CoreML、OpenVINO 都可以纳入对齐验收，不必为了位级一致
  牺牲用户可见能力。

### 2. Windows 便携包提供两个推理变体

- `cpu`（默认）：`requirements-win.lock`，onnxruntime 1.31.1 CPU，兼容性最好。
- `dml`（可选）：`requirements-win-dml.in` / `.lock`，`onnxruntime-directml==1.24.4`
  （PyPI 上最新稳定；无 1.31 wheel）。与 CPU 变体共用同一份 `requirements.in`
  的非推理依赖，但**不**同时安装两个 onnxruntime 发行版（B-03 同款冲突）。
- `build_portable.py --variant cpu|dml` 产出两个 zip；首启/设置页探测
  `ort.get_available_providers()` 含 `DmlExecutionProvider` 才显示「GPU 加速：开启」。
- 默认值 0.2.0 仍是 CPU；R2-306 压测数据出来后，若 DML 在主流核显上稳定，
  1.0 再考虑改默认。

### 3. Rust 侧 ort 为默认、tract 兜底

- `LS_INFER_BACKEND=ort` + `ORT_DYLIB_PATH` 指向随包 `onnxruntime.dll`
  （DML 变体携带 `DirectML.dll` 并启用 DML EP）；tract 仅作为无动态库环境的兜底。

### 4. 模型来源与差异

- CI 干净检出的便携包使用官方 release 导出的 `yolo11n.onnx`（640 输入 / 80 类
  COCO，哈希锁定）。开发机上的黄金语料、契约测试仍以本地自行导出的模型为基准。
- **已知差异**：官方 ONNX 与本地导出不是同一文件，权重/后处理可能有细微差异；
  按 §1 的跨后端容差验收，不要求位级一致。
- **恢复计划（需维护者操作）**：把本地导出的模型上传到项目自建 assets release
  （当前 `Windy-LabSentinel/assets` 的 `ppe-v1` 等相关 URL 实测 404，Q-08 按需下载
  同样受影响），把 `assets.lock.json` 的 `yolo11n.onnx` URL 与 sha256 改为该资产，
  并在 `docs/archive/验证记录-S7-L1黄金语料对齐.md` 追加一次两种模型的分层容差验收记录。
  完成前，正式 Release 的发布说明必须写明「CPU 变体使用官方 ONNX，与开发期
  黄金语料模型不同，按分层容差验收」。

## 代价

- 两套锁 + 两个 zip，构建矩阵变复杂（由 `build_portable.py --variant` 单参数控制）。
- 分层容差比逐位一致弱：需要 R2-308 把黄金语料扩到 ≥10 段场景来补覆盖。
- DML 变体的 onnxruntime 版本（1.24.4）与 Rust ort 2.0.0-rc.15 加载的
  `onnxruntime.dll`（1.31.1）不同，DML 变体下 Python/Rust 不做同后端逐位对齐。

## 性能门槛（R2-306）

- Windows CPU 变体：4 路 720p@5fps 每路 ≥ 4.5fps。
- DML 变体：8 路 720p@5fps 每路 ≥ 4.5fps，事件端到端 P95 ≤ 1.0s。
- Rust-ort 与 Python 同后端：吞吐 ≥ 1.5×，RSS ≤ Python 的 60%。
- 未达标前不推荐用户把 `analyzer` 切到 rust（1.2.0 「实验性 rust 分析器」门槛）。
