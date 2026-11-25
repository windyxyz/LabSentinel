# ADR-0012：统一跨厂商 Vulkan GPU 推理路线（CV + 离线 VLM 双轨合一）

- **状态**：`Proposed`（2026-11-23 用户拍板立项；待 `LS-086` 真机 Vulkan CV Spike 实测数据补齐并经 `LS-093` 灰度验证后，于 `LS-095` 转为 `Accepted` 并同步修订 `AGENTS.md` §3.2）
- **关联 ADR**：延续并计划演进 [0009-alignment-vs-gpu-and-model-provenance.md](0009-alignment-vs-gpu-and-model-provenance.md)、[0011-rust-first-complete-refactor.md](0011-rust-first-complete-refactor.md)

## 背景与痛点

1. **DirectML 并发红线（事故 A1）与算子局限**：
   - 在 Windows AMD 驱动下，ONNX Runtime DirectML EP 严禁跨线程并发 `Session::run`（触发 Windows 整机冻结 / Kernel-Power 41），必须全仓串行加锁（`dml_lock()` + `with_memory_pattern(false)` + `with_parallel_execution(false)`）。
   - 多路实时监控叠加多模型（人检 + PPE + 人脸 + 裁剪分类）时，单锁成为吞吐与排队的长期瓶颈；且 DirectML 对部分新算子（如 DETR/Transformer Int64 索引）兼容性差。
2. **离线 VLM 托管需求**：
   - 端侧离线 VLM（`llama-server.exe` 跑 `Qwen3-VL-2B/4B`、`SmolVLM2`、`InternVL3`）在 Windows 上 **Vulkan 后端** 跨 NVIDIA / AMD（含 Radeon 核显）/ Intel 均已非常成熟，且天然与 DirectML DX12 队列隔离。
3. **跨厂商统一诉求**：
   - 用户明确决策继续坚持「GPU 默认 + 跨厂商通用 + 不做 CUDA 专属路线」，并批准将 GPU 加速长线演进目标统一至 **Vulkan**（CV 与 VLM 双轨合一）。

## 决策方案（渐进零风险切换）

1. **后端抽象先行（`v0.6.1` · `LS-080`）**：
   - 在 `ls-infer` 建立插件化 `InferBackend` trait，支持 `auto_vulkan | vulkan | dml | cpu | tract`（统一配置键 `runtime.backend`，向下兼容旧 `runtime.ep`）。
2. **统一物理设备枚举与显存协同（`v0.6.2` · `LS-083`）**：
   - 新增 `gpu_enum.rs` 动态加载 `vulkan-1.dll` 枚举物理 GPU（独显/核显、`VK_EXT_memory_budget` 可用显存，DXGI LUID 映射回退），与 `runtime.gpu_id`、`runtime.cv_vram_reserve_mb`、`llama.vulkan_device`、`llama.vram_budget_mb` 统一协同。
3. **真机 Vulkan CV 选型 Spike（`v0.7.0` · `LS-086`）**：
   - 在独立 `vulkan-spike` 特性中，于真实 Windows 物理机（至少覆盖 AMD Radeon 核显与 NVIDIA/Intel GPU）客观对比：
     - **`ncnn-vulkan`**（BSD-3-Clause，动态 `libloading` 单点 FFI + `pnnx` 导出，CNN/YOLO/分类器生态成熟）
     - **`ort` WebGPU / Dawn EP**（over Vulkan）
     - **`wgpu` / Burn**（纯 Rust）
   - **硬性晋级门禁**：数值与框匹配度 ≥98%、4 路并发零 `VK_ERROR_DEVICE_LOST`/TDR、单帧延迟不劣于 DirectML、发行包增量 ≤15MB。
4. **分步上线与永久兜底（`v0.8.0` → `v0.9.0`）**：
   - `v0.8.0`（`LS-092`）：离线 `llama-server.exe` 率先原生启用 Vulkan（固定短切片防 WDDM 2s TDR，强制反 NF4 量化拦截）。
   - `v0.8.5`（`LS-093`）：落地胜出 Vulkan CV 生产引擎并开放 `auto_vulkan` 灰度。
   - `v0.9.0`（`LS-095`）：经多机长稳验证后将 `runtime.backend` 默认切换为 `auto_vulkan`；**永久保留 `Vulkan → DirectML (ls-dml-worker + dml_lock) → CPU` 自动降级回退链**，保证老旧无 Vulkan 驱动机器开箱可用且永不触发事故 A1。
5. **GA 长稳认证（`v0.9.6` · `LS-100`）**：
   - 72 小时 4/9 路 Vulkan CV + Vulkan VLM 并发长稳零 TDR/零 Kernel-Power 41，外加 12 小时 DirectML 回退回归认证。

