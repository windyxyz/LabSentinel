# Findings — LabSentinel 全仓审阅关键发现、风险与约束备忘

> 2026-11-22 四线全仓只读审阅基线：`main @ 10e7006`（`v0.4.0`）。每条均经源码逐行核验。

## v0.9.6 推进约束

- **v0.9.6 发布签名条件**。仓库 Actions API 返回零个自托管运行器，本机证书库也无可用签名证书。为 v0.9.6 在 `main` 上提供明确选择的预览发布入口，要求原有安装、服务、卸载和播放验收全部通过。使用未签名名称和预发布标记。
- **Rust CI #83**。录制测试仅在 Windows 设置子进程标志，Ubuntu 上的共享可变绑定触发 `unused_mut`。审计日志另指向 `tokenizers 0.22.2 → paste 1.0.15` 的停更公告 `RUSTSEC-2024-0436`。上游 v0.23.2 清单仍声明 `paste`。
0.9.6 本轮继续本地正常验收并修复可复现的问题。NVIDIA、Intel、生产监控和真实签名验证留待具备对应条件时运行。开发期旧包只作已有兼容性取证。当前包用测试版本标记验证真实安装覆盖及回滚，因为回滚会过滤同版本归档。

原 AMD 4/9 路测量中，PPE 同帧最大逐路 P95 分别为 217.94/262.87 ms，整图阶段为 40.28/86.51 ms，裁剪阶段为 44.33/141.05 ms。分段 P95 来自不同帧，不能相加。1080p 指源流，解码后分析画布为 1280×720，已走 2:1 缩放。截图编码九路最大逐路 P95 为 10.67 ms，可优先评估现有 JPEG 依赖的加速。

按人裁剪的 PPE 再检允许边距，也可能包含另一人、挂在附近的衣物或仅覆盖背后区域的框。整图已有唯一人员归属门槛。裁剪再检获取的框坐标属于裁剪区域，应用相同归属门槛前需要加回裁剪起点。

发布流水线原先直接把未签名双产物上传至草稿。签名需要可信证书、私钥和 RFC 3161 时间戳。签名会改变字节，需在生成 ZIP、安装器和校验和前签服务程序，安装器构建后再签。使用专用签名运行器，使会清除完整安装数据的验收留在 GitHub 托管的隔离机器。

0.5.0 正式安装器的 SHA-256 与发布流程锁一致。旧版本在隔离数据目录自举 schema 16，加入确认、误报及两种 AI 复核记录。新程序迁移到 32 后，旧版回滚可读取同一库，默认卸载只保留数据。按正式源的精确 tag 查询未找到 0.9.5，本地版本历史和已检查目录也没有 0.9.5 安装包。当前用户和系统证书库均无代码签名证书，Windows 会话未提权。

上轮安装验收被会话中断，首份报告已证明新包升级和 40 个文件与 ZIP 一致。原卸载日志停在删除文件，恢复前先核实安装进程及注册范围，随后仅对本轮隔离目录重跑卸载。最终只留下测试数据库。

语义搜索 30 ms 门槛只计向量检索。文字编码和 HTTP 响应单独采样，后台 CPU 编码对实时分析的影响另做同期对照。机位和事件类别应在取前 K 个结果前生效，权限更新还需核对单独媒体入口。

万条向量查询基线 P95 84.7–88.6 ms。直接向量表查询约 80 ms，扩大 SQLite 页缓存没有明显收益，持久映射的诊断查询约 3–10 ms。发行实现在 Windows WAL 库的向量检索期间限制映射为 128 MiB 上限并恢复原设置，重开库实测 P95 24.9–28.7 ms。写入后未关闭库的无范围查询仍可略超 30 ms。并发命中正确及释放映射后数据库缩小均有实测，SQLite 错误和 WAL 状态继续沿用原语义。

Lab Coat Detection 3 v12 公开标签把蓝色、黑色和卡其工装也归入实验服，部分标注是衣物局部或未穿在身上的服装。按原标在两种 GPU 后端得到 8 个穿戴分歧，其中 7 个与现场白大褂口径无关，另 1 个是穿浅色大褂的逆光样本。抽取 21 张目视核对的子集后，给定人框也没有恢复白大褂分类，最终两种后端和实际候选均确认 5 个未穿，1 个已穿被判未穿，15 个未知。这组图含动作和背景相近的连拍、短袖实验服和户外逆光，不能外推为现场准确率。按全场同源环境做训练及单独留出未来验收图，模型训练留待后续。

人物裁剪放大时，相邻输出行常用到相同源行。发行实现复用未经舍入的水平定点加权和，三个典型裁剪中位数从 0.95–1.36 ms 降至 0.38–0.49 ms。原有整图缩小继续走原流程。相同环境的 RGB 归一化查表收益不稳定，沿用原运算。四路端到端短测没有证实整体加速，PPE 同帧样本和竞争波动需单列。

Windows 回滚正在运行的主程序时，会把原程序重命名为 `labsentinel-server.exe.old` 后再恢复归档程序。该备份由回滚生成，首次个人安装升级、回滚后卸载实测残留该文件。Inno 按固定文件名清理后复测，仅保留测试数据目录。

性能优化以识别结果为先。缩小实验服模型输入已改变同一组样本的分类结果，候选继续沿用 640 输入。缺乏现场真值时，固定夹具对照只证明输出一致性。打包与短测使用独立目录，GitHub 发布等待本地验收。

审计包原先只保留事件来源哈希，离线缺少对应设置原文。导出时固定原文并分别显示记录和归档哈希。联动原先仅在内存保存旧设备，发送后未确认复位若碰上退出或崩溃，重启只加载新配置。将可能已点亮的设备留在设置表，收到零级别成功回执后再清除。
原路由统计 183/183 仅计算文档路由是否由程序提供，未显示同路径缺失的方法。双向按方法、路径对账发现 48 个实现入口未登记，将其纳入 0.9.6 集中门禁。

Retina 解包保留 H.264/H.265 视频访问单元和 RTP 时间戳。通过管道内的 Matroska 帧封装保留时间戳，避免裸 H.26x 输入由解码器重猜帧率。沿用原事件缓存与 FFmpeg 解码，机位偏好放在现有设置表，避免改变冻结的数据库结构。RTSPS、协商失败、无可用关键帧和流参数变化走 FFmpeg。
Retina 0.4.22 依赖的 pretty-hex 在 0.4.3 新增 `HexConfig.autoskip`，旧结构初始化编译失败。本次锁到 0.4.1，升级依赖时重新验证。已验证同一子流以 FFmpeg 回退后继续分析，保存的 H.264/H.265 片段保留 B 帧显示时间和递增解码时间。

