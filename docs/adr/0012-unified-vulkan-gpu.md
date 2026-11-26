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
   - `v0.9.0`（`LS-095`）：复用稳定性证据，验证跨厂商短测、产品性能和回退后默认使用 `auto_vulkan`。保留 `Vulkan → DirectML (ls-dml-worker + dml_lock) → CPU`，让无 Vulkan 驱动的设备仍可运行。
5. **GA 组合回归（`v0.9.6` · `LS-100`，Vulkan 主后端）**
   - 复用 AMD 880M 同卡四小时结果。以 4/9 路 Vulkan CV 与 Vulkan VLM 短测和 DirectML 回退用例核对崩溃、TDR、Kernel-Power 41、性能和恢复。

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

## 2026-11-25 Radeon 880M 原生 DLL 复测

- 驱动 `32.0.32029.2010`。Rust 直接加载与上轮相同的 Windows VS2019 ncnn SDK。探针从该 DLL 查询实际 Vulkan 设备数，复现 `LS_NCNN_ADAPTER=999` 时拒绝执行。上游会在设备索引无效时尝试默认设备，因此探针在创建模型前核验索引。见 [ncnn 设备选择实现](https://github.com/Tencent/ncnn/blob/20261020/src/net.cpp#L1454-L1463)。
- 用 Rust 产品后处理按全部类别匹配框。FP16 权重与 FP16 算术同时启用会让 PPE 人员框横坐标不稳定。首次 PPE 为 1/2 匹配，错误框整体移到画面外，复测偏移出现在另一帧。分别把权重或算术改为 FP32 的短次对照均未出现该偏移。FP16 权重加 FP16 存储、FP32 算术作为下一轮验证配置。模型来源和类别表照旧。
- 同一组 4 张样图重复 3 次，CPU 与候选做逐类、逐帧一对一匹配，要求 IoU ≥0.95、置信度差 ≤0.02。人员 60/60，PPE 24/24，无额外框。每路计时 80 帧。样图只用于后端结果对照。

| 模型 | DirectML 单路 / 四路 | Vulkan 单路 / 四路 | Vulkan p95 单路 / 四路 |
| --- | ---: | ---: | ---: |
| 人员 | 53.94 / 56.01 FPS | 90.10 / 168.58 FPS | 11.79 / 26.91 ms |
| PPE | 88.18 / 114.80 FPS | 109.11 / 187.32 FPS | 10.37 / 24.40 ms |

- 原始五个文件用最高级标准 Deflate 压到 `15,453,639` 字节。将 DLL 无损压成 XZ 后与四个模型文件一起装入标准 ZIP，探针包为 `13,421,960` 字节，相比原包少 `2,460,263` 字节，也低于按十进制计算的 `15,000,000` 字节上限。重新打包的 SHA-256 一致，解包后五个文件与原件逐字节一致，篡改模型参数的对照包被拒绝。该结果覆盖探针资产，发行包集成和增量须在灰度打包时重新核对。
- `tools/vulkan_spike_bundle.py pack <原始文件目录> <探针 ZIP>` 与 `unpack <探针 ZIP> <新目录>` 可生成、还原并核对清单中的 SHA-256。`tools/vulkan_spike_compare.py --help` 给出原生三后端对照命令，候选传入 `--candidate-mode ncnn-fp16-storage`。报告记录运行库、权重、输入帧的哈希以及设备编号。AMD 本轮报告位于 `Resources/runs/Vulkan 推理后端对照/zip-size-comparison/`。
- `tools/vulkan_spike_coexist.py` 用同一 AMD Radeon 880M 运行四路人员检测和 llama.cpp Vulkan 的 SmolVLM-256M。四路完成 2,400 帧，164.65 FPS，p95 27.65 ms。两次本地图片问答的请求与四路计时区间分别重叠 456 和 457 ms。探针校对双方设备名及 llama.cpp 的 GPU 层加载日志。详见 `radeon-880m-cv-vlm-synchronized/report.json`。

### 四小时同卡探针

在同卡脚本的现有参数后增加 `--soak-seconds 14400 --fps-per-lane 5 --request-interval 120 --ppe-model <PPE.ncnn>`。四个 worker 各加载人员与 PPE 模型，按固定时间槽对样帧推理。每路记录实际帧数、跳过的时间槽、超出时间槽的帧数。控制器持续把 VLM 请求写入 `requests.jsonl`，Rust 每分钟将每路进度写入 `cv.log`。结束时检查运行时长、各路频率、实际重叠的请求和本次测量窗口的 Windows `Display 4101`、`Kernel-Power 41`。`report.json` 的 `passed` 字段以全部检查通过为准。输出目录内的 `stop.requested` 可触发取消，取消会退出子进程。

18 秒实机冒烟每路 90 帧，合计 360 帧，零跳槽和晚帧，四次 VLM 请求均重叠，双模型合计 p95 64.11 ms。取消后 CV 与 VLM 子进程均退出。四小时同卡探针通过，四路各 72,000 帧，未跳过任何一帧，120 次视觉请求均与 CV 重叠，未记录图形重置或设备丢失。双模型 P95 75.66 ms。报告在 `radeon-880m-soak-4h-2026-11-25/`。后续用发布包短测对照端到端延迟。

### 灰度包资产与 Rust 装载入口

`scripts/build_rust.py --strict --zip --vulkan-probe <已锁定的探针 ZIP>` 在独立 Windows x64 staging 目录放置已校验的模型和 DLL XZ 包。锁文件同时固定原始 ONNX、PPE 类别表、探针 ZIP、归档成员和展开后的 DLL，随附 ncnn 许可。用同一已构建的主程序对照完整组包，普通 ZIP 为 `141,226,931` 字节，灰度探针 ZIP 为 `154,652,214` 字节，实际增加 `13,425,283` 字节。公共成员逐字节相同，损坏探针和不匹配类别表在写入 staging 前被拒绝。

Rust 的 `vulkan-cv` 针对随包人员与 PPE 模型使用 FP16 存储和 FP32 算术。`vulkan_spike.exe --verify-package <安装根> <缓存目录>` 已在中文路径验证冷、热缓存，并拒绝改动后的 DLL、转换参数和 PPE 类别表。

独立灰度 ZIP 启用主程序的 `vulkan-cv` 编译特性。网页使用 `runtime.backend` 选择 `auto_vulkan`，继续兼容原有 `runtime.ep`。`runtime.gpu_id=auto` 使用 ncnn 默认显卡，`vulkan:<n>` 指向 ncnn 枚举编号。设备列表优先提供唯一 `uuid:<32 位十六进制>`，并兼容 `luid:<16 位十六进制>`。ncnn 在自己的 Vulkan 实例上按选定身份查找设备。DirectML 通过当次 Windows LUID 与 DXGI 对应。缺少匹配或存在歧义时按已配置的 DirectML 选择与 CPU 回退。UUID 可恢复普通系统重启后的设备选择，硬件拓扑或异常驱动变动后可能需要重新选择。实时相机、视频和图片文件夹分别报告人员模型、PPE 模型的实际后端。模型加载或推理失败时进入 ONNX 路径。自定义和辅助模型沿用现有 ONNX 配置。

托管本机视觉由 llama.cpp 列出 `VulkanN` 和设备可用显存。自动档优先选择能满足请求层数的设备，再比较是否可以与检测分卡以及可用预算。只有在显卡能被唯一识别，且与已选的 CV 及备用 DirectML 设备都能确认分离时，才省去该卡上的 CV 保留量。身份不明时照常保留。显存按模型大小、投影及运行空间估算；启动后网页显示所选设备、估算预算、计划层数或 CPU 回退原因。

白大褂辅助复核在灰度包内使用同源 ncnn 模型。先从 `labcoat-confirm.onnx` 的 `images` 到 `/model.23/Concat_3_output_0` 提取 `[1,8,8400]` 原始预测头，再用固定 pnnx SDK 及 `[1,3,640,640]` 输入形状导出 FP16 资产。`tools/build_vulkan_coat_head.py` 用原 ONNX 和导出的 `.param`、`.bin` 生成随二进制嵌入的 `coat-head.xz` 及锁定哈希。Rust 从经过校验的原 ONNX 和嵌入描述还原 `.bin`，写入前验证哈希，随后执行原图的两级 TopK 选框。原 ONNX 用于 DirectML 与 CPU 回退。其余自定义和辅助模型沿用现有 ONNX 配置。

灰度包可以用 `scripts/smoke_rust.py --zip <灰度 ZIP> --vulkan-validation active` 核对模型、图片留痕和手动切回 CPU。用 `fallback` 复现设备无法使用后的自动回退。短时多路产品链路可用现有 Rust `windows_rtsp_benchmark`，配置 `LS_BENCH_EP=auto_vulkan`，在报告中核对各路实际后端。
