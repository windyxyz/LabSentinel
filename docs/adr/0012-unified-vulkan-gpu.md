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
5. **GA 长稳认证（`v1.0.0` · `LS-100`）**：
   - 72 小时 4/9 路 Vulkan CV + Vulkan VLM 并发长稳零 TDR/零 Kernel-Power 41，外加 12 小时 DirectML 回退回归认证。

## 后果与影响

- **正面**：彻底统一 CV 与 VLM 跨厂商 GPU 底座（Windows + Linux 通用），摆脱 DirectML 单锁并发天花板与驱动脆弱性，实现 CV 与 VLM 显存预算显式协同或双卡分流。
- **代价**：若 `ncnn-vulkan` 胜出，训练工具链需支持 ONNX + `pnnx`/ncnn 双格式导出（在 `tools/export_onnx.py` 自动化，不增加用户使用门槛）；须在 `ls-infer` 单文件内严格收口动态库 FFI 并遵守 `unsafe_code = "deny"` 规范。