固定 ncnn SDK 为每次提取借用工作内存，但 GPU 命令记录会读写缓冲区的阶段和访问状态。共享原生网络时串行保护同一模型，人员与 PPE 各有独立网络。保留独立网络基线用于对照。

Vulkan 模型锁等待和 DirectML 调度等待属于应用侧排队。ncnn 同步提取包含提交和等待驱动，现有调用计时无从单独测量 GPU 内核。定位冷启动时需区分缓存生成、网络加载和网络首次成功调用。

Radeon 880M 同构建、不同隔离缓存目录的两次四路测试中，DLL 解包为 192 和 189 ms，PPE 原生模型加载为 5.615 秒和 330 ms。耗时差集中在原生加载阶段，须进一步计量内部步骤再确定原因。

固定 SDK 暴露 `ncnn_pipelinecache_create` 与 `ncnn_option_set_pipeline_cache`。缓存借给该设备的网络，实际管线复用由 ncnn 内部的锁和兼容性判定保护。缓存和 DLL 的生命周期需长于全部借用网络。

跨进程恢复编译缓存前先验证 Windows 当前账户封装、应用层 SHA-256、长度上限和普通文件属性，再由 ncnn 检查驱动及设备。无法恢复时从空缓存开始。源权重和原生 DLL 仍按静态资产锁验证。

Windows LUID 可在同次启动中跨 Vulkan 与 DXGI 对应物理显卡。保存唯一且非零的 Vulkan `deviceUUID` 来覆盖正常重启，设备重新插拔、拓扑变化或驱动异常仍可能使它变化。无匹配或存在歧义时沿用回退策略，避免按名称或序号猜测设备。

人员与 PPE 同帧总耗时包含整图、逐人裁剪和必要时的白大褂复核。PPE 按事件或采样执行，同帧 P95 的样本数少于常规分析帧，验收需要记录实际调用次数和阶段。

PPE 裁剪原生对照的 15 个候选人框包含原图和缩小摆放到灰底的夹具。固定 ncnn 输入从 640 改为 448 或 320，保持权重不变但改变 4 或 2 个实验服标签，包含穿着滑向未穿。候选暂留探针。显卡算术选项收益小且存在跨阈值的分数。
2026-11-26 用户确认识别准确率优先于时延收益。0.9.6 验收记录真实耗时与未达目标的情况，后续版本任务可独立推进。

现有 YOLOE 白大褂复核权重的最终选框子图直接由 pnnx 转换时残留未注册层，ncnn 实机拒绝加载。灰度使用原始预测头，Rust 执行原模型的两级 TopK。固定样本对照最终框，逐人标签、现场准确率和性能仍按路线图验收。

对同源权重尝试无损重建时，源 ONNX、转换描述及还原产物都须核对锁定哈希。基于已知资产生成描述，禁止在产品运行时执行第三方转换程序。

后续 GPU 稳定性回归改按用例完成。AMD 四小时报告证明本机固定探针同卡稳定性。产品短测分别记录性能、异常与恢复；长期资源增长率留作可选观察。
Vulkan 的账户级管线缓存可按需重建。存储管理只统计并清理 SHA-256 标识及显卡编号格式匹配的管线缓存。新类别使用独立的保留策略，已有缩略图自动清理规则继续只应用于缩略图。
2026-11-26 AMD 产品短测中，首次编译人员模型为 5.591 秒，恢复时为 48–52 ms，损坏缓存重建时为 246 ms。同一台机器跨进程测试仍受驱动自身的缓存影响，需用有无应用缓存的对照解释首帧时间。

托管本机视觉的 `VulkanN` 编号由随包 llama.cpp 枚举。判断是否需要为 CV 保留显存时，设备身份不明确按可能共卡处理。层数预算是启动估算，仍需保留运行时失败的 CPU 回退和状态说明。

go2rtc 1.9.15 的 FFmpeg RTSP 回调按命令散列，几路并发启动同一个命令时会争用回调。发行实例多路夹具为每路使用相同内容的独立文件路径。失败样本中第 4 路没有进入分析，产品仍正常关停。

仅运行 Rust RTSP 基准无法证明大屏历史、真实回放和服务子进程生命周期。发行实例的持续采样输出每次原始数据，短跑结果用于核对采样与断流流程，正式门槛继续按路线图 §6。

值守大屏原先在读取事件列表后用 `since=latest` 启动实时流，两个请求之间新入库的事件可能被跳过。断线游标的记录被保留策略清理时，原握手也没有告知客户端刷新可查列表。

固定的 llama.cpp CPU 运行时对同一事件截图的 WebP 载荷返回 HTTP 400，JPEG 载荷返回 HTTP 200。托管本机档案在送图边界转 JPEG，数据库与证据磁盘保持 WebP 对账。综合验收通过实际托管模型确认带图事件问答已经写入 `images` 和正确证据 ID。

MCP 路由内部测试已覆盖可见工具与授权分支。实际客户端验收需走 HTTP 协议握手，逐项检查写操作的业务效果和数据库留痕。复核用隔离截图和单次本机模拟响应。

`LS-093` 的 ncnn 设备编号以固定 SDK 的枚举为准。DXGI 编号继续用于 DirectML，网页枚举的 Vulkan 实例编号不直接转换为 ncnn 编号。逐路报告人员和 PPE 的实际后端，避免辅助模型覆盖进程级最近设备后产生误读。

原生 ncnn 调用若绕过 YOLO 的线程级调用计数，Vulkan 推理成功仍可能产生零推理统计，继而影响依赖调用计数的 PPE 定期采样。端到端统计应区分实际执行了 PPE 的帧和只执行人员检测的帧。

`LS-102` 扩展既有保留和维护 API。手动清理按预览范围执行并报告实际删除数量和字节。删除条目与媒体引用统一对账，排除原始图片文件夹、当前日志和在用事件证据。设置操作走管理鉴权和审计。

唯一移出路线图的交付项是连续录像。保留按运动和告警触发的事件片段、短时前置缓存、回放与检测并行、多路优化和值守大屏。验收无触发时的持久存储、片段配额和断流恢复。

0.9.6 保留完整 MCP、原生 Windows 服务、声光联动与审计证据包。AI 复核、带图多轮问答、报告和语义检索作为特色功能纳入集中验收。数据集链路可独立推进。Vulkan 是最终默认，晋升沿用 AMD 四小时证据，并按 `LS-086/093` 补齐跨显卡短测。项目本机仅评估与推理，GitHub Release 暂停。