## 后果与影响

- **正面**：彻底统一 CV 与 VLM 跨厂商 GPU 底座（Windows + Linux 通用），摆脱 DirectML 单锁并发天花板与驱动脆弱性，实现 CV 与 VLM 显存预算显式协同或双卡分流。
- **代价**：若 `ncnn-vulkan` 胜出，训练工具链需支持 ONNX + `pnnx`/ncnn 双格式导出（在 `tools/export_onnx.py` 自动化，不增加用户使用门槛）；须在 `ls-infer` 单文件内严格收口动态库 FFI 并遵守 `unsafe_code = "deny"` 规范。

## 2026-11-24 Radeon 880M 实测

Windows 驱动 `32.0.32015.16001` 提供 Vulkan `1.4.359`。系统另有虚拟显示适配器。本轮使用仓库的两张 `1280×720` 验收图片和随包人员、PPE 两个模型，统一输入 `640×640`，先热身再测单路和四路。两份验收图片只用于小样本数值对照。

| 实验 | 人员单路 / 四路 | PPE 单路 / 四路 | 观察 |
| --- | ---: | ---: | --- |
| Rust 当前 DirectML | 52.25 / 60.83 fps | 94.58 / 126.44 fps | 人员 p95 为 20.55 / 70.78 ms，PPE p95 为 11.53 / 33.01 ms。DirectML 调用在既有专用线程上串行执行。 |
| Rust 动态 ncnn FP32 | 56.29 / 75.30 fps | 59.38 / 80.67 fps | 已复现 DLL 提前卸载后的退出异常。保持库映射至进程退出后，两套模型的 FP32 和 FP16 各完成单路、四路短测且进程正常退出。 |
| Rust 动态 ncnn FP16 | 77.08 / 152.92 fps | 105.85 / 177.32 fps | p95 分别为 14.43 / 39.92 ms 和 10.05 / 30.17 ms。该轮每路 12 帧，另有编译任务，数据用于候选筛选。 |
| ORT 1.32.0 + Dawn 0.6.0 | 47.56 / 47.87 fps | — | 前向与回读计时，独立于 Rust 整帧口径。严格 GPU 图初始化拒绝 CPU 节点。启用混合图后，人员模型两帧共执行 474 个 WebGPU 算子和 82 个 CPU 算子。 |
| Burn 0.23.0-pre.2 + wgpu | — | — | 两个 ONNX 都转换和链接成功。设备固定 Vulkan，两个模型均在首轮推理触发 CubeCL `MaybeVectorizedType` panic。 |

人员模型的 ncnn FP32、FP16 和 Dawn 都与同样输入的 ORT CPU 对照匹配 10/10 个框，PPE 模型的 ncnn FP32、FP16 均匹配 2/2 个框。匹配要求类别一致、IoU 至少 `0.95`、单框置信度差最多 `0.02`。FP16 人员、PPE 模型所有锚点中最大置信度差分别为 `0.01490` 和 `0.03998`。这组数据用于检查转换与推理口径，现场准确率仍需另行验收。

SDK 取 GitHub `20261020` 的 Windows VS2019 共享构建，由 Ghost Downloader 下载后核对 SHA-256。按发行 ZIP 的 Deflate 6 配方，x64 DLL 加两套 FP16 参数与权重共增加 `15,882,223` 字节，当前方案超出 `15 MiB`。生产版继续使用 DirectML 和 CPU 回退。ncnn FP16 留作首选候选，晋级仍依据包体复测、NVIDIA 和 Intel 真机以及四小时 CV 与 VLM 同卡稳定性门禁。

本地复现使用 `cargo build -p ls-infer --features vulkan-spike --example vulkan_spike --release`。`vulkan_spike.exe` 接收 `cpu`、`dml`、`ncnn`、`ncnn-fp16`、模型、每路次数以及 BGR 原始帧。ncnn 通过 `LS_NCNN_DLL` 指定 SDK DLL，`LS_NCNN_ADAPTER` 默认为 `0`，模型由 `pnnx` 从同一 ONNX 导出，参数使用 `.ncnn.param` 和 `.ncnn.bin`。`python tools/vulkan_spike.py` 提供 ORT CPU 数值对照、ncnn 全类别框比对和 Dawn Vulkan 分区计数。Dawn 使用 `--allow-dawn-cpu-ops --dawn-profile <输出前缀>` 记录混合图实际执行情况。

[ncnn Windows SDK](https://github.com/Tencent/ncnn/releases/tag/20261020) · [ORT WebGPU 执行提供器](https://onnxruntime.ai/docs/execution-providers/WebGPU-ExecutionProvider.html) · [Burn ONNX 导入](https://burn.dev/books/burn/onnx-import.html)