`LS-096` 选择独立图像与文字的 SigLIP 2 ONNX 量化模型进行本地 CPU 验证。两份权重约 378 MB，优先按需提供。索引按编码版本隔离并验证机位授权。旧版回滚后事件删除仍须可用，处理 SQLite 扩展表时避免让旧运行时依赖新扩展。

## v0.8.0 至 v0.8.5 交付约束

品牌图使用用户提供的两张原图。网页、PWA、Windows 应用和安装器使用同一无字标识。Vulkan 灰度按 `LS-086` 真机证据决定范围，DirectML 继续串行并保留 CPU 回退。沿用本机只推理、GitHub Release 暂停的安排。

## LS-097 同类项目工程做法规划审阅

- `FrameReader` 已按来源分别使用实时最新帧和离线顺序读取，`performance.rs` 已统计覆盖帧数与帧龄。后续核对掉线恢复、文件处理和后台多路识别。[FrameReader](../../engine-rs/crates/ls-ingest/src/capture.rs)
- 当前事件详情支持截图与证据导出。`LS-097` 保留触发录像与事件回放，并补渠道回执和故障恢复。`LS-090/091` 提供画面状态与当时的规则，`LS-094` 提供模板的变更预览和冲突检查。[事件详情](../../web/src/views/Events.vue)
- 同一 FFmpeg 会话管道同时取分析帧和原编码片段。16 MiB 原编码片段预算包含待保存引用，最多保留 12 秒、1024 片段和 3 个触发预留。HEVC 片段使用 `hvc1`，浏览器缺编解码器时可以下载原始证据或查看截图。Retina 可选入口仍需验证 B 帧时序与回退。[录制模块](../../engine-rs/crates/ls-ingest/src/recording.rs)

## LS-093 模型来源与 PPE 证据对照

- `v0.7.5` 两张高位合成图的 3 人在默认配置均为未知。用相同的 PPE 权重、类名和阈值绕过末级 YOLOE 后，确认两件白大褂，浅色开衫保持未知。`LS-091` 优先显示末级复核输入、归属和分数。调整策略前补白衬衫、浅色外套与多人遮挡对照。
- YOLO26n 官方 ONNX 在 `v0.7.5` CPU 照片入口可用。公开测试图片的 15 个已标人员框，从 `12/15` 提高到 `13/15`。18 图 CPU 小样本中 P50 `52.39 → 43.82 ms`，只计推理和输出解码。ELYRA 普通上衣类把白天合成图的浅色开衫判为合规上衣，分数 `0.913`。SaKaNa 的公开仓库有实验服分类报告与配置，未见训练权重文件。
- Chemist Eye 的软件与实验资产记录均标 CC BY 4.0，按画面级基准使用。逐人归属、可见性和动作时序各有专用标注。数据按机位及原始录像分隔对照，记录正常检查、未知、错归属及真实告警。完整来源、哈希和范围见[模型对照](../plan/model-source-review-2026-11-24.md)。

## LS-076 v0.6.0 基线缺陷审查

2026-11-23 接续实施从 `LS-080` 开始。GitHub `main` 使用直连推送，后续版本暂停发布标签及 GitHub Release。本机模型工作限验证和推理。

产品源码基线 `2fef1b4`，路线图提交 `8dca517`。实测使用本地发行暂存目录的 `v0.6.0` 可执行文件、回环地址和独立临时数据库。图片为项目的 `Resources/acceptance/imgs/person-frame.jpg`，视频由随包 FFmpeg 从同一素材生成。模型测试使用 CPU。任务分配和通过条件见 [1.0.0 路线图 §1.3](../plan/v1.0.0-roadmap.md#13-ls-076-缺陷与发布验收)。

| 缺陷 | 验证与定位 |
|---|---|
| `LS-076-01` | **隔离复现**。照片检出 1 人，配置区域内 0 人，且不在规则星期范围内，仍产生 `occupancy_over`。配置 L3 写成 L2。**源码确认**。未穿实验服只判断区内检测结果，未查适用的 `enter_no_ppe` 规则。见 [worker.rs](../../engine-rs/crates/ls-server/src/worker.rs#L2094) 的照片分支。 |
| `LS-076-02` | **隔离复现**。目录运行后新增人数规则，下一张图片检查成功，告警 0 条。手动重启后下一张触发 1 条。运行中删除同一规则后再扫一张，累计告警增至 2 条。保存接口写数据库，分析任务持有启动时副本。见 [规则写入](../../engine-rs/crates/ls-server/src/handlers/zones_rules.rs#L139) 和 [分析配置](../../engine-rs/crates/ls-server/src/worker.rs#L895)。 |
| `LS-076-03` | **隔离复现**。同一段 3 秒视频，随包人员模型在 15 帧均检出人员。给测试进程指定输入可加载、输出与人员检测不兼容的 ONNX 后，日志有 15 次推理失败，状态仍为 `completed`，检查事件显示处理 15 帧且 0 帧检出人员。见 [detector.rs](../../engine-rs/crates/ls-analyzer/src/detector.rs#L92) 和 [worker.rs](../../engine-rs/crates/ls-server/src/worker.rs#L1464)。 |
| `LS-076-04` | **隔离复现**。在临时库记录一个已丢失的 PPE 文件路径，状态接口仍返回 `source=bundled`、`active=true`。新来源完成了一张照片的人员检测，日志未出现 PPE 加载，对照来源有 PPE 模型加载记录。见 [状态选择](../../engine-rs/crates/ls-server/src/handlers/misc_status.rs#L1381) 与 [推理配置](../../engine-rs/crates/ls-server/src/worker.rs#L555)。 |
| `LS-076-05` | **隔离复现**。用临时投递行模拟发送中断，重启后旧 `pending` 仍挂起，手动重试返回 HTTP 404。**源码确认**。后台重试只查 `retry` 和 `failed`，事件落库先于独立建投递行，进程中断可能留下无投递记录的事件。见 [事件入库](../../engine-rs/crates/ls-server/src/worker.rs#L150)、[投递写入](../../engine-rs/crates/ls-jobs/src/lib.rs#L507) 和 [重试查询](../../engine-rs/crates/ls-store/src/lib.rs#L1369)。 |
| `LS-076-06` | **隔离复现**。5 条正常检查、3 条待处置告警、0 条已处置告警。首页接口返回待处置 8，报表按总数减待处置显示已处置 5。见 [首页统计](../../engine-rs/crates/ls-server/src/handlers/misc_status.rs#L46)、[报表聚合](../../engine-rs/crates/ls-server/src/handlers/stats.rs#L213) 和 [已处置卡片](../../web/src/views/ReportsView.vue#L184)。 |
| `LS-076-07` | **隔离复现**。按后台复核的字段写入一条 `likely_false` 样本。事件接口仍返回空，且从疑似误报筛选消失、混入未复核。**源码确认**。报表 AI 小计为常量。见 [后台回写](../../engine-rs/crates/ls-server/src/genai.rs#L142)、[事件输出](../../engine-rs/crates/ls-server/src/handlers/events.rs#L74)、[筛选查询](../../engine-rs/crates/ls-store/src/lib.rs#L904) 和 [报表小计](../../engine-rs/crates/ls-server/src/handlers/stats.rs#L333)。 |
| `LS-076-08` | **脚本与 ZIP 核验**。给准备函数传入临时目录的无效 `yolo11n.onnx`，函数返回并显示「使用已准备的模型」。构建代码对该文件允许仅凭存在就打包。此次暂存目录和本地 ZIP 中的实际人员权重均匹配转换后的锁定哈希与大小。见 [准备脚本](../../scripts/prepare_rust_assets.py#L65) 和 [构建脚本](../../scripts/build_rust.py#L89)。 |
| `LS-076-09` | **源码与产物核验**。Inno 安装器全用户模式提供安装为 Windows 服务，代码寻找 `tools/winsw.exe`。安装器复制的暂存目录及 ZIP 均无该文件和服务配置。验收采用目录清单，交互安装列入后续验证。见 [任务选项](../../deploy/windows/installer.iss#L91) 和 [执行分支](../../deploy/windows/installer.iss#L226)。 |

## LS-080 裁剪候选第一轮验证

- PaddleDetection 官方 PP-LCNet 人员属性标签包含通用长外套 `LongCoat`。可供服装特征参考。直接用于实验服判定缺少对应类别。 [官方标签表](https://github.com/PaddlePaddle/PaddleDetection/blob/release/2.9/docs/advanced_tutorials/customization/pphuman_attribute.md)
- 从本地 LAB PPE 测试集的标注框切出 10 个实验服与 7 个未穿样本，把裁剪边长限制为 256 后输入现有 640 像素 YOLOE。仅确认 1/10 实验服，另有 1/7 未穿样本误确认，中位 CPU 推理 116.13 ms。源图裁剪边长上限 640 的对照为 0/10 正确确认、1/7 误确认、107.48 ms。以上为小样本推理对照，256 指裁剪采样，模型输入仍为 640。该裁剪方案留在实验范围。
- 另将本地 `models/yolo11n_ppe.pt` 转为原生 256 ONNX（opset 17、IR 8），采用上身与长衣双窗，仅做导出与推理。对 14 个能唯一匹配实验服标注的人框，随包 640 模型单次全帧判定 12 个正确，原生 256 对 10 个正确，两者其余均为未知。Python ORT 的单次全帧与逐人双窗中位耗时为 143.68 ms、41.30 ms，运行环境不同于发行包。`v0.6.2` 继续采用已验收的随包组合，原生 256 候选留在实验目录。后续以新候选覆盖同一组难例后的结果决定启用。

## LS-083 实机验证范围

- Windows 真机使用随包 ONNX Runtime，两个加载请求命中同一个 DirectML 模型会话。关掉其中一个引用后，余下引用仍能完成推理。使用三种优先级并发提交的纯调度验证确认任务都在同一命名线程执行。
- 当前 Vulkan 验证覆盖显卡列举与显存预算查询。Vulkan CV 推理选型、现场多路长跑及各 GPU 厂商对照归入 `LS-086` 和后续现场回归。
- 新事件证据采用纯 Rust WebP 编码。截图格式、文件摘要、原图叠框、缩略图和派发用 JPEG 分别按真实字节核对。标注池和旧事件继续读取原有图片格式。

## v0.7.0 集中回归审阅

- 清理任务对 SQLite 旧库按实际表存在性处理抑制存证和事件段。清理预览与实际清理共享开放事件段保护条件。分别用旧库和有抑制截图的新库验证。
- 使用当前 `since=latest` 实时流约定更新前端子协议测试。系统路由测试引用 SQLite 当前 schema 常量，避免迁移版本固定在 14 时持续误报。
- 真实 Radeon 880M 的候选比较、包体差额与残余硬件门禁见 [ADR-0012](../adr/0012-unified-vulkan-gpu.md#2026-11-25-radeon-880m-原生-dll-复测)。PPE 的 FP16 权重加 FP16 算术组合出现过非确定性坐标偏移，后续候选采用 FP16 存储、FP32 算术。原生对照做逐类一对一勾稽，设备索引须经实际加载的 DLL 核验。

## LS-087 专项规则边界

- 穿戴策略按机位和区域生效。缺少可靠视觉证据时保留未知；连续合规需满足时间与帧数。基准线按机位标定，拉门、门状态和液体风险应可由明确画面证据复核。

## LS-084 违规事件段边界

- 连续段使用实际观测连续性。PPE 未知、模型错误及暂时遮挡都缺少恢复证据。历史规则默认保持零宽限。
- 本地开发使用项目素材和隔离实例。现网配置、录像与外部 AI 凭据留在现有环境。

## LS-088 本地与 API 协作

- 2026-11-24 用户更正复核额度按人员与时间窗口计，不按违规名称或事件段计。已确认身份可以贯通断轨和机位，无可靠身份的轨迹互不合并。相同违规类型可以出现在不同人身上。
- API 精测使用项目验收图片。正面图的目标穿白大褂、口鼻无遮挡且未见口罩，回环返回口罩违规。另一张只看见侧后方，口罩判定为不确定。同步核对请求只含对应人物截图及事件不会跨人回填。
- 原有近景的拍摄角度和目标距离与现场高位广角差别较大。新增合成验收图需覆盖遮挡、普通浅色外套、灰度夜视和小目标。能看见设备门缝时才可报告门开，画面不足时保留未知。
- 两张 `Resources/acceptance/imgs/lab-*-cctv.jpg` 为生成的虚构验收画面。白天前景人员穿白大褂且口鼻无遮挡，远端人员穿普通浅色开衫并戴口罩，椅背白大褂没有人。夜视图能确认设备门与搁架可见，背向人员的口罩与手套状态不可确认。素材适合验证链路和记录可见性差异，真实机位精度需现场测试。
- 视觉视频检查从原来的最多 8 帧减到最多 3 帧，输入帧上限减少 62.5%。使用已有 SHA-256 实现冻结帧去重，也用于截图和提示词溯源。后续问答附图数量为 0。计费随模型、分辨率和文本输入变化，本机精测不采集云端计费数据。
- 0.7.5 回归审阅发现指定复核档案更新可能只重载跟随全局的机位；固定场景失败后需要恢复重试；未带事件编号的历史追问需要继承上轮事件，显式换事件须隔离历史原图结论。
- 0.7.5 最终便携包 CPU 实测。白天和夜视两张图检出 2 人及 1 人，已穿和明确未穿白大褂计数均为 0。该视角下随包佩戴模型的判定不足，现场泛化仍按路线图 `LS-093`、`LS-100` 重新测量。
- 原实时复核按机位冷却，每次从证据环取多帧。事件升级和人数灰区会重复触发，且启用旧后台 AI 时可能双送。协作改为复用持久化事件和可见轨迹。区域进出、人数超限和单人留守由本地规则判定。人数灰区按轨迹只探测一次，同帧的新事件共用一张图，独立事件保留授权和每日预算。复核失败或看不清时保留本地告警。
- [Frigate 对象复核](https://docs.frigate.video/configuration/genai/genai_objects/) 依据轨迹生命周期触发并支持区域筛选和单截图。静态穿戴使用一帧，确需动作顺序的规则另行使用受限多帧。
- [Scrypted 检测链](https://docs.scrypted.app/scrypted-nvr/troubleshooting.html#detection-flow-chart) 用轻量信号决定何时运行下一层，另把已检测到的事件交给 [AI 摘要](https://docs.scrypted.app/scrypted-nvr/features.html#ai-summaries)。本项目保持本地规则先触发，AI 结果补写到原事件。

## LS-074 检测权重随包约束

- 仅采纳可核验来源、明确允许随包分发且实测具备使用场景的权重。口罩和实验服各自验证。人脸不可见或证据不足保留未知。
- 既有现场视频仅作隔离推理验证。本机不训练，不更改生产实例和监控配置。GitHub 发布继续暂停。
- 近景照片出现可核对增益，俯拍样本即使裁头也未达到阈值。近景补判默认关闭。两类口罩权重不能可靠判定佩戴方式，不增加相应告警。Windows 本地模型和验证集下载使用 Ghost Downloader。

## LS-075 模型合并与淘汰口径

- 同一批样本比较新增检出、误报、无法判断、推理时延和包体成本；只发布带来独立收益且许可明确的模型。缺少正类检测不能作为未穿白大褂的证据。
- 比较候选时保留已验证的基础能力及用户数据。准备同时满足局部检测和实时分析的单一 Rust 运行时。公开数据只做本地推理对照，不在本机训练。
- 旧合并权重把现场 57 个人框全部判成已穿，已从 0.6.0 包移除。专项权重能给出独立未穿证据；已穿仍须通过白大褂复核，无法归属或模型未确认时记为未知。
- AIZOO 在俯拍样本的高置信度输出不足，较低阈值把后脑和遮挡误报为未戴。Grounding DINO INT8 在同一机器约 2.15 秒／帧，量化人员权重比当前 CPU 推理慢且照片少检。对照结果不代表全现场准确率。

## 1. P0 级阻断/高危缺陷清单

| 编号 | 领域 | 位置 | 根因与影响 | 归属 |
|---|---|---|---|---|
| F-01 | 推理/A1 | `ls-infer/src/yolo.rs:185-214,270-278` | DirectML 建会话/`Session::run`/`Drop` 无全局串行锁，且未设 `with_memory_pattern(false)`/`with_parallel_execution(false)`，多路或软重启可触发事故 A1（Windows 整机冻结 / Kernel-Power 41） | `LS-041` |
| F-02 | 0.4.0 PPE | `ls-server/src/router.rs:262`、`handlers/misc_status.rs:648-733`、`worker.rs:457-463`、`build_rust.py:130` | 四重失效：① Axum 默认 2MB body limit 拦截 10MB+ `.onnx`；② `worker.rs` 未读 `app_settings["ppe_model"]`；③ 上传未生成 `ppe-custom.classes.txt` 致 `make_ppe_runner` 返回 `None`；④ `LabSentinel.bat` 强注 `LS_PPE_MODEL` 覆盖自定义模型 | `LS-041` |
| F-03 | 规则引擎 | `ls-store/src/lib.rs:671-675`、`pipeline.rs:520-542` | `Store::add_rule` 硬编码写入 `days_of_week = 0`，`weekday_allowed` 按 ISO `1..=7` 匹配恒 `false`，所有网页新建规则永久不触发 | `LS-041` |
| F-04 | 时钟/时区 | `worker.rs:778-779`、`util.rs:15`、`ls-store/src/lib.rs:1738`、`detect.rs:394`、`logger.rs:36` | ① 实时流误用离线 `frame_ts_for`（硬编码 `2026-01-01`）；② 5 处手写 UTC 函数与 `Local::now()` 撕裂 8 小时（中国 UTC+8 新事件立刻误升级、预算早 8 点重置） | `LS-041` |
| F-05 | 存储安全 | `ls-store/src/lib.rs:65-72` | 设了 `synchronous=NORMAL` 却从未开启 `PRAGMA journal_mode=WAL`，停留在默认 DELETE 模式存在断电损库与高并发 `SQLITE_BUSY` 风险 | `LS-041` |
| F-06 | VLM/安全 | `ls-server/src/vlm.rs:80-90,141-178` | ① `openai_compat` 推理漏发 `Authorization: Bearer` 且硬拼 `/v1/chat/completions`（双 `/v1` 404）；② `gemini` 缺 `/v1beta` 且 `?key=<明文>` 随错误串泄漏到日志/DB/HTTP 响应，临时 mp4 仅含 `pid` 并发互踩 | `LS-041` |
| F-07 | Cascade | `ls-server/src/worker.rs:786,821-867,1595-1671` | `cascade + on_negative=suppress` 下被冷却限流或未命中复核门的本地事件被永久吞没；复核确认放行时 `snapshot_path` 硬编码 `None` 丢截图 | `LS-041` |
| F-08 | 告警渠道 | `handlers/alerts.rs:190-211,540-573` | 网页配置的 PushPlus/Qmsg 构造时 `url` 为空串 100% 被 `check_url` 拒绝；`email.smtp_port` 以 JSON Number 提交被 `raw.as_str()` 静默丢弃 | `LS-041` |
| F-09 | 前端契约 | `ReportsView.vue`、`Events.vue`、`FolderImages.vue`、`Settings.vue`、`Zones.vue`、`Cameras.vue`、`Wall.vue`、`router.rs` | 11 处前后端契约/渲染 P0：`stats/hourly` 返回对象致报表页崩溃、周视图写错 `daily`、`folder-images` 缺 `thumb_url` 令 `mediaUrl` 崩、`events/bulk-status` 缺字段、`Settings` `refresh is not defined`、`rule-templates` 键名 422、`alert-silences` 422/`NaN`、`alerts/status` 无 `channels`、`camera-groups` 字符串数组、`/ws/webrtc/:cid` 未注册 | `LS-042` |
| F-10 | CI/打包 | `.github/workflows/{release,tests}.yml`、`deploy/windows/installer.iss`、`deploy.rs`、`engine-rs/Cargo.toml` | `release.yml` 绕过 `rust.yml`；CI Node 20 与 `vitest@5.1`（需 `>=22.12`）冲突；`installer.iss` 缺 UTF-8 BOM；升级前未停服务且 `--rollback` 自覆盖运行中 exe 必败；10 个子 crate 缺 `[lints] workspace = true` 致 `unsafe_code="deny"` 失效 | `LS-045` |

## 2. P1 级重点缺陷摘要

- **推理与 PPE（`LS-041` / `LS-043`）**：`EP_USED` 用 `OnceLock` 切换 GPU/CPU 不更新；`ppe.rs` 每帧 2+ 次整图推理 + 8400 次 `Vec` 分配 + 整图 640 缩远景 + 阈值硬编码不可调导致灰度图检出 0；`model.json` 误覆盖有效类表；`Unknown` 误发 `enter_no_ppe`；GUI 子系统吞 `println!`/`eprintln!`。
- **Worker / 采集 / 跟踪（`LS-043`）**：`folder_loop` `.take(20)` 超过 20 张图永久饥饿；`given_up` 未清理 `st.workers` 锁死重启；软重启旧线程覆写状态/删新句柄；`Instant::now() - vlm_interval` 刚开机 panic；`build_cmd`/`decode_image`/`encode_mp4_with`/`ffprobe` 漏 `CREATE_NO_WINDOW` 闪黑框；RTSP 缺 `-rw_timeout`；`alive_since` 置位过早；ByteTrack `lost_track_buffer` 双重缩放；`cooldown_s` 当帧数比较缩短 `fps` 倍；`occupancy_over` 多规则串台。
- **安全与后台任务（`LS-044`）**：`retention.rs` 相对路径按 CWD 解析致安装版 1h 后误删有效事件媒体；`/ws/events` 绕过 `trusted_hosts`；限流盲信 `X-Forwarded-For`；`ADMIN_GETS` 漏 `/api/config/export` 等；`GET /api/cameras` 未脱敏 RTSP 密码；`folder_image_file`/`serve_event_media` 缺严格 `Path::starts_with`；`vlm`/`genai`/`ls-dispatch` URL 可被 `@` 绕过且 `ls-dispatch` 未设 `max_redirects(0)`；`detect_models` 改 URL 复 key 外泄；`urldecode`/`discovery` UTF-8 切片 panic；`version_gt` 比较错误；`mqtt.rs` 断线 100% CPU；`graceful_shutdown` 未停 `jobs`/插件/`mqtt`/托盘；`alert-deliveries/:did/retry` 无消费者。
- **打包与文档（`LS-045`）**：`installer.iss` `{sys}\prefetch` 路径写错（应 `{win}\Prefetch`）；`build_rust.py` 不清 `DIST`、双份 `onnxruntime.dll` 膨胀 ~20MB；版本号五源漂移致 `.exe` 文件属性恒 `0.1.0.0`；`NOTICE` 仍列已删 Python 栈；`status.md`/`README.md` 滞后。
- **口径更正**：`POST /api/folders/{cid}/scan` 早在 RA1.3 已于 `handlers/folder_images.rs:115-145` 完整接线，当前路由覆盖为 106/106（仅 `router.rs:83` 注释未更新）。

## 3. 不可违背的路线约束（来自 `labsentinel-user-decisions.md`）

- Rust-first（Python 运行时已退役，`tools/` 与 `scripts/` 仅开发支具）；不做 Docker / VM / Flutter / Tauri / 安卓 / CUDA。
- GPU 默认开启走 DirectML（跨厂商通用，失败回落 CPU），但必须受 `DML_LOCK` 串行保护。
- 提交者统一 `windyxyz <siyuan060203@163.com>`，远端只保留 `main`；发布双产物 `zip` + Inno Setup 7 `setup.exe`；PPE 随包分发；卸载干净且默认保留用户数据。
- 中文路径坑：`CARGO_TARGET_DIR=C:\Users\User\devtools\ls-target`（纯 ASCII），`PATH` 前插 `w64devkit\bin` 与 `.cargo\bin`，配齐 `RUSTFLAGS`。

## 4. v0.5.0 本机复验发现

- ONNX Runtime rc.15 每进程只允许创建一个环境。PPE A/B 评测与多路 worker 必须复用该进程级环境。DirectML 会话创建、运行和释放继续走 `dml_lock()`。
- 随包 go2rtc 1.9.15 使用 `-c` 读取内联配置。流名采用 `cam_<id>`。使用内联配置时，流已在内存注册或注销，持久化步骤仍返回 `config file disabled`。
- 随包 FFmpeg 读取 JPEG 存证帧序列时显式指定 MJPEG 解码器，避免短图像序列探测失败。
- 诊断包按允许列表导出配置项，并保留日志时间和级别。日志正文可携带第三方端点凭证及用户输入，因此统一脱敏。
- 只读值守分享需要先启用管理令牌和读取保护。模板数据与真实现场样本应分开验证，工业 PPE 数据的类别覆盖与实验室现场不同。

## 5. LS-058 v0.5.0 正式发布复审

审查基线为 `main` 和 `v0.5.0` 共同指向的 `a992a42`。发布内容与复验记录见 `progress.md`。[GitHub Release](https://github.com/windyxyz/LabSentinel/releases/tag/v0.5.0) · [发布 CI](https://github.com/windyxyz/LabSentinel/actions/runs/45010145971)

| 编号 | 优先级 | 指向 | 影响与验收条件 |
|---|---|---|---|
| LS-058-01 | P1 | `scripts/prepare_rust_assets.py:129`、`scripts/build_rust.py:111`、`.github/workflows/release.yml:88` | 实测 GitHub 正式 ZIP 缺 `tools/go2rtc.exe`，本机同名 ZIP 有该文件。CI 只准备模型、FFmpeg 和 ORT，严格打包也允许缺 go2rtc。安装器使用同一 staging。修复后应校验 go2rtc 来源及哈希，检查双产物内容，并用正式包验证 RTSP 播放。 |
| LS-058-02 | P1 | `web/src/components/WebrtcPlayer.vue:164`、`web/src/views/Wall.vue:25`、`web/src/views/Cameras.vue:413` | 实测 go2rtc 1.9.15 拒绝当前格式的 offer。播放器发出 base64 字符串，上游同名协议要求 SDP 对象。播放器在两个视图握手失败后继续重连，快照分支没有接管。使用上游真实协议做回归，覆盖握手、首帧、失败降级和恢复。[go2rtc 1.9.15 信令实现](https://github.com/AlexxIT/go2rtc/blob/v1.9.15/internal/webrtc/webrtc.go#L127-L142) |
| LS-058-03 | P1 | `deploy/windows/installer.iss:198`、`engine-rs/crates/ls-server/src/deploy.rs:116` | 正式二进制隔离复现。先归档 `0.4.0` 返回 0，模拟复制新版后 `--rollback` 返回 2。`VERSION` 为 `0.5.0`，`current.txt` 仍为 `0.4.0`，唯一可用归档被过滤。有更旧归档时可能跳过正确的回滚点。验收需覆盖从单一前版覆盖升级并恢复。 |
| LS-058-04 | P1 | `engine-rs/crates/ls-server/src/handlers/events.rs:263`、`engine-rs/crates/ls-server/src/handlers/cameras.rs:285`、`web/src/App.vue:105` | 源码显示分享期限和吊销只在 WebSocket 握手时检查。正常页面每 15 秒复查分享状态，服务端没有在授权失效后主动关闭现有会话。验收需确认已有事件、检测数据和视频会话随吊销或到期停止。 |
| LS-058-05 | P1 | `engine-rs/crates/ls-infer/src/yolo.rs:381`、`engine-rs/crates/ls-infer/src/yolo.rs:517`、`engine-rs/crates/ls-infer/src/post.rs:50` | 源码显示 Rust 把所有末维为 6 的输出当成 YOLO 像素角点。Ultralytics RT-DETR 的六列输出可为归一化中心框，预处理也不同。统一按模型格式定义预处理和解码，使用真实导出模型检查框位置、类别和置信度。[Ultralytics RT-DETR 参考实现](https://github.com/ultralytics/ultralytics/blob/main/ultralytics/models/rtdetr/predict.py) |
| LS-058-06 | P1 | `deploy/windows/installer.iss:135`、`engine-rs/crates/ls-server/src/handlers/misc_status.rs:232` | 全用户模式默认装到 Program Files。网页保存局域网配置时写安装根 `.env`，标准用户在默认权限下无法保存。把可写配置纳入用户数据路径并兼容已有部署配置，使用标准账户验收保存、重启、恢复。 |
| LS-058-07 | P2 | `engine-rs/crates/ls-server/src/handlers/dataset.rs:407`、`tools/merge_datasets.py:208` | 正式 ZIP 中单个已核验样本导出 HTTP 200，全部进 `val`，训练集为空。Python 合并器用合成图实测 1～4 张全部进 `train`，同时仍生成指向空验证集的 YAML。明确小样本行为，并覆盖 1、2、4、5 张以及缺图场景。 |
| LS-058-08 | P2 | `web/src/views/Wall.vue:88`、`web/src/views/Wall.vue:135` | 大屏把 L1 事件数除以总事件数当作合规率。事件级别描述严重度，当前数据无法提供合规检查的总次数。例如当天只有一条 L1 提示时会显示 100%。验收需定义分子、分母和无观测时的显示。 |
| LS-058-09 | P2 | `engine-rs/crates/ls-server/src/go2rtc.rs:59`、`engine-rs/crates/ls-server/src/go2rtc.rs:190` | 30 秒是重复调用的限频，启动后缺少周期监管。子进程意外退出后，普通摄像头启动和预览路径不会重新启动它并恢复 RTSP 流表。用隔离子进程测试自动恢复和流表重建。 |
| LS-058-10 | P2 | `.github/workflows/rust.yml:71` | Linux 步骤未显式设置 shell，默认 `bash -e`。在 `python ... | tee` 中，覆盖率脚本退出 1 可能被 `tee` 的成功状态覆盖。CI 报告本次确实为 106/106。负向验收需让低于阈值时整个步骤失败。[GitHub Actions shell 语义](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#jobsjob_idstepsshell) |
| LS-058-11 | P2 | `engine-rs/crates/ls-jobs/src/lib.rs:518`、`engine-rs/crates/ls-jobs/src/lib.rs:662` | 首次发送会根据渠道和设置携带证据图，后台和手工重试只传文本。出现临时投递失败时，成功重试可能丢失现场证据。用支持图片的通道验证失败后重试的正文和截图。 |
| LS-058-12 | P2 | `scripts/acceptance_vlm.py:204`、`Resources/acceptance/imgs/empty-frame.jpg` | 验收脚本把画面中可见实验人员的图片当作空场景。正式包用该素材时 mock 调用由 2 增至 3，随即把下一张的复核挡在冷却期内。替换临时素材为合成无人图后，空场景计数保持 `2→2`，补入有人帧后 `2→3`，脚本通过。应使用符合断言的固定素材。 |

`LS-067` 实施前核对。小样本切分须在确认图片可读之后执行。事件投递历史需跨手动重置保留，并沿用管理端读取约定。合规趋势的分母采用实际可判定样本，保留未知与覆盖计数。

现场使用大华云联和乐橙。应用名称只能说明远程查看渠道，接入由录像机或摄像头的型号、固件与局域网可达性决定。现有实时摄像头入口接受 RTSP／RTSPS。纯云端设备须先获得厂商视频授权，现场设备联通与授权暂未验证。

`LS-069`。现场 v0.5.0 已登记并解码 59 张 JPG，数据库无解码错误。默认近七天按文件修改时间筛选，再加上关键图片条件，页面为空。摄像头列表从 `/api/system` 读取未返回的识别计数并回退为 0。视频源 2 在停止状态，底图接口返回 `no_worker`，随包 FFmpeg 可以解码该 AV1 MP4 首帧。

v0.5.0 随包人员模型为 ONNX opset 22，随包 ONNX Runtime 1.20 只支持至 21。此前发布验收读取 PPE 状态字段便通过，没有验证人员模型输出，模型加载失败也会被计为零人。发布冒烟须对正式包中的真实解码与人员输出建立断言。

`LS-070`。生产实验服为白大褂。随包 PPE 类表已含 Lab Coat、No Lab Coat、Mask、No Mask，当前本地级联只消费实验服类别。人脸识别已有按相机许可及确认匹配记录，现场告警需独立开启和限频。大华设备接入使用操作者可访问的 NVR／IPC RTSP，通道及主辅码流均由表单指定。

`LS-070` 卸载扫描记录中的 Temp 项来自 `labsentinel-benchmark-*`、`labsentinel-face-*` 和 `labsentinel-ls058-release-*` 三次开发运行，安装器无法安全推断任意同名前缀目录的归属。BAM、CloudStore、AMD HKIDs 等由系统或驱动管理。安装器应只按安装路径清理自身状态，并用隔离的安装和卸载验证数据保留与删除行为。

`LS-071` 现场 v0.6.0 库只读查到两路来源、无区域规则、视频状态 completed、事件 0 条。照片 59 张中 56 张检到人，共 57 人框，全部判成穿实验服。隔离 CPU 对现场 64.233 秒 AV1 视频加规则复跑得到 20 条进入事件，未穿实验服和未戴口罩均为 0。每两秒取一帧的 32 张图片识别出 87 个人框，11 个有穿衣证据，0 个有未穿证据。模型整图原始输出在两个违规类上峰值约为 0.019 和 0.000，现场样本需单独验收 PPE 召回。

其它核验结果：当前 DirectML 会话创建、运行和析构均有进程级 `dml_lock()`，Windows 测试通过锁串行用例。正式包内置 PPE 模型通过 CPU 评测，安全区和排班单测、只读分享守卫单测与工具清洗用例通过。路由报告为 106/106，并列出 46 条 OpenAPI 清单外的路由。前端构建通过，Vitest 提示 `Events.vue` 表格缺少行分组包装，且 jsdom 未提供 Canvas 上下文。正式安装器 Authenticode 状态为 `NotSigned`，签名证书继续在 `LS-900` 跟踪。`NOTICE` 记载的 ONNX Runtime 版本 `1.24.4` 与实际准备脚本固定的 `1.20.1` 有出入。

## 6. LS-059 后续迭代的现状核对

- 首页 `Dashboard.vue` 把 Rust 后端和 GPU 提示放在首张卡片。实际模型状态已有 `gpu_active` 与 `gpu_pref` 可用。默认正式包实测 `GET /api/update/check?refresh=1` 返回 `enabled=false`、`current=0.5.0`、`latest=null`、`url=null`。接口依赖环境变量清单，发布流水线只上传 ZIP、安装器与哈希，现有生成脚本的产物链接在 `platforms` 中，接口读取顶层链接。默认更新入口统一走官方 Release。 [最新正式发布](https://github.com/windyxyz/LabSentinel/releases/latest)
- 告警截图对应 `AlertSettings.vue` 的 SMTP 和 Webhook 表单。当前 flex 布局按每行最高卡片决定下一行位置，字段数量不同造成空档。适配窄屏时卡片最小宽度为 360 px。其它可见残留包括报表标题「任务87」、侧栏「M1」、过时的令牌指引和手动启动 go2rtc 提示。
- 「管理令牌」当前保存浏览器本地副本，服务端令牌由运行配置决定。设备页能创建一次性码、列表和吊销，仓库 Web 没有常用的浏览器配对认领入口。人脸目前依赖 CompreFace 环境变量，支持名单登记与删除。待新增的会话、设备范围和人脸模型能力见 `task_plan.md`。
- 参考文件 `0.4.0_x64.exe` 的产品名为 `facewinunlock-tauri`、版本 `0.4.0`、Authenticode `NotSigned`。本机 SHA-256 为 `c5d157b3e1f771357a988d013947ef58dc3cd07e4c3f717123c103cf5366f99c`，与官方发布的 `0.4.0_x64-setup.exe` 一致。此次依据文件元数据和公开介绍做交互参考。 [官方发布](https://github.com/zs1083339604/FaceWinUnlock-Tauri/releases/tag/0.4.0)

## 7. v0.6.0 统一范围与发版

- 用户确认全部后续任务进入 `v0.6.0`，规划后直接执行，完成后走 GitHub 正式发布。`LS-060`～`LS-067` 为实施范围，`LS-068` 负责验收与发布。旧三版本建议仅为历史讨论。
- 安装目录 Program Files 不应承载运行账户写入的配置。回滚选择依据当前已安装文件版本，历史 `current.txt` 可能指向旧版归档。RTC、密码和设备迁移均需使用既有数据及实际会话进行回归。
- `LS-060` 浏览器实测已收到 WebRTC 首帧，同时分析 worker 未取得快照。随包 FFmpeg 对同一 RTSP 流返回 `Option rw_timeout not found`，其 RTSP 解复用器提供的是 `-timeout`。将分析取帧作为正式包播放验收的一部分。
- `LS-065` 记录的分析帧龄始于服务端读完原始帧。摄像头到浏览器的端到端延迟需另测。实时采集允许覆盖待分析帧，必须同时复核关键事件真值和证据。子码流与主码流使用相同视角和画面比例。
- `LS-066` 本地采用五点对齐及余弦相似度的 YuNet／SFace。CompreFace 自身可配置是否存源图，因此引导单独的识别服务并显示远程删除状态。人员识别仅用当前同意范围，启停与 PPE、登录独立。 [OpenCV SFace](https://github.com/opencv/opencv_zoo/tree/main/models/face_recognition_sface) · [CompreFace REST API](https://github.com/exadel-inc/CompreFace/blob/master/docs/Rest-API-description.md)
`LS-072`。事件页旧实现同时启用文档 View Transition 和抽屉 CSS 过渡。照片留痕未持久化原始图片记录的关联，事件媒体和样本入口因此找不到图。

`LS-073`。用户说明模型和数据用于本地非商业学习。实验服和口罩图像标注、时序动作标注、已有同意约束的人脸模板应分别复用现有流程。

- **PPE 候选**。Roboflow Pharma Manufacturing（CC BY 4.0，含未穿实验服和未戴口罩）最贴近负类。BioSafety PPE Detection（CC BY 4.0）和 UAEU Safety Lab（CC BY 4.0）用于正类补充。CPPE-5 有公开模型 Zoo 与医疗防护正类，作者 Hugging Face 数据卡未给出明确数据许可。AIZOO SSD 提供轻量口罩基线，FMLD 可区分正确与错误佩戴，底图沿用原数据集条款。
- **落地**。继续复用人员框与 ByteTrack，以头部和躯干裁剪比较轻量分类器与原模型。RT-DETR、RTMDet 作为检测对照，均按输出和 ORT 兼容性单独适配。动作先做清晰的饮食、区域停留，使用 AVA 预训练 SlowFast 或姿态辅助。录入人脸按同轨迹关联，低质量或不确定结果交人工核验。
- **评估**。用真机独立片段测试，按摄像头或原始视频隔离验证集。新候选尚未跑过现场基准。现有随包模型已经使用过一个 LAB PPE 来源，新增公开数据需去重。
