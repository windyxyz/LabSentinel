# Progress — LabSentinel 会话进度与交接快照

## Latest Handoff Snapshot

- **`LS-101` v0.9.6 GitHub 发布**。用户确认发布，仓库签名运行器尚未配置。本次准备未签名预览版，待 GitHub Windows 验收及发布资产核验。
- **`LS-104` Rust CI #83**。将录制测试的可变绑定限定在 Windows。为 `paste` 的停更公告 `RUSTSEC-2024-0436` 登记单项例外。Windows GNU 通过 `cargo fmt --all -- --check`、`cargo clippy --offline --locked -p ls-ingest --all-targets --all-features -- -D warnings` 和 `cargo test --offline --locked -p ls-ingest recording::tests`，8 项通过，1 项性能基准默认忽略。`cargo deny --locked --all-features check` 四项通过。已通过 `gh run watch --exit-status` 确认修复提交 `a90698d` 的 [Rust CI #84](https://github.com/windyxyz/LabSentinel/actions/runs/45848606557) 双平台 Clippy、格式、路由覆盖和两项依赖审计通过。同一提交的 [tests #156](https://github.com/windyxyz/LabSentinel/actions/runs/45848607090) 通过。
- **本地缓存与目录维护 `LS-103`**。Rust 与 Vue 源码位置保持原有结构，模型目录改为 `models/`，接力文档进入 `docs/maintenance/`，早期 Android 和模型实验移入各自的归档。43 个本地验收批次按用途改名并归入 `Resources/验收归档/`。当前可试用的双包与 SHA-256 在 `Resources/本地发行包/0.9.6/`。
- **清理验证**。Cargo 官方清理从 C 盘移除 63.4 GiB 编译生成物，D 盘程序构建缓存和 Python 软件包缓存分别清理 490.1 MiB 与 981.6 MiB。历史验收与旧环境采用 NTFS 压缩。用户授权后使用 Windows PowerShell 5.1 执行 `scripts/清理重复验收缓存.ps1 -Apply`，删除 1,039 个重复文件，原始大小 18.20 GiB，实际增加 11.25 GiB 可用空间。D 盘可用空间为 218.49 GiB。脚本改用 .NET SHA-256，隔离预览与 `-WhatIf` 通过，清理后再次核对双包哈希。此前一并复验 8 个随包模型文件、Rust fmt、离线 Cargo 元数据、Python 语法、186 条接口和新旧目录入口。
- **0.9.6 本地验收收口**。按用户确认，继续现有环境的正常验收和可复现问题修复。NVIDIA、Intel、现场监控与正式签名在对应环境验证。两份本地包继续随附现有识别模型。白大褂模型能力缺口留待训练新权重，GitHub 发布等待用户本地试用。
- **`LS-101` 只用本版候选验收部署**。移除 `release.yml` 的旧版安装器下载。PyYAML 与 PowerShell 语法检查通过。用实际 0.9.6 安装器实测新装、带测试版本标记的覆盖和回滚、网页文件哈希还原、恢复候选版本、HTTP 自检及默认卸载保留数据。隔离夹具的路径末尾分隔符和卸载参数修正后复验通过。管理员服务与全清卸载验收仍在隔离 Windows 流程。报告 `Resources/验收归档/发布 - 0.9.6 安装回滚和多路短测/report.json`。
- **最终包核验与 AMD 短测**。ZIP 的七个 ONNX 及实验服类别表逐项匹配锁定哈希，真实安装后的 40 个文件与 ZIP 相同。用 `web/scripts/soak_product.mjs` 对本包跑四路 24 秒、九路 28 秒，逐路 4.980–5.021 FPS、4.990–5.061 FPS，事件片段可回放，SQLite 完整，正常退出。人员与 PPE 同帧的最高逐路 P95 为 118.25 ms 和 328.25 ms，仍高于正式性能目标。报告位于 `Resources/验收归档/发布 - 0.9.6 安装回滚和多路短测/` 的 `four/`、`nine/`。
- **当前 `LS-100`，裁剪再检归属**。把裁剪再检的衣物框投回原图并复用唯一人员归属。增加相邻人、重叠归属、边距外物及正确归属测试。Windows GNU `cargo fmt --all -- --check`、`cargo test --offline --locked -p ls-analyzer --all-features` 共 37 项、`cargo clippy --offline --locked -p ls-analyzer --all-targets --all-features -- -D warnings`、`cargo build --offline --locked -p ls-server --features vulkan-cv --example ppe_acceptance` 通过。
- **同模型逐人对照**。新旧程序分别在同批 112 张公开图及同一份候选资产走 DirectML 和 AMD Vulkan。每个后端比较原图、模拟降亮度及检测或标注人框，共 477 项逐人判定。DirectML 477 项一致。Vulkan 476 项一致，一张蓝色工装由未穿改为未知。复验消除了跨目录的整帧差异。21 张按现场白大褂口径目视确认的对照判定均保持，逆光误报仍存在。报告 `Resources/验收归档/识别 - 白大褂逐人准确率/crop-ownership-comparison-dml.json`、`crop-ownership-paired-vulkan.json`。
- **新版 0.9.6 本地双产物**。严格离线组装、Inno Setup 7 编译和 SHA-256 校验通过。ZIP 的 40 个文件与暂存目录及实际个人安装逐一一致，相对上轮仅主程序变更。新 ZIP 运行 `scripts/smoke_rust.py --zip … --vulkan-validation uuid`，验证 Vulkan 与 CPU 切换、照片与视频留痕、截图、本地问答及退出。新程序从文件夹检查 21 张目标样本，输出与评测一致，独立数据库完整。本机指定目录个人安装及 HTTP 自检通过，默认卸载清除注册并保留测试数据库。产物与报告见 `Resources/验收归档/发布 - 0.9.6 最终双包核验/`。
- **接续 `LS-100`**。用既有分段计时定位当前多路 PPE 延迟，再做不改变输入尺寸、数值和模型输出的短测。白大褂逆光和未知仍建议通过后续训练解决，真实低光和其他显卡验证待相应环境。管理员服务、全清卸载、签名和 GitHub 发布仍按 `LS-101` 执行。
- **当前 `LS-101`，隔离部署与签名流程**。手动运行 `release` 可在 GitHub 托管 Windows 测个人安装、原生服务、网页重启、覆盖安装和保留或全清卸载。标签发布仅接收专用运行器验证过的签名双产物。已通过 YAML 依赖和 6 个 PowerShell 文件解析，用模拟证书和签名调用跑真实 ZIP 和 Inno 组装，核对主程序字节与两个校验和。无证书、无时间戳、ZIP 内容与已签主程序不一致均会阻断。未运行 GitHub Actions 或制作正式签名包。
- **白大褂优化复核**。在两份现有 ONNX 上对照全图、整人裁剪和上身裁剪。主 PPE 模型的裁剪未降低穿戴漏检。单独调用随包白大褂复核模型，在 112 张公开图、119 个标注人框上按 0.65 得分和 0.15 类间差可补出 7 个原未知，其中 6 个已目视确认为白大褂，48 个原标签为未穿的人框无新增判为穿戴。旧有逆光白大褂误报仍无法纠正。这是 CPU 分析性对照，阈值在同一批素材探索，现场泛化、遮挡与多路额外开销尚未验证。当前候选模型与判定不变。报告 `Resources/验收归档/识别 - 白大褂逐人准确率/coat-crop-sensitivity.json`、`coat-confirmation-expanded.json`。
- **当前 `LS-101`，补齐 0.5.0 个人升级**。核对旧包 SHA-256 与 CI 一致，在隔离位置实际安装并初始化 schema 16。0.9.6 覆盖升级后 40 个包内文件与 ZIP 一致。使用真实新版程序迁移至 schema 32，保留已确认、误报和两类历史 AI 复核，按结论查询返回正确事件。回滚后的 0.5.0 可从同一新库读取两条事件，默认卸载仅保留完整数据库且清除注册。报告 `Resources/验收归档/搜索与权限 - 万条语义索引/from-0.5.0/installer-report.json`。
- **兼容性记录**。本机个人路径已验证从 0.5.0、0.6.0 迁移及全新安装。后续不逐一找开发阶段的安装包。管理员 SCM、全清卸载和签名流程保留，按对应环境安排验收。发布等用户确认。
- **当前 `LS-101`，候选安装与项目主页**。上次中断时已经通过 0.6.0 覆盖升级、40 个文件逐项一致和回滚自检。重新运行独立目录卸载，仅留下已核验的测试数据库。另从最新版安装器全新安装到独立目录并运行 `smoke_rust.py --install-dir ... --vulkan-validation fallback`，照片、视频、事件证据、本地问答和可用设备回退通过，卸载后目录和注册项清理。ZIP 实际解包走 AMD Vulkan UUID 的冒烟通过。双产物及各自 SHA-256 见 `Resources/验收归档/搜索与权限 - 万条语义索引/`。
- **0.9.6 集中工程回归**。Rust fmt、全工作区全特性 Clippy 和测试通过，280 项通过、12 项专用用例保留忽略。前端 108 项、Vite 构建、开发支具 16 项通过，最终 ZIP 中 10 个 Web 资产与本轮构建逐字节一致。Rust/OpenAPI 双向 232/232，前端契约 186 项，Chromium WebRTC/MSE、回退与授权 6 组均通过。前端仍有原 Vite 配置兼容提示和测试 Canvas 提示。
- **本地主页**。更新 `README.md` 的安装、接入、事件触发录像、AI、MCP、存储及验收范围，保留项目标志。新增实际候选的仪表盘、事件证据、存储管理三张合成演示截图，验证本地链接和 PNG。GitHub 发布继续等待用户本地验收。
- **接续**。本地双产物供用户试用。进入正式发布阶段时读取 `scripts/README.md` 的隔离验收和签名条件，再核对现场设备、真实低光、异厂显卡和管理员服务。
- **当前 `LS-100`，万条语义索引**。10,000 条 768 维实库中，Windows 的向量检索期间临时启用有界映射读取，结束后恢复原值。保留现有余弦距离、排序与预过滤。三个查询各运行 35 次，重开同一持久化库的 P95 由 84.7–88.6 ms 降至 24.9–28.7 ms。刚写完未关闭库时，无机位限制的 P95 为 30.65 ms。见 `Resources/验收归档/搜索与权限 - 万条语义索引/knn-*.json`。
- **一致性与存储清理**。定向 15 项存储测试通过，额外核验 25 次并行更新和 20 次限定机位查询。最新证据修改会过滤旧向量。Windows 结束查询后映射恢复为 0，清理夹具记录并整理数据库后从 41.2 MB 到 10.5 MB，SQLite 完整。Rust fmt、ls-store Clippy、实模型语义检索和配对测试通过。
- **实包权限与编码**。优化候选在 10,000 条向量上实际运行中文、英文和以图搜图。机位从两路收窄至一路后，列表、截图、缩略图、录像字节范围和审计包一致收紧。另一设备吊销后旧会话失效。后台新编码 1 张真实截图并通过相似查询。三个同图 HTTP 单次耗时从 125–140 ms 到 58–63 ms。首次文字模型加载仍约 1.85 秒，见 `optimized-http/report.json`。
- **后台编码短测**。用优化前候选对比 4 路 1080p 各 30 秒，后台新增 13 条真实向量的同时，各路帧率变化为 −0.667% 至 +0.629%，均在本次 3% 目标内。两轮均完成证据片段回放和正常退出。见 `indexing-throughput.json`。
- **当前 `LS-100`，白大褂逐人验收**。`ppe_acceptance` 使用随包 0.9.6 权重、文件解码尺寸和 PPE 阈值，在 Vulkan、DirectML 分别跑 112 张公开图。原标注 8 个已穿／未穿分歧中，7 个穿有色工装，按用户的白大褂场景单列。按目视复核选 10 张穿白色实验服、11 张普通衣服。原图均见一人，两后端白大褂 0 确认穿戴、1 判未穿、9 未知。普通衣服 5 判未穿、6 未知。给定标注人框也得到相同计数。降亮度后漏检 1 人，不能作为真实低光验收。报告 `Resources/验收归档/识别 - 白大褂逐人准确率/whitecoat-vulkan-summary.json`、`whitecoat-dml-summary.json`。
- **发行候选核验**。实际 0.9.6 DirectML 在隔离实例逐张扫描这 21 张，计数与逐人工具一致。逆光穿白色实验服的样本仍判未穿。首轮夹具把输入放在数据目录而被扫描器按设计排除，拆开后通过，数据库完整。报告 `package-whitecoat-report.json`。离线构建、fmt、定向 Clippy、IoU 唯一匹配和参考清单复核通过。生产权重、阈值和安装包沿用上一轮。
- **接续**。执行 `rg -n '逐人 PPE|低光|Vulkan 多路' docs/plan/v1.0.0-roadmap.md`。按用户口径训练安排在后续，当前保留现场白大褂的精度缺口，接续其他发布验收项。到现场后按机位独立做低光、背影和遮挡复核。
- **当前 `LS-100`，PPE 裁剪等值优化**。BGR 上采样缓存两行完整定点水平加权和，最后一步统一舍入。使用 `rustc -O` 对照原函数，9 组输出逐字节一致，三个典型裁剪 P50 分别为 1.360→0.409、0.953→0.375、1.244→0.489 ms。这项改动沿用权重、尺寸和阈值。fmt、`cargo clippy --offline --locked -p ls-infer --all-targets --all-features -- -D warnings` 和 `cargo test --offline --locked -p ls-infer --all-features` 通过，23 项通过、6 项专用用例保留忽略。
- **发行候选复验**。分别运行 `compare_images.py before/after`，16 张固定图和 30 张已有公开样本，共检出 47 人，各图 PPE 计数、人员数、区域和违规字段完全一致。这验证同输入的输出等值。逐人和低光真值门槛仍未验收。AMD 用相同 1080p 四路各 22 秒短测，分别约 5 FPS，回放、退出和数据库检查继续通过。人员与 PPE 同帧的最高逐路 P95 为 122.56→131.77 ms，各路只有 6–13 个样本，本轮未证实整条链路提速。
- **更新本地候选双产物**。严格离线打包、7 个 ONNX 与 Vulkan 哈希验证、最终 ZIP 冒烟通过。安装包实际 0.6.0 → 0.9.6 覆盖升级、回滚及默认卸载再验，40 个文件与 ZIP 一致，保留样本数据库。新候选及报告在 `Resources/验收归档/识别 - 裁剪预处理优化/`。原 0.9.6 候选目录用于对照，GitHub 发布继续等用户验收。
- **接续 `LS-100`**。执行 `rg -n '逐人 PPE|难负例|PPE.*裁剪' docs/plan/v1.0.0-roadmap.md`。补齐逐人真值、低光与遮挡验证，再据固定素材判断精度优先的端到端优化。跨厂商实测、现场设备、原生服务及正式签名仍待对应环境，已完成的四小时 AMD 稳定性证据继续复用。
- **0.9.6 本地候选双产物**。版本号与 npm/Cargo 锁文件同步，`npm run build`、Rust fmt、严格离线打包和 Inno Setup 7 构建成功。ZIP 155,197,985 字节，安装器 126,704,787 字节。产物、SHA-256 和报告见 `Resources/验收归档/发布 - 0.9.6 首个候选/`。锁定的 7 个 ONNX 模型 opset 11–21，Vulkan 资产及解压 DLL 哈希通过。主程序和安装器为本地未签名候选。
- **升级、回滚与卸载**。先实际安装 0.6.0，再覆盖 0.9.6。40 个安装文件与 ZIP 内容一致，新旧程序分别通过 HTTP 自检。首轮卸载发现回滚残留 `labsentinel-server.exe.old`，在 Inno 清理清单中增加固定文件名后重编安装器并重跑。复测仅保留独立测试数据库，数据库完整、记录未丢失、卸载注册已清除。取证 `installer-report-before-fix.json` 和 `installer-report.json`。
- **当前 `LS-100` 精度优先**。使用候选实际程序运行 `verify_image_equivalence.py`，16 张图片、16 个人员检测的 PPE、事件和其他检测字段与原样本输出完全一致。当前输入尺寸、权重、置信度与策略沿用对照。候选两轮冒烟核对 API 版本 0.9.6、照片及视频检查、事件证据、本地问答，AMD UUID 与无效设备 DirectML 回退均通过。三名远景人员的白大褂仍未知，固定样本等值之外的精度需独立真值验收。
- **AMD 880M 发行候选短测**。运行 `web/scripts/soak_product.mjs`，4 路 34 秒含受控断流及一次真实本机 VLM，9 路 32 秒。断开前 4 路各 4.986 FPS，计入断流后 4.267–4.410 FPS，重连后分析 3.457 秒恢复、大屏 4.077 秒恢复。VLM 单次 3.444 秒，同期各路仍分析 17–18 帧。9 路 4.987–5.049 FPS，大屏同时解码 5 路。两轮触发录像可回放，SQLite 完整，退出后未发现采样到的媒体进程残留。人员加 PPE 同帧各路 P95 最大 217.945 和 262.866 ms，尚有优化空间。见 `four/report.json`、`four/vision.jsonl`、`nine/report.json` 及逐路采样。
- **接续**。执行 `Get-Content -Encoding UTF8 docs/plan/v1.0.0-roadmap.md | Select-Object -Skip 252 -First 30`。按固定逐人真值和各显卡短测追踪准确度、PPE 延迟及现场通道。原生服务、实物联动、完整安装矩阵及正式签名待对应环境。GitHub 发布等待用户本地验收。

- **`LS-099` 离线审计与联动恢复短测**。审计 ZIP 含按摘要去重的规则和模型记录，截图与记录摘要分别展示核验状态。真实隔离事件导出 1 条、6 项哈希全部通过，录像独立解码成功，修改隔离截图后注明与存证不符，未认证导出返回 401。
- **`LS-099` 强杀恢复**。隔离 HTTP 设备先确认旧设备 L2。旧设备拒绝复位时换新设备，强杀测试进程后重启仍等待旧设备确认 0，再给新设备发 L3。正常关停确认新设备 0 后清除记录。未注册服务状态码 3，直接进程启动服务模式被 SCM 拒绝。报告 `Resources/验收归档/联动与审计 - 继电器恢复和证据包/proof/report.json`。Rust 联动 3、审计 1、服务 2 项，调试构建、fmt、Clippy、Vite 构建、186 条契约通过。管理员 SCM 生命周期和实物继电器待现场。
- **当前 `LS-100`，0.9.6 集中工程回归**。同一工作树最终执行 Rust fmt、全工作区全特性 Clippy、`cargo test --offline --locked --workspace --all-features --no-fail-fast`，280 通过，12 个专用用例保留忽略。首轮告警回环夹具在 Windows 的 accept 继承非阻塞模式，显式恢复阻塞读取后全量复测通过。前端 Vitest 108 项与构建、Python 开发支具 16 项、186 条前端 API 静态契约通过。
- **接口登记**。补齐 48 个未登记实现，复核实际挂载时补 WebRTC 信令路径并修正两条 WebSocket 根路径。Rust 注册清单、OpenAPI 和实际路由源码三方核对均为 232 个方法与路径。CI 改用 `--strict`，夹具验证同一路径缺方法或凭空多登记均会失败。
- **应用短测**。在 0.8.5 调试暂存实例，Chromium 的 WebRTC、MSE、主子流回退、预览恢复及会话撤销 6 组通过。真实 AMD Vulkan UUID 和无效 Vulkan 设备回退冒烟分别通过，覆盖照片、视频、事件、规则及本地问答。两张代表性图共检出三人，实验服三人均未得明确判断。日志及明细 `Resources/验收归档/回归 - 0.9.6 集中工程验收/summary.json`。
- **接续**。下一命令 `Get-Content -Encoding UTF8 web/scripts/soak_product.mjs -TotalCount 37`。用最终候选进行 4/9 路及本机视觉组合短测，对照现场逐人准确度和跨厂商性能。原 AMD 四小时证据继续复用。管理员 SCM、实物继电器和大华/乐橙现场通道仍待对应环境，正式签名与本地双产物尚未验收。

- **`LS-097` 可选 Retina 分析入口**。摄像头的分析码流里可启用 Retina，选择即时生效并随配置导入导出，数据库仍为 v32。H.264/H.265 保留编码和 RTP 时间戳。失败时当前 worker 用 FFmpeg 重试同一地址，再沿用主码流回退。RTSPS 直接用 FFmpeg。
- **三路回环短测**。`python -X utf8 -B "Resources/验收归档/取流与录像 - Retina 和 FFmpeg 回退/verify.py" acceptance-v2` 通过。AMD 隔离调试实例同时跑 Retina H.264、Retina H.265 和模拟拒绝后 FFmpeg 子流回退。静止阶段写盘 0 段。三段触发证据均可用 HTTP 206 和 FFmpeg 读取，分别为 5、5.167、5.25 秒，维持 640×360。ffprobe 验证 185 个片段包的 SHA-256 全部命中源包，三段 DTS 单调且有 B 帧 PTS 重排。握手中停止 65 ms 内关闭连接，正常退出，SQLite 完整。报告 `Resources/验收归档/取流与录像 - Retina 和 FFmpeg 回退/acceptance-v2/report.json`，首轮夹具在状态已运行但首份计数未发布时提前读取，复测等到采样后取证。
- **工程验证**。`cargo test --offline --locked -p ls-ingest` 20 项通过，长耗时录像基准保留忽略。服务分析设置定向 1 项、配置导入导出及旧配置默认、调试构建、三 crate 全特性 Clippy、fmt、Vite 构建和 186 条接口契约通过。依赖锁将 pretty-hex 留在 0.4.1 兼容 Retina 0.4.22。
- **接续 0.9.6**。大华及乐橙具体型号、主辅流和现场重连仍需经授权的实机通道。接续 `LS-099`，执行 `rg -n '原生 Windows 服务|audit_pack|Actuator' .github/workflows/release.yml engine-rs/crates/ls-server/src engine-rs/crates/ls-jobs/src`。当前 Windows 会话未提权，系统没有 LabSentinel 服务或可见串口，可先推进隔离审计包及联动验证。保持 GitHub Release 暂停。

- **`LS-095` 准确率优先的裁剪优化**。AMD 上实测同权重 640/448/320 输入，15 个来自原图及半尺寸夹具的人框，模型调用 P95 为 9.90/5.29/5.60 ms。448 和 320 分别改变 4 和 2 个实验服标签，含从穿着到未穿。原生算术选项也出现跨阈值分数，继续使用现有 640 和 FP16 存储策略。报告 `Resources/验收归档/性能 - 防护识别预热与裁剪/crop-sizes.json`、`native-options.json`。
- **等值缩放已验证**。预先计算列索引和定点权重，按行处理 BGR。旧版与新版 9 组尺寸的输出逐字节一致，1080p 缩放中位数 1.604→1.079 ms，三个裁剪案例中位数减少 0.420–0.584 ms。两份 Vulkan 发行构建通过 16 张照片对照，人员数、PPE、违规与事件字段一致，库完整。报告 `Resources/验收归档/性能 - 防护识别预热与裁剪/resize-bench.txt` 和 `folder-comparison.json`。
- **短测与门槛**。使用随包 ORT，`cargo test --offline -p ls-infer --features vulkan-cv yolo::tests` 10 项、`cargo test --offline -p ls-analyzer --all-features ppe` 9 项通过。fmt、定向全特性 Clippy、发行构建和契约审计通过。四路 30 秒 AMD 短测逐路 4.991 FPS，GPU 调度等待 P95 最大 25.421 ms，帧龄 P95 最大 70.184 ms，事件片段可回放、退出无观测残留进程、SQLite 完整。人员加 PPE 同帧 P95 61.696–125.991 ms，每路 7–16 个 PPE 样本。该短测未证实端到端提速，继续以 25 ms 为优化目标。报告 `Resources/验收归档/性能 - 防护识别预热与裁剪/four/report.json`。
- **接续 0.9.6**。先推进 `LS-097` 剩余 RTSP 通路与回退，执行 `rg -n 'rtsp_transport|RecordingTap' engine-rs/crates/ls-ingest/src`。保留 AMD 四小时证据，Vulkan 默认晋升等待异厂设备短测和 §3.1 验收。当前构建仍为 0.8.5。

- **当前 `LS-095`，Vulkan 原生管线缓存**。同一块显卡共享 SDK 编译缓存，应用重启后可在当前 Windows 账户恢复。缓存无效时走正常编译。独立 C 接口探针 PPE 输出逐字节一致，模型加载 244→50 ms，另需约 7 ms 恢复。报告 `Resources/验收归档/性能 - Vulkan 冷启动缓存/probe-comparison.json`。
- **AMD 产品短测**。同一调试构建运行四轮照片检查，分别生成、恢复、损坏重建、恢复。三模型均走 Vulkan，每轮识别同一张图中的 1 人，均正常退出，SQLite 完整性通过。首次人员模型载入 5.591 秒，恢复两轮为 52、48 ms，缓存损坏的重建轮为 246 ms。首次检查 11.224 秒，恢复约 3.6–3.8 秒，损坏重建轮 3.418 秒，冷启动还受驱动自身缓存影响。报告 `Resources/验收归档/性能 - Vulkan 冷启动缓存/product-proof/report.json`。
- **存储管理**。新增独立的 Vulkan 编译缓存类别，显示大小并可预览、确认清理和设置自动清理。账户缓存实测约 1.9 MB。定向验证覆盖保留期、同目录 DLL 和其他文件。Rust 缓存完整性 1 项、AMD 共享网络及白大褂对照 2 项、服务存储 2 项、fmt、两 crate 全特性 Clippy、Vite 构建和 186 条契约通过。Vite 仍有既有配置加载方式提示。
- **接续**。沿用已通过的 AMD 四小时报告，短测逐帧 PPE 延迟与回退。下一命令 `rg -n 'crop_bgr_roi|detect_classes|coat_confirm' engine-rs/crates/ls-analyzer/src/ppe.rs engine-rs/crates/ls-infer/src/yolo.rs`。结合裁剪、复检和同步耗时选择下一处优化。25 ms 与跨厂商门槛仍开放，灰度基线 0.8.5。
- **当前 `LS-095`，Vulkan 等待与首次推理**。新增 `gpu_queue_ms`（按帧合计 Vulkan 模型锁等待与 DirectML 调度等待）、`vulkan_lock_wait_ms` 与 `vulkan_first_run_ms`。现有 `vulkan_run_ms` 记录拿锁后的同步调用，包含驱动等待及输出复制。大屏可查看 GPU 调度等待和 Vulkan 首次推理。
- **缓存并发与验证**。对同进程的 DLL 解包、白大褂权重生成分别加锁并复查缺失状态，只在首次生成时写入，保留输入与最终缓存的哈希校验及失败回退。Rust 定向缓存失败重试、推理计时、服务统计 3 项通过。AMD 真机复跑 12 张复核匹配、损坏缓存回退和共享网络生命周期通过。`cargo clippy --offline -p ls-infer -p ls-server --all-features --all-targets -- -D warnings`、fmt、前端大屏 12 项、Vite 构建、JS 语法及 186 条 API 契约通过。
- **AMD 发行版 4/9 路短测**。`cargo build --offline --release -p ls-server --features vulkan-cv` 通过。4 路两轮 30 秒、9 路一轮 25 秒，逐路 4.98–5.03 FPS。4 路 GPU 等待 P95 最大 13.46 和 22.74 ms，9 路最大 26.88 ms，都低于各自门槛。本机 Chromium 验证全部可见机位显示数值，事件录像回放、正常退出与 SQLite 完整性通过，无图形事件和采样媒体进程残留。报告 `Resources/验收归档/性能 - Vulkan 等待与执行计时/comparison.json`。
- **首轮瓶颈**。每个隔离候选实例各生成一次 DLL 缓存（187–192 ms）与白大褂缓存（18–20 ms）。首轮四路就绪 10.700 秒，PPE 原生模型载入 5.615 秒，含首次 Vulkan 调用的帧记录 140.081 ms。四路复测就绪 4.527 秒且 PPE 原生模型载入 330 ms。候选同帧人员与 PPE P95 四路首轮 171–227 ms、四路复测 65–106 ms、九路 75–121 ms。下一步分解首次原生模型加载和冷帧推理，25 ms 门槛仍开放。当前发行候选版本号 0.8.5。
- **当前 `LS-095`，白大褂复核 Vulkan 路径**。从随包 ONNX 重建原始预测头的 ncnn 权重，并在 Rust 复现两级 TopK。4,899,224 字节权重中 4,797,888 字节可从相同 ONNX 的张量直接生成。嵌入参数及压缩描述共 4,608 字节，构建工具复现产物哈希。运行时校验源 ONNX 和还原权重。缓存不可用或 Vulkan 失败时继续用 ONNX。报告 `Resources/验收归档/性能 - 白大褂复核 Vulkan 后端/delta-report.json`。
- **真机模型对照**。Radeon 880M 在 12 张固定公开样图及验收帧上对照 ORT CPU 原 ONNX。正式阈值 0.55 的 5 个框全部按类匹配，要求 IoU ≥0.95、置信度差 ≤0.02。`cargo test --offline -p ls-infer --features vulkan-cv coat_head_preserves_packaged_detections -- --ignored --nocapture` 复验还原权重缓存损坏时校验会拒绝，原 ONNX 自动通过 DirectML 加载。PPE 定向规则 4 项、Clippy、Rust fmt、API 契约和差异检查通过。现场识别精度仍按 §6.1 的逐人标准验收。
- **四路发行程序短测**。`cargo build --offline --release -p ls-server --features vulkan-cv` 后，使用上一轮候选和新候选分别两次 30 秒。新候选两轮按日志只初始化一份辅助复核网络，四路 DirectML 推理调用均为 0，原版每路 7–8 次。每路约 5 FPS，真实事件录像解码、数据库完整性和正常退出符合用例。GPU 内存中位数从 234–238 MiB 降到 178–205 MiB。隔离版本仍为 0.8.5，基础照片视频检测及无效设备 DirectML→CPU 回退冒烟通过。报告 `Resources/验收归档/性能 - 白大褂复核 Vulkan 后端/comparison.json`。
- **性能边界**。首次候选四路就绪 10.706 秒，倒序复测 4.374 秒，原版为 4.358–4.386 秒。候选整帧同帧人员与 PPE P95 首轮 189–228 ms，慢样本都出现在首个快照，倒序复测 76–100 ms。原版两轮为 53–109 和 57–108 ms。候选的白大褂复核阶段 P95 倒序复测 18–25 ms，原版 16–24 ms，暂未证实延迟收益。保留统一 Vulkan 的资产和灰度路径，接续区分初始化、队列等待及真正推理耗时，优化 25 ms 全链路门槛。
- **当前 `LS-095`，持久显卡选择**。设备列表优先使用唯一、非零的 Vulkan UUID。当次 LUID 关联 DXGI 的 DirectML 回退。旧 LUID 仍能唯一定位设备时自动迁移。设备不可用、身份重复或无法关联 DXGI 时沿用配置的回退。已失配而未迁移的旧标识需重新选卡。
- **AMD 隔离升级和程序重启**。用上轮灰度二进制保存 Radeon 880M 的当前 LUID，再由本轮调试二进制打开同库。旧设置自动迁移 UUID，DXGI 生效编号为 0。升级和再次重启软件后两次照片的人员与 PPE 均走 Vulkan。写入不存在的 UUID 后，两者自动进入 DirectML，第三次照片仍有识别留痕。三次进程正常退出，SQLite 完整性为 ok。首轮夹具按自动恢复机位等待超时，便携模式明确启动机位后复测通过。报告 `Resources/验收归档/性能 - GPU 可复用选择/restart-proof-v2/report.json`。
- **定向回归**。`cargo test --offline -p ls-server --all-features gpu_enum::tests::` 两项通过，包含模拟更换 LUID、重新排序、缺卡与重复 UUID。配置兼容和拒绝部分写入一项通过。`cargo clippy --offline -p ls-server -p ls-infer --all-targets --all-features -- -D warnings`、Rust fmt、前端 13 项、Vite 构建、186 条契约及差异检查通过。系统重启由变更 LUID 的夹具覆盖，真机验证使用隔离软件重启。
- **隔离调试目录冒烟**。`python -X utf8 -B scripts/smoke_rust.py --install-dir "Resources/验收归档/性能 - GPU 可复用选择/staging" --vulkan-validation uuid` 通过。人员与 PPE 先走明确指定 UUID 的 Vulkan，网页切到 CPU 后继续识别并留痕。照片、视频、事件截图、规则、问答和正常退出通过。版本号仍为 0.8.5。
- **当前 `LS-095`，Vulkan 同卡模型复用**。灰度 Vulkan 在同一套已验证资产、显卡及精度下复用原生网络。每次加载仍验证资产。同一网络的调用串行，人员和 PPE 各用独立网络，最后一个调用方退出即释放网络。真实 AMD 测试确认两个调用方输出逐值等于独立网络，对象释放后可重新加载。
- **四路发行构建 A/B**。Radeon 880M，原版与候选各两次 30 秒。候选日志只初始化人员和 PPE 各一次。原版 LabSentinel 进程 GPU 内存中位数为 386.0–386.9 MiB，候选两轮均为 237.0 MiB。进程工作集从 759.5–768.2 MiB 降至 459.6–519.1 MiB。各轮均为每路约 5 FPS，事件短片解码、正常退出、无采样媒体进程残留、SQLite 完整性均通过。报告 `Resources/验收归档/性能 - 多路共享识别模型/comparison.json`，每轮目录保留原始采样。
- **冷启动与延迟边界**。候选首轮从创建机位到四路开始识别用时 9.664 秒，PPE 网络加载用时 5.536 秒。倒序复测候选 4.345 秒，原版两轮 4.347、4.364 秒。候选同帧人员与 PPE P95 首轮 168–211 ms，较慢样本已出现在首个快照，复测 59–107 ms。原版分别为 63–82 和 93–106 ms。候选保留在 Vulkan 灰度，继续优化首轮和 PPE 时延，25 ms 晋升门槛仍开放。程序灰度版本号为 0.8.5。
- **重启后旧显卡标识失配**。本次 Windows 启动后 Radeon 880M LUID 变更，旧灰度配置走 DirectML 回退。四路 A/B 使用自动选卡且双方均为同一张 AMD 卡。接续检查唯一硬件身份恢复选卡，保留设备缺失、同型号多卡歧义和旧配置兼容。
- **本轮定向验证**。`cargo build --offline --release -p ls-server --features vulkan-cv` 通过。`cargo test --offline -p ls-infer --features vulkan-cv cameras_share_a_net_until_the_last_camera_stops -- --ignored --nocapture` 在实现前缺少复用入口，完成后真机 1 项通过。`cargo test --offline -p ls-infer --all-features --lib` 20 项通过，Clippy 检查 `ls-infer`、`ls-analyzer` 与 `ls-server` 全特性全目标通过。发行目录 `scripts/smoke_rust.py --install-dir … --vulkan-validation luid` 通过，覆盖物理设备 Vulkan、设置切换至 CPU 后继续识别、照片视频与事件证据。Rust 格式、取证脚本语法和 186 条前端 API 契约通过。
- **当前 `LS-095`**。网页可按物理显卡名称保存 Windows LUID。ncnn 在自身 Vulkan 实例上通过 LUID 定位设备，DirectML 通过 DXGI 独立匹配相同 LUID。找不到设备时使用已保存的 DirectML 设置，旧式 ncnn 编号也可继续使用。LUID 在驱动或系统重启后可变化。
- **AMD 真机及定向验证**。Radeon 880M 的物理 ID 对应 DXGI 0。隔离图片分别核对选中显卡后人员与 PPE 均运行 Vulkan、故意缺少锁定 Vulkan 权重时同卡 DirectML 回退、物理 ID 不存在时回到已配置的 DirectML。照片均入库有识别结果。报告 `Resources/验收归档/性能 - 显卡标识与回退/run-63fb69d1/report.json`。Rust 配置与不同枚举顺序测试通过，Clippy、fmt、前端检测页 13 项、Vite 构建、186 条契约检查通过。
- **灰度双产物**。`Resources/验收归档/性能 - 显卡标识与回退/release/` 下的 ZIP 154,768,829 字节，SHA-256 `675df43a9979e2efd5feef0b72f0a38e5873bb2a7dccf092c11f11d1de238e54`。安装器 126,383,445 字节，SHA-256 `0e7c1e9dd7a4a3bf039ca79e1ffd24122c4383bcbc6fbeada54d7decf75a9d2e`，未签名且未安装。Inno 编译和 ZIP 的 `--vulkan-validation luid`、图片视频识别、事件证据和优雅退出均通过。当前灰度版本号为 0.8.5。
- **发行版同卡短测**。所选 AMD LUID 用于四路人员和 PPE 的 Vulkan，llama.cpp 明确确认同卡 `Vulkan0` 的 31/31 层。25 秒场景按 CV 预留 1024 MiB 与视觉预算 1536 MiB 启动，两次真实带图答复期间各路分别分析 17 和 7 帧。事件短片已回放，正常退出、无采样媒体进程残留、SQLite 完整。报告 `Resources/验收归档/性能 - 显卡标识与回退/release-luid-vision-four/report.json`。
- **PPE 预热窗口验证**。Rust RTSP 基准增加 `LS_BENCH_WARM_PPE=1`，每路实际触发 PPE 后保留预热数据并清空统计。AMD 调试版单路 20 秒实际 Vulkan 识别 94 帧，采样 2 帧合规且事件 2 条。预热阶段 Vulkan P95 153.794 ms（13 帧），测量阶段 11.886 ms（94 帧）；同帧 PPE 预热 188.735 ms（1 次），测量 P95 182.182 ms（4 次）。报告 `Resources/验收归档/性能 - 显卡标识与回退/ppe-warm-debug-single.json`。这些为调试构建数据，用于核对窗口，后续用同权重发行程序测性能。
- **白大褂转换资产压缩试算**。三套 ncnn FP16 权重无损压缩逐字节复原。现用 Deflate 6 约 17.91 MB，普通 XZ 约 17.56 MB，双字节按位拆分再 XZ 最小为约 16.80 MB。含 DLL、参数和归档空间估算，仍高于 15 MB 门槛。未改变包体或模型。报告 `Resources/验收归档/性能 - 显卡标识与回退/weight-compression.json`。
- **`LS-095` PPE 延迟诊断**。新增整图、逐人裁剪、白大褂复核、近景口罩分阶段耗时与样本数。分阶段取值属于实际调用的帧，尤其可识别几帧稀疏样本的 P95。20 秒真机调试短测中，人员与 PPE 均在 AMD Vulkan，完成 92 帧和 2 个合规采样。5 个 PPE 帧均调用裁剪，2 个调用白大褂复核，口罩辅助模型未运行。报告 `Resources/验收归档/性能 - 防护识别分段计时/vulkan-debug-single.json`。
- **预处理半尺寸缩放**。对 2:1 双轴 BGR 输入按四像素整数平均，保留原定点舍入。七组边长和分辨率逐像素一致。独立优化编译的 1280×720 → 640×360 基准从 1.592 ms 降到 0.246 ms，测量范围为缩放步骤。普通比例使用原实现。
- **定向测试**。分位样本数的回归用例从缺项失败到通过。Rust PPE 相关 12 项通过，YOLO 9 项通过，显式执行的故障回退测试通过。YOLO 首轮在系统 DLL 上遇到 IR10 不支持，指定随包 ONNX Runtime 后同命令通过。三 crate Clippy、fmt 与差异检查通过。
- **短时发布构建对照**。分阶段基线为 `Resources/验收归档/性能 - 防护识别分段计时/staging`，含新遥测和原缩放路径。Vulkan、DirectML 和 CPU 的 4 路 25 秒短测均通过取证，包含真实事件片段播放、无采样孤儿进程和数据库关停检查。同帧人员与 PPE P95 分别为 67.291–82.337 ms、217.069–258.561 ms、1037.579–1891.491 ms，每路 PPE 样本分别为 7–8、12–14、11–13。Vulkan 整图 17.104–19.296 ms、逐人裁剪 15.148–16.833 ms、白大褂复核 17.448–32.773 ms。报告 `baseline-auto_vulkan/`、`baseline-dml/`、`baseline-cpu/`。
- **半尺寸优化成品复跑**。灰度暂存 `Resources/验收归档/性能 - 防护识别分段计时/optimized/staging`。四路两次均接近 5 FPS、真实事件录像播放和关闭检查通过。首次 197.157–276.710 ms P95，较慢样本已包含在首个短测快照。相同参数重新测量时，基线 68.910–86.153 ms、优化版 61.149–88.796 ms，首个快照均未发生 PPE 调用，各路最终 PPE 样本 6–7。当前可确认像素等值与局部缩放加速，成品 P95 收益未能稳定复现。报告 `baseline-vulkan-repeat/`、`optimized/vulkan-four/`、`optimized/vulkan-repeat/`。下一步采集同帧的首次与预热后时序，逐项对照模型调用并继续优化。
- **白大褂确认模型本地转换探针**。已有 ONNX 文件经固定 pnnx 输出候选，在 ncnn 真机加载时因未注册的 `pnnx.Expression` 返回 -1，转换日志另有 TopK、GatherElements 和 Mod。该候选仅保存在 `coat-conversion-probe/`，没有纳入发行资产。
- **原始预测头替代探针**。截取同一 ONNX 的 `[1,8,8400]` 预测头可实际在 Radeon 880M 执行。隔离脚本复现原图的两次 TopK。四张原监控样图在诊断阈值 0.02 下是 22/22 框匹配。补现有 ENSI 测试、验证集的 8 张实验服相关样图后，正式阈值 0.55 下 5/5 框匹配，无新增框，最大坐标误差 0.302 px、分数差 0.001389。证据 `coat-conversion-probe/top-k-parity.json`。资产以 Deflate 6 试打包会增加 4,490,257 字节，叠加当前 13,421,960 字节超过 15 MB 限制，仍需优化包体和在实际代码上核对逐人结果。
- **当前目标 `v0.9.6`，接续任务 `LS-095`**。托管本机视觉选卡、预算及回退已在隔离发行实例取证。AMD 四小时探针已结束。接续推动 Vulkan 端到端延迟优化与设备选择。GitHub Release 留待用户本地验收。
- **用户调整稳定性验收**。保留已通过的 AMD 四小时结果。0.9.6 后续 Vulkan 和 DirectML 采用短时用例回归，72 小时与 12 小时连续跑改为可选诊断。跨厂商短测、性能目标、识别、权限与故障恢复要求按当前路线图继续。
- **AMD 4/9 路发行包 Vulkan 组合验证**。修正 go2rtc 夹具源冲突后，38 秒四路与 31 秒九路均通过，所有机位人员与 PPE 均运行 Vulkan。VLM 按选定的 `Vulkan0`、1536 MiB 视觉预算、1024 MiB CV 预留启动，九路运行时日志确认 31/31 层卸载。四路 3 次、九路 2 次真实带图问答均与各路分析重叠。断流后分别 4.064/4.617 秒恢复大屏。九宫格最多 5 路解码，均有事件短片实际回放、无图形事件、关闭后无采样媒体子进程残留且 SQLite 完整。报告 `Resources/验收归档/性能 - AMD 四路独立画面/report.json` 与 `Resources/验收归档/性能 - AMD 九路基线/report.json`。
- **产品延迟仍未达标**。终态滚动 `person_ppe_ms` P95 为四路 259.506–348.791 ms、九路 228.580–431.638 ms，超过 25 ms 目标。断流前 16.073 秒四路为每路 4.98 FPS，断流前 15.062 秒九路为 4.91–5.05 FPS。九路含断流全窗口为 4.11–4.35 FPS。接续按同输入 CPU/DirectML/Vulkan 分拆 PPE 各阶段耗时。
- **本机视觉 GPU 启动策略**。网页可从 llama.cpp 自身枚举中选择显卡，也可自动选卡。按显卡可用显存、用户预算和 CV 保留量估算卸载层数。设备身份不明确时保留检测显存，显卡不可用、预算不足或加载失败时由 CPU 接手，网页可查看原因。参数和接入档案同事务保存，旧启动请求继续有效。稳定 LUID 选择与 Vulkan 默认晋升仍待后续收口。
- **定向验证**。`cargo test --offline -p ls-server --all-features local_vlm::` 3 项通过，覆盖 llama.cpp 输出解析、Windows 守护子进程取输出和预算计算。`cargo clippy --offline -p ls-server -p ls-infer -p ls-ingest --all-targets --all-features -- -D warnings`、`cargo fmt --all -- --check`、前端检测页 12 项、`npm run build` 与 186 条接口契约通过。隔离程序和真实 llama.cpp CPU 组件验证无 Vulkan 设备时返回回退原因、拒绝多设备 ID、拒绝未授权探测、保存设置及旧请求显式启用 CPU。SQLite 完整性为 ok。报告 `Resources/验收归档/AI - 本地模型显存预算/isolated-90e98c26/report.json`。
- **显存不足独立实例**。枚举夹具声明 `Vulkan7` 有 600 MiB 可用。真实应用返回预算 600 MiB、计划 0 层和 CPU 回退原因，守护进程收到明确的 `--device none` 后启动真实 llama.cpp CPU 组件。SQLite 检查通过。报告 `Resources/验收归档/AI - 本地模型显存预算/isolated-d9e67610/report.json`。
- **组合脚本复验**。`web/scripts/soak_product.mjs` 先使相机分析与大屏取帧就绪，再启动托管 VLM，报告可记录所选设备、预算和回退原因。用上一份灰度包和同一 CPU 组件完成 18 秒单路短跑。真实带图问答 2.930 秒，期间 14 帧检测，事件 #4 实际播放，SQLite 完整、进程正常退出。报告 `Resources/验收归档/AI - CPU 与 GPU 资源协调/report.json`。
- **本批灰度双产物**。`Resources/验收归档/AI - 本地模型显存预算/release/` 下 ZIP 154,762,730 字节，SHA-256 `af86b7037646e452ffb42ccb5b9d690e0dd4d62276e2d4d7f8e29a42a56cedc9`。安装器 126,393,090 字节，SHA-256 `85b1eeacd085c7134cd951391ff24ff22bee4d8094c243b0e5f706eb3d96863f`，未签名且未实装。ZIP 冒烟和发行程序的 600 MiB 模拟回退通过，报告 `Resources/验收归档/AI - 本地模型显存预算/isolated-5b29f58d/report.json`。
- **本批发行程序 CPU 组合短测**。32 秒单路跑出两次真实视觉问答，用时 4.127 / 4.430 秒，分别与同机位 20 / 21 次推理重叠。断流后源就绪至分析恢复 2.664 秒、大屏恢复 4.158 秒。片段 #6 实际播放，退出与 SQLite 完整性通过。先前 22 秒配置下，断流恢复和单并发视觉处理使 2/3 个计划请求完成，严格断言失败，证据保留。通过报告 `Resources/验收归档/AI - 含空格路径部署/report.json`。
- **原 AMD 四小时探针通过**。Radeon 880M 跑 14,400.143 秒，四路各 72,000 帧，跳过为 0，迟到分别为 7/8/6/6。120 次真实 Vulkan VLM 请求均重叠，平均 361.525 ms。llama.cpp 确认使用同一张卡及 31/31 层，原进程均退出，终态报告没有 Windows 图形事件或设备丢失。探针人员与 PPE 累计执行 P95 75.6569 ms，仍需产品条件下对照 §6.3 的 25 ms 门槛。报告 `Resources/runs/Vulkan 推理后端对照/zip-size-comparison/radeon-880m-soak-4h-2026-11-25/report.json`。
- **新包 AMD 照片入口**。`--vulkan-validation active` 和 `fallback` 串行均退出 0。自动选择时人员与 PPE 实际使用 Vulkan，非法 ncnn 编号时两者自动进入 DirectML，两组都能通过网页设置软重启至 CPU 并继续检查。
- **四路取证夹具修正**。同一 FFmpeg 输入命令使 go2rtc 的四个发布进程使用同一回调，第 4 路重连失败，首次在 45 秒预热时报错。按路使用独立路径后复跑，保持相同图像及同一包版本。原报告 `Resources/验收归档/性能 - AMD 四路基线/report.json`。
- **大屏事件补发**。首屏历史查询记录最后可见编号，WS 从该编号继续。旧游标不可见或已清理时，握手标记重置，大屏提示、刷新并合并期间到达的事件。Rust 真实 WebSocket、机位授权定向测试 2 项，前端 30 项、构建和 Clippy 通过。隔离 Chromium 实测首屏与握手之间提交一条，断网时提交两条，再模拟清理断线游标。授权事件 `[3,4,6,7]` 按编号各到一次，三个外范围事件均不可见，最终列表 `[7,4,3,1]`。报告 `Resources/验收归档/浏览器 - 事件断线重放/report.json`。
- **发布构建真实 VLM 复验**。`ls100-managed-vision-candidate` 的灰度 ZIP 为 154,733,789 字节，SHA-256 `5ba199324837750ae55f6e99ca6e3eb8488a131bf010d4ad68a2cff83aeaa3b0`。同载荷隔离运行 CPU 1080p 单路，真正的托管视觉问答 4.679 秒，同期 24 帧分析。片段 #5 并行播放，源就绪至大屏恢复 4.055 秒，退出及 SQLite 完整性通过。结果 `Resources/验收归档/AI - 发行环境视觉组合/report.json`。
- **加入补发修正的灰度双产物**。`Resources/验收归档/浏览器 - 重放测试候选/` 的 ZIP 154,734,868 字节，SHA-256 `9cf039b06c55c0895efa0db92869a892432b2d0bc4dded42183470364ab34819`。同载荷安装器 126,356,736 字节，SHA-256 `820b81c3759271909dc440c404d0d4f1d5fc3607acdf06fed2828fada642c3fa`，未签名且未安装。隔离 ZIP 冒烟通过。Chromium 发行实例复现授权补发 `[3,4,6,7]`、过期编号 6 和最终列表 `[7,4,3,1]`。报告 `Resources/验收归档/浏览器 - 发行环境大屏重连/report.json`。
- **产品长稳取证入口**。`web/scripts/soak_product.mjs` 运行隔离 Windows 包、回环 1080p RTSP 和 Chromium 大屏。支持 1/4/9 路、定时断流和托管 VLM。逐样本记录模型设备、推理、帧龄、进程工作集和句柄，并验证事件 MP4 的范围请求、解码、大屏同步播放、Windows 图形事件、采样到的媒体子进程和 SQLite 完整性。每个样本的性能量为当时滚动窗口，正式 72 小时与跨卡门槛仍待实测。
- **实际本机 VLM WebP 兼容性**。用同一张验收事件图和固定 llama.cpp 运行时复现 WebP HTTP 400，JPEG HTTP 200。修正托管本机档案发送 JPEG，原留存截图和通用视觉档案保持独立。Rust 两项定向测试、fmt、Clippy 和 JS 语法通过。修正后用调试主程序、同套灰度资产与真实托管 CPU SmolVLM 复跑。视觉问答 4.983 秒、同期该路完成 22 帧推理，返回事件原图 ID。模拟断流后，RTSP 源就绪至分析恢复 2.677 秒、至大屏恢复 3.291 秒。事件 #5 支持 Range 并在 Chromium 与大屏同时解码。退出正常，采样过的媒体进程均退出，SQLite 完整性为 ok。报告 `Resources/验收归档/AI - CPU 托管修复核验/report.json`。
- **`LS-098` 官方 MCP 客户端端到端**。以 `@modelcontextprotocol/sdk@1.32.2` 连接隔离发布程序，完成真实 Streamable HTTP 握手、全部 11 项只读调用和 9 项写工具逐项授权。各写操作均核对业务效果和 `audit_log.detail` 的 allowed/denied。验证机位隔离、证据缩略图额外许可、凭证与篡改拒绝、跨域及转发来源拒绝、凭证吊销。单次回环视觉请求携带一张截图，复核结果已写回事件。文字搜图在本轮验证关闭时的返回，已启用索引的现场对照仍属 `LS-100`。Rust 入站测试增加非回环同机地址且 LAN 关闭的 403 断言并通过。复跑命令见 `docs/dev/api-reference.md`。
- **Vulkan 推理计数与 PPE 统计**。原生 ncnn 成功调用计入 worker 的实际推理次数，供实时性能和定期合规采样使用。新增 `vulkan_calls`、`vulkan_run_ms` 和 `person_ppe_ms`，后两项排除相应阶段未执行的帧。RTSP 灰度测试要求各路真实运行人员与 PPE，并核对定期合规样本落库。预热结束清空测量窗口时保留分析码流类型。
- **本轮验收证据**。新增的三个 Rust 定向测试、相关 crate 的 fmt 与 Clippy、两份大屏测试共 12 项、前端构建和 186 条 API 面覆盖通过。真实合成 1080p RTSP 的 CPU 调试短测中，22 秒内推理 102 帧，落库 2 帧定期合规样本，报告保留 `input=main`。受控断开 2 秒后重新取帧，总恢复时间 6,145 ms。`person_ppe_ms` P95 为 310.525 ms，按调试基准记录，发布构建性能仍待另测。报告 `Resources/验收归档/性能 - Vulkan 灰度验证/cpu-telemetry-rtsp-reconnect.json`。
- **更新后的遥测灰度双产物**。位于 `Resources/验收归档/性能 - Vulkan 灰度遥测/`。`labsentinel-vulkan-probe-0.8.5-20261125.zip` 为 154,733,272 字节，SHA-256 `0bb397d4df983e67f3c1bee703f14c20e9a0be8667ef1b63c4cd98283e8d4993`，比上一份普通包大 13,497,255 字节。`LabSentinel-setup-0.8.5.exe` 为 126,350,955 字节，SHA-256 `08c4665250195d8703f6b85ff70edc107f1a4162c0713910cec2205560a3581c`，未签名，尚未实装。新版 ZIP 隔离 CPU 冒烟通过。四小时探针出报告后，用此 ZIP 跑 `--vulkan-validation active`、`fallback` 和产品 RTSP 对照。
- **上一份普通／灰度包实测对照**。普通包为 141,236,017 字节，SHA-256 `5f34ff70ba6f6ea0c36730f2179a4d55503830a9bd3edc173bc1d75d8542965b`，灰度包为 154,732,758 字节，相差 13,496,741 字节。逐文件哈希核对后，两包共享的模型和运行时资源一致。两个构建的主程序特性不同，前端使用不同时间构建的资产，因此保留本次全包实测差值及早前相同主程序的纯资产对照。普通包 `smoke_rust.py --zip ... --timeout 90` 通过，接口确认灰度能力为 `false`。
- **`LS-093` 产品灰度入口**。网站按包能力显示 Vulkan，旧 `runtime.ep` 兼容读取与回写。`runtime.backend` 可选 `auto_vulkan`。`runtime.gpu_id=auto` 使用 ncnn 默认设备，指定 `vulkan:<n>` 时按 ncnn 原生索引验证。实时相机、视频和图片文件夹使用同一加载链，逐路报告人员与 PPE 的实际设备。灰度路径遇到 GPU 推理失败会在下一后端重试同一帧。普通包仍默认 DirectML。
- **定向验证**。Rust fmt、灰度及探针特性的 Clippy、普通包 `cargo check --no-default-features --all-targets`、旧设置和阈值 API 测试（2 项）、真实 ONNX CPU 上注入 GPU 错误后重试并更新设备状态（1 项）、检测/模型面板 Vitest（17 项）、`npm run build`、186 条前端契约和 `git diff --check` 通过。已登记的 OpenAPI 路由在灰度主程序上覆盖 183/183，47 条现有唯一 method/path 还缺 OpenAPI 记录。`windows_rtsp_benchmark` 可选择 `auto_vulkan` 并验收各路人员和 PPE 的实际设备。
- **本地灰度 ZIP**。`scripts/build_rust.py --strict --zip --vulkan-probe "Resources/runs/Vulkan 推理后端对照/zip-size-comparison/ncnn-fp16-verified.zip"` 通过。`Resources/本地发行包/历史测试包/labsentinel-vulkan-probe-0.8.5-20261125.zip` 为 154,732,758 字节，SHA-256 `0d2a86ac3a29c667362739977fc3b28ddfac9b1f74ef7187b7574e61779d2243`。`scripts/smoke_rust.py --zip ... --timeout 90` 通过，确认灰度能力、CPU 照片和视频识别、留痕、告警预览、本地问答和优雅退出。两张现场风格验收图检出 3 人，实验服均待确认。正式四小时流程结束后，用 `--vulkan-validation active` 和 `fallback` 验证真实 Vulkan 和无效设备回退，再用 `LS_BENCH_EP=auto_vulkan` 运行产品 RTSP 对照。
- **本地灰度安装器**。Inno Setup 7 用上一份灰度载荷构建 `Resources/验收归档/性能 - Vulkan 灰度验证/LabSentinel-setup-0.8.5.exe`，126,354,566 字节，SHA-256 `0ea45f78b47deb4d3def212dbc71ddf57bf37f8e2d3c42c4bb694a8728ab5771`。主程序和安装器签名状态均为 `NotSigned`，安装器尚未实际运行。
- **Windows 灰度资产集成**。`build_rust.py --vulkan-probe` 走独立 staging。新资产锁同时校验原始人员/PPE ONNX、对应类别表、探针包、每个归档成员和解包后的 DLL。用同一已构建 `ls-server` 比较完整 ZIP，基线 141,226,931 字节，灰度 154,652,214 字节，增加 13,425,283 字节。公共成员逐字节一致。错误探针、错配 PPE 类别表在覆盖 staging 前失败。
- **Rust 灰度资产验证**。`ls-infer/vulkan-cv` 提供锁定模型的 `YoloModel::load_vulkan`、有界 XZ 解包和缓存哈希验证，共用 ncnn FFI。`cargo clippy --offline -p ls-infer --features vulkan-spike --example vulkan_spike -- -D warnings` 和该 example 调试构建成功。`vulkan_spike --verify-package` 在中文路径通过冷、热缓存校验，篡改 DLL、参数和 PPE 类别表均退出 1。
- **AMD 四小时同卡探针运行中**。2026-11-25 香港时间 19:41:12 开始，预计 23:41:12 完成。运行目录 `Resources/runs/Vulkan 推理后端对照/zip-size-comparison/radeon-880m-soak-4h-2026-11-25/`。21:45 实查控制器 PID 15388、CV 15656、VLM 26276 均存活。`cv.log` 每路 36,901 帧，零漏时隙，每路 2 帧超时隙，`requests.jsonl` 62 个本机图片请求回执。接续核对同一路径的进程、进度和最终 `report.json`，进程退出且无通过报告时检查同目录及 `paced-4h.stderr.log`。
- **长稳验收工具的定向验证**。四路各 5 FPS、人员和 PPE 双模型，18 秒完成每路 90 帧，4 次 VLM 全重叠，零漏时隙、零晚帧、Windows 事件查询命中 0。双模型 p95 64.11 ms，`LS-100` 的产品链路目标待实测与优化。另验证用 `stop.requested` 取消后两个子进程均退出，且不生成通过报告。Rust 定向 Clippy、release 探针构建、原有非定速 CLI、Python AST 检查及 `git diff --check` 通过。
- **`LS-086` AMD 原生对照**。复用已下载并固定的 ncnn SDK 与同版模型。发现 PPE 在 FP16 权重配合 FP16 算术时会出现不稳定的框横坐标。FP16 存储、FP32 算术用 4 张样图重复 3 次按类匹配，人员 60/60、PPE 24/24。四路吞吐人员 168.58 对 DirectML 56.01 FPS，PPE 187.32 对 114.80 FPS。原生测量工具校验 Vulkan 设备范围，`LS_NCNN_ADAPTER=999` 负例退出 1。探针 crate 定向 Clippy、release example 构建、Python 语法和框匹配反例通过。
- **`LS-086` 包体**。独立探针 ZIP `Resources/runs/Vulkan 推理后端对照/zip-size-comparison/ncnn-fp16-verified.zip` 为 13,421,960 字节，SHA-256 `56abbbabffe7f0b640e235c2de97244de92b757ecd9822c73b349897e0628434`。只压缩 DLL 的表示形式，五个文件解包后与原件逐字节相同。重复打包一致，篡改参数对照被拒绝。发行包集成待核对。
- **`LS-086` 同卡短测**。`tools/vulkan_spike_coexist.py` 最终通过。AMD 880M 四路人员 CV 完成 2,400 帧，164.65 FPS，p95 27.65 ms。等待四路计时开始后，两个本地 SmolVLM 图片请求分别重叠 456、457 ms，llama.cpp 日志确认 `Vulkan0` 和 31/31 层卸载。结束后探针与 llama-server 子进程均退出。详细报告见 `radeon-880m-cv-vlm-synchronized/report.json`。四小时、跨厂商及 72 小时测试仍待执行。
- **`LS-099` 原生服务**。全用户安装在默认 Program Files 路径可选 SCM 值守，使用独立虚拟账户和 `service-data`，默认启用读取保护。服务在重启后恢复用户已启动的实时源和文件夹监听，避免重跑离线视频。设置的网页重启复用 SCM 进程并重建运行时。安装器按所属二进制确认 SCM 注册，更新前停止服务，卸载默认保数据。Inno Setup 7 使用临时载荷编译通过。发布 CI 增加注册、网页换端口、覆盖安装、默认卸载与全清卸载冒烟，YAML 与 PowerShell 语法已校验，尚未触发远端执行。当前进程为中等完整性，管理员组仅作拒绝使用，实机 SCM 生命周期待隔离管理员环境。
- **`LS-099` 声光联动**。设置内可启用内网 HTTP JSON 继电器或 USB-RS485 Modbus RTU 8E1。单线程按 SQLite 已放行事件计算最高级别，静默、误报、待复核不驱动输出。检测事件段恢复后降级或复位，硬件失败独立展示并重试。实测回环 HTTP 503 后重新同步输出，验证三级线圈写入报文和 CRC。现场 USB-RS485/继电器留待实物验收。
- **本批定向验证**。`cargo test -p ls-server -p ls-jobs -p ls-store --all-features -- service actuator deploy::tests audit_pack_exports mcp_is_disabled browser_pairing_scopes` 共 15 项通过。三 crate Clippy `-D warnings`、GNU debug 构建、`--selftest-http`、`npm run build` 和 `git diff --check` 通过。直接运行 `--service-run` 离开 SCM 环境会退出 1。契约脚本匹配 186 条前端 API / 路由登记 232 条；OpenAPI 181/181 覆盖，另有 49 条现有唯一 method/path 尚无 OpenAPI 描述。
- **本地双产物**。在 `Resources/验收归档/联动与审计 - 原生服务和本地边界/` 独立暂存，沿用 `VERSION=0.8.5`。严格 Rust 组包与 Inno 正式载荷编译通过。`labsentinel-local-0.8.5-2026-11-25.zip`（141226864 字节，SHA-256 `8b547100dac5bb8518481da32976f383994fd7c508a6ab78e2f758766f7542a2`），`LabSentinel-setup-0.8.5.exe`（113110313 字节，SHA-256 `701dfeb5cd6b5cd9213a0d720edc8e06f6a8e78c7e7f023509b592626c6d7e64`）。主程序和安装器 Authenticode 均为 `NotSigned`，本机当前用户证书库无代码签名证书。保留组包暂存，自动审批拦截了其递归清理命令。
- **预览包冒烟**。`scripts/smoke_rust.py --zip ... --timeout 90` 通过，覆盖照片和视频预览、检查留痕、告警截图、规则、推理失败记录、本地问答和报告。两张实验室样图检出 3 人，实验服穿戴和确认未穿均为 0，仍为未知。`node web/scripts/smoke_webrtc.mjs "Resources/验收归档/联动与审计 - 原生服务和本地边界/staging"` 6 项通过，覆盖分析辅流和回退、WebRTC、MSE、网关恢复，以及设备配对、读取授权和访问密码。安装器未在当前机器运行。现场 SCM、物理联动、Vulkan 跨厂商和 72 小时长稳仍待正式验收。
- **`LS-098` MCP**。修正复核提示词编译错误，限制请求体接收时间为 10 秒，执行前再次核对令牌和权限。测试覆盖默认关闭、独立令牌、11 项只读、9 项逐项授权、Tier-3 拒绝、机位隔离、撤销、过期与凭证篡改。实际客户端与所有写工具的业务效果留 §6.2 复核。
- **`LS-099` 证据包**。事件页按当前筛选导出 HTML、CSV、JSON、截图、有效的触发录像及 SHA-256 清单。配对设备只可导出获准机位。验证过滤、证据路径限制、缺失媒体状态、CSV 公式与 HTML 转义及逐文件校验。`cargo test -p ls-server --all-features -- mcp_is_disabled audit_pack_exports browser_pairing_scopes` 3 项通过，`cargo clippy -p ls-server -p ls-store --all-features --all-targets -- -D warnings`、`npm run build`、`python -X utf8 -B scripts/contract_audit.py`（184 前端 / 230 Rust）与 `git diff --check` 通过。

- **范围修订（2026-11-25，覆盖此前的范围收窄）**。仅取消连续录像。按运动和告警触发的录像、事件回放、同时播放和检查、多路优化和值守大屏全部保留。MCP 全套、原生 Windows 服务、声光联动、审计包、AI 复核、自然语言问答和 AI 报告均保留。Vulkan 为最终默认后端，按 `LS-086/093/095` 的原实测门槛推进。
- **开发前规划核验**。用 `git show aee9cb6:docs/plan/v1.0.0-roadmap.md` 对照当前任务表及验收。原有 22 个任务 ID 无缺失，另保留 `LS-102`。完整 MCP 段落和 `LS-098/099` 条目与原版一致。10 项范围检查通过，`git diff --check` 通过，开发前核对的现存源码、版本文件及用户草案摘要一致。
- **`LS-097` 告警**。SQLite v31 以同一事务保存事件和待投递意图。保持 VLM 挂起、放行和抑制语义。隔离回环验证原子回滚、待发恢复后只投递一次、WebHook 稳定请求标识、200 回执与状态不明确时待核实。`cargo test -p ls-store -p ls-jobs --all-features` 14 + 19 项通过，`ls-server event_` 3 项通过，生产入口原子回滚用例从失败修到通过。
- **`LS-097` 触发录像**。FFmpeg 一路取流同时供实时分析和原编码片段环，触发后才落盘。`ls-ingest` 定向测试验证 H.264 `avc1`、HEVC `hvc1`、关键帧、时间跳变、超出有界缓存和原包不重编码，1280×720 的 10.04 秒 fMP4 本机写盘样本 2.92ms。隔离 go2rtc 双路实测静止无录像、运动后原编码短片、HTTP 206 和一边读录像一边正常获取实时帧。源分辨率 320×192 保留，压测与原厂设备未在这轮完成。定向 Clippy、`npm run build` 和 179 条前端接口契约通过。
- **`LS-097` 强杀复测**。隔离 go2rtc 与测试数据目录中，依次强杀并重启 Rust 进程三次。每轮重启前 SQLite `PRAGMA integrity_check` 均为 `ok`，两段已入库的原编码触发短片均通过 FFmpeg 解码。
- **存储管理已定向验收**。设置页按类别展示占用及空间，支持预览、确认清理、自动保留和数据库整理。隔离测试核对管理员权限、过期预览、共享证据、未处置告警、原图片、当前日志及最新自动备份。数据库清理返回实际回收量。未更改现场数据。
- **离线搜图已定向验收**。Rust 通过可选模型的双语 CPU 编码与真实截图索引，对照机位权限缩小后立即生效。两张样本的白大褂英文相似度为 0.1075 对 0.0600，中文 0.0708 对 0.0699，保留现场验证门槛。
- **当前批次验证**。`cargo test -p ls-store` 10 项通过，`cargo test -p ls-server --all-features storage` 2 项通过，独立模型搜图用例 1 项通过。定向 Clippy、Vite 构建、179 条接口契约与可执行文件的 167/167 路由覆盖通过。集中回归按节点执行。

- **新增存储管理 `LS-102`**。0.9.0 增加分类占用、预览和清理。既有保留作业已覆盖部分事件副本、孤儿媒体和旧标注目录，本批扩展网页分类清理与保留设置。

- **`LS-096` 权重**。Ghost Downloader 已取得 SigLIP 2 图像、文本 ONNX 及分词器。两份权重 SHA-256 核对上游，CPU 推理和向量索引已完成定向验证。

- **推进基线**。用户将当前目标设为 0.9.6。产品基线仍是 `aee9cb6` 对应的 0.8.5。`LS-095` 的数据集子项可独立验收，Vulkan 默认晋升继续等待真机门槛。GitHub Release 暂停。
- **`LS-095` 数据集子项**。SQLite v29 保留旧标注并新增光照、困难负样本和误报点来源。按训练集和光照导出的 ZIP 有版本摘要、样本路径、图片与标签 SHA-256、排除与缺失 ID。低光验证目录不会混入训练合并。开发支具输出参与合并的来源及原许可，训练预检可读，正式训练保存相同台账。本批未运行训练。
- **定向验证**。`cargo test -p ls-server -p ls-store --all-features dataset` 中导出与样本测试通过；`cargo test -p ls-store --all-features` 8 项通过，含 v28 旧样本迁移与重复迁移。`python -X utf8 -B -m unittest discover -s tools/tests` 16 项，`npm run build` 和 171 条接口契约通过。Vulkan 默认晋升与现场精度验收仍需补实测。

- **品牌与版本**。GitHub `main @ d5d4e0f` 的有字图已按远端 blob 摘要核对。无字图进入网页、PWA、托盘、主程序与安装器。`LS-101` 已登记 1.0.0 发布前按项目实况重写 README。
- **`v0.8.0` 本地验收**。`fmt`、全特性 Clippy、Rust 236 项通过（5 忽略），前端 104 项、构建及解包冒烟通过。独立实例验证溯源、热点、温度超限和低比特 GGUF 拒绝。Ghost Downloader 获取并校验了 `llama.cpp b11193` Vulkan、SmolVLM-256M Q8_0 和 F16 投影。独立实例启动进程、写入加密档案并完成一张图的真实回环复核。
- **`v0.8.5` 本地验收**。修正后再次执行 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings` 和 `cargo test --workspace --all-features`，237 项通过（6 忽略）。`npx vitest run` 104 项通过，`npm run build` 通过。最终产物的 `scripts/contract_audit.py` 匹配 171 条前端 API，`scripts/rust_route_coverage.py --fail-under 100` 覆盖 159/159。包内 ORT 核验了姿态 ONNX 的 CPU 和 GPU 偏好。独立包核验四套场景、过期预览返回 409、YOLO26n 切换、实际照片检查和恢复默认。两个 Windows 可执行文件和 ZIP 网页图标均与新图标一致。Vite 的表格、配置和 jsdom Canvas 仍有既有警告。
- **`v0.8.5` 产物回归**。首次解包用例发现，已选择的人员模型文件丢失时静默回退，导致失败的视频被显示为检查完成。恢复原有模型错误状态后，`python -X utf8 -B scripts/build_rust.py --strict --zip`、`python -X utf8 -B scripts/build_setup_exe.py`、`python -X utf8 -B scripts/smoke_rust.py --zip Resources/dist/labsentinel-rust-0.8.5-20261124.zip --timeout 55` 全部通过，覆盖照片和视频留痕、视频预览、告警截图、叠框、推理失败对照及优雅退出。安装器交互安装、升级与卸载待独立环境验收。
- **`v0.8.5` 最终文件**。`Resources/dist/labsentinel-rust-0.8.5-20261124.zip` 为 139335435 字节，SHA-256 `65923952fdebf90a06db375e02c89b25cf1bdeea6bb33f6d1618ac3077b7e252`。`Resources/dist/LabSentinel-setup-0.8.5.exe` 为 111931949 字节，SHA-256 `1ae61b3684ff6e8e50f75af87660e654a62c031c413d5c0cb81f9a9cacc3c69f`，文件属性版本为 `0.8.5`。
- **`v0.8.5` 主线**。产品提交 `373fb264ceabcc99775824449e780a2ecc8b23b8` 已直连推送。`gh workflow run tests.yml --ref main` 的[回归运行 45364006105](https://github.com/windyxyz/LabSentinel/actions/runs/45364006105)核验同一提交，前端、Ubuntu Rust、Windows Rust 全部通过。Release 工作流未触发。
- **`v0.8.0` 本地文件**。`Resources/dist/labsentinel-rust-0.8.0-20261124.zip` 为 130480214 字节，SHA-256 `60cfbbfbf9e25cf2ce2f1423016d3fe130d4ab0a049cbc1922608e9d7c994c3a`。`Resources/dist/LabSentinel-setup-0.8.0.exe` 为 103497338 字节，SHA-256 `7e25dd22a9d5aec56e225ac1c03d0dd8e6525ce084ed0cc9293609e0ce4adeb2`。
- **Vulkan 灰度条件**。Radeon 880M 对照中 ncnn FP16 一路和四路约 91 / 143 fps。YOLO26n DirectML 一路和四路约 86 / 109 fps，两者是不同权重的独立短测。`LS-093` 已完成可选人员模型对照，Vulkan CV 等其他厂商设备、包体与同卡稳定性条件。GitHub Release 暂停。
- **范围变更（2026-11-24）· 放开非商业/学术数据集随包限制**。用户确认本项目仅作非商业自用与学习研究，允许 `SH17`（CC BY-NC-SA）、`FASDD`、`URFD/Le2i` 等非商业/学术数据集参与训练、评测与随包权重发布；取消 `LS-095` 中的 NC 硬拦截（`exit 2`），改为自动登记来源与原许可证并写入 `NOTICE`。已同步更新 [docs/plan/v1.0.0-roadmap.md](../plan/v1.0.0-roadmap.md)、[task_plan.md](task_plan.md)、[task_registry.md](task_registry.md)。下一实施任务仍从 `LS-089` 接续。
- **同类项目工程做法规划审阅**。`LS-097` 保持 `READY`。对照 `v0.7.5` 源码和同类项目原始资料，更新[路线图版本图与 §2.2](../plan/v1.0.0-roadmap.md#22-同类项目的工程做法与验收)。补入机位健康、历史证据、场景模板、录像回放、告警回执和权限验收。复用实时最新帧、已有预览池与断线补发。此轮为文档更新，下一实施任务仍从 `LS-089` 接续。
- **文档验证**。`git diff --check` 通过，核对了 10 个版本图节点、22 项原有任务与 7 行新增验收的版本归属。
- **模型来源与路线图预研**。`LS-093` 保持 `READY`。补入从 `v0.8.0` 诊断现有 PPE、`v0.8.5` 候选权重和后端分别灰度，到 `v1.0.0` 分类验证的顺序。[来源与实测](../plan/model-source-review-2026-11-24.md)记录权重、许可、哈希与适用范围。产品实现仍从 `LS-089` 接续。
- **定向推理验证**。`python -X utf8 -B "Resources/验收归档/识别 - 模型试用对照/compare_local.py"` 退出 0，输出 18 图 CPU 对照。`python -X utf8 -B "Resources/验收归档/识别 - 模型试用对照/check_folder.py"` 退出 0。使用 `v0.7.5` 暂存包与独立回环实例，随包配置 3 人均未知，绕过末级复核后确认两件白大褂，YOLO26n 替换人员模型后两张图仍检出 3 人。`vulkan_spike.exe cpu <yolo26n.onnx> 5 <3 张 BGR>` 加载随包 ORT，单会话和四会话返回检测框，退出 0。外部 API 调用 0，测试图片来自项目合成及公开样本。
- **规划核验**。`git diff --check` 通过，确认路线图仍为 22 项，候选报告中的本地引用与两份推理结果均已对照。
- **当前节点**。`v0.7.5` 本地测试版已验收。`LS-087`、`LS-088` 完成，下一阶段从 `LS-089` 的任务记录继续。`LS-086` 跨厂商硬件门禁依旧待设备，选型依据见 [ADR-0012](../adr/0012-unified-vulkan-gpu.md#2026-11-24-radeon-880m-实测)。
- **人员窗口更正与验收**。同一人在默认 300 秒内最多送一张，按已录入本机匹配跨轨迹和机位，无匹配只按当前连续轨迹。窗口和多人裁剪即时生效，窗口内其他本地事件继续保留。用两张项目图片由助手判读口罩结果，回环模拟 API 实收两次，每次核对裁剪 JPEG 字节。重复轨迹不发送，不同人触发同类规则各自保留，额度用完继续本地。用已确认身份的测试输入验证换轨、换机位、重启、到期与删除身份。当前证据未测现场人脸准确率。`cargo fmt --all -- --check`、定向 Clippy、`cargo test -p ls-server --all-features worker:: -- --nocapture`（22 项）、前端 15 项及构建通过。
- **补充合成监控验收**。新增白天高位广角和灰度夜视两张合成图。`cargo test -p ls-server --all-features --offline laboratory_fixed_review` 通过。白天指定区域无法确认设备门，模拟接口返回不确定，无事件。夜视可见敞开门扇和搁架，写入确认阶段和原始截图。稳定视图未重复发送，准确收到两次单图裁剪，全程使用回环模拟接口。
- **v0.7.5 回归首轮**。产品版本和 Schema `v26` 已同步。Rust 格式、工作区 Clippy、231 项 Rust 测试（5 项忽略）、104 项前端测试、Vite 构建和 164 条前端 API 契约检查通过。回归审阅修正了 Rust 动态路由写法、原图转码意外放大和表格结构变更后的前端选择器。问答精测仅两次回环请求，分别验证原图与后续纯记录；重发、超范围和过期修订均未增加调用。日报本地降级和 Markdown 导出通过定向用例。双产物构建及解包验收进行中。
- **回归审阅修正**。显式 API/协作机位按实际复核配置变化软刷新，送审前核对档案与授权。固定视图在预算耗尽、请求失败或响应无效时等待 45 秒再试，已成功判读的相同画面保持去重。未指定新范围的追问继承上轮事实，同机位同日的其他事件不带入旧事件的视觉答复。冲突或不清楚的中英文穿戴证据会保留不确定。修正后再次执行 Rust fmt、全工作区 Clippy、全工作区测试，233 项通过、5 项忽略。前端 104 项与构建、164 条前端 API 契约通过。
- **首轮候选实测**。`python -X utf8 -B scripts/smoke_rust.py --zip <0.7.5 包> --timeout 55` 全部通过。随包模型在合成白天图检出 2 人、灰度夜视图检出 1 人，两张图确认穿着实验服均为 0。视频参考线告警有截图和独立的检查记录，本地安全助手可追问、生成和导出报告。候选包在独立数据目录的 Edge 实测显示安全助手和会话历史，回归审阅修正后的双产物以第二次构建为准。
- **0.7.5 主线**。审阅修正后的产品代码 `679777a3cff55e9fafa680595eb87dd7eef2fcc0` 已按直连方式推送到 `main`，远端引用已核实。全量 GitHub `tests` 手动运行号 `45333601816` 的 Web、Windows 和 Ubuntu 均通过。GitHub Release 保持暂停。
- **0.7.5 双产物与解包验收**。`python -X utf8 -B scripts/build_rust.py --strict --zip` 和 Inno Setup 7 编译均通过。`Resources/dist/labsentinel-rust-0.7.5-20261124.zip`，119726207 字节，SHA-256 `37ab7eed86d5bf76bd6195df8d5d855564a6799ac9f7ab53f02a1cd5031de855`。`Resources/dist/LabSentinel-setup-0.7.5.exe`，93408336 字节，SHA-256 `db64c60d85b4a22ffe8bfc4428ea26b6c3ceb3b42b98b6b09cb1d563b1eff66a`。安装器属性确认版本 `0.7.5`。便携包隔离解压冒烟通过照片与视频记录、预览、原图与叠框、抑制审计、错误对照、参考线告警、助手本地问答、报告导出与优雅退出。可执行路由覆盖 `151/151`。两张合成监控图共检出 3 人，实验服模型没有给出确定穿戴或缺失判定。安装器交互安装、升级和卸载未在生产机实测。
- **v0.7.0 双产物**。`Resources/dist/labsentinel-rust-0.7.0-20261124.zip`，119471354 字节，SHA-256 `d983659910f6d2179dcc504853785767e1e86cddf316e6435100939df3d5a8e6`。`Resources/dist/LabSentinel-setup-0.7.0.exe`，93246734 字节，SHA-256 `cf5ce268e92d2ff82050c34cd4962e2feba659352c5ccfd111dcdc4f6b7f3829`。严格打包、Inno Setup 7 编译、解包冒烟中的照片视频留痕、视频预览、告警图片、PPE、抑制审计、推理失败对照及退出均通过。交互安装、升级与卸载待现场复测。
- **v0.7.0 主线**。`3091a9e` 已直连推送。GitHub `tests` 手动全量任务 `45319400069` 的 Web、Windows 和 Ubuntu Rust 均通过。GitHub 标签与 Release 暂停。
- **v0.7.0 全量回归审阅**。首次运行发现旧库清理在缺失抑制审计与违规片段表时中断、清理预览包含仍活跃的旧告警，以及 WebSocket 与数据库版本断言落后。修复后 Rust fmt、工作区全特性 Clippy、221 项测试（5 项忽略）、前端 103 项与构建通过。路由覆盖 138/138，前端 152 条 API 合同匹配，另登记既有 OpenAPI 条目缺口 49 条待按后续功能批补。实际版本为 `0.7.0`，数据库仍为 `v25`。
- **LS-085 定向验收**。事件解释读取历史检测置信度、实验服归属差值、保存时的区域参数、违规片段及 AI 复核。抑制审视支持筛选、分页、用审计帧入池；相近记录采样截图，所有命中仍计数。解释、图片管理员权限、重复提交一份样本、10 分钟事件段复验、前端 5 项、构建与 152 条 API 契约通过。
- **LS-084 定向验收**。SQLite v25 实施。穿戴规则每 2 秒复检，首次合规后 1 秒复核。受控 10 分钟序列产生 301 次检出，提示与升级共用一个事件 ID，峰值截图刷新，确认合规后生成恢复记录。旧库迁移、规则宽限读写、重放、定向 Clippy、前端 19 项、Vite 构建与 148 条前端 API 契约通过。
- **验证节奏**。`v0.6.2` 运行必要检查和定向验证。从 `v0.7.0` 起，每个大版本集中跑全量回归并做回归审阅。
- **LS-083**。DirectML 会话集中在单线程并按使用中的模型共享。实机测试确认两个租用方共享会话，释放其一后另一方完成推理。ZIP 的运行时配置枚举到 2 个 GPU，其中 1 个匹配 Vulkan。事件 WebP、原图叠框、摘要、JPEG 缩略图及告警前转码通过验证。
- **本地产物**。`Resources/dist/labsentinel-rust-0.6.2-20261124.zip`（119401468 字节，SHA-256 `d6cfde886bb02eedbb8d7bef5c7eb1c372f965f946083a74ffb97c602edc7685`），`Resources/dist/LabSentinel-setup-0.6.2.exe`（93191780 字节，SHA-256 `90061902fb4454b62bbae4901cb7685bf38ee88dcd9b0642aaaf9ba76af0e130`）。
- **v0.6.2 验证**。`scripts/build_rust.py --strict --zip` 与 Inno Setup 7 编译成功。解包执行 `scripts/smoke_rust.py --zip <包> --timeout 45`，照片和视频检查、规则触发、PPE 回退、告警截图、抑制审计及退出清理均通过。定向前端 28 项、Vite 构建与 147 条前端 API 契约通过，已登记 OpenAPI 路由 134/134。交互安装和生产环境多路长跑留待大版本验收。
- **LS-082**。照片和视频共用可调整的区域锚点。新区域默认 12 帧离场宽限，历史区域保留全局迟滞。支持画运动遮罩和目标过滤遮罩，从有位置的误报事件记住同类误报点，在区域设置中移除，并提供独立的抑制审计查询。固定遮罩里的静止轨迹降频时仍按运动预筛的心跳重检。
- **LS-082 验证**。`cargo test -p ls-core spatial_`、`cargo test -p ls-analyzer motion_mask`、旧库往返迁移和四组服务端定向测试通过。相关 Rust crate 的 Clippy、`npx vitest run tests/zones.test.js tests/events_keys.test.js`（9 项）、前端构建和 146 条前端 API 契约检查通过。Vite 配置和 jsdom 的 Canvas 提示仍在。
- **已验证的第一批实现**。修复照片人数规则的区域、排班和等级判定，视频区分推理失败和有效零检测，PPE 状态与分析共用模型选择。抽出推理后端接口，增加同次 COCO 上下文取框、PPE 唯一归属、YOLOE 正负复核、灰度裁剪增强和遮挡断轨。
- **验证**。使用本地发行目录 ONNX Runtime 1.20.1 执行 Rust fmt、Clippy 和工作区全特性测试，205 项通过、4 项忽略。默认安装环境的旧 ONNX Runtime 首次拒绝 IR 10，切换随包 DLL 后通过。重新构建 `ls-server`，在临时安装布局运行 `python -X utf8 -B scripts/smoke_rust.py --install-dir <临时目录> --timeout 45`，照片规则、视频完成与推理失败对照、事件预览及退出清理通过。
- **GitHub**。首批修复 `55ec098` 已直连推送，`git ls-remote --heads origin main` 已确认远端提交。版本号仍为 0.6.0，推送限主线。
- **补充验收**。Rust 三连复验通过。扩展临时安装目录的 `smoke_rust.py`，实测自定义 PPE 文件丢失后随包模型真正加载，人员模型完全缺失及逐帧失败均留下检查记录和错误终态。
- **模型试验**。已有 YOLOE 的裁剪方案仍有误判。原生 256 双窗导出及 CPU 对照成功，14 个人框中确认 10 个，随包方案确认 12 个，双窗未补出额外结果。候选权重和一次性实验环境位于忽略的 `Resources/runs/实验服输入尺寸对照/`，开发始终采用推理和导出。见 [裁剪候选](findings.md#ls-080-裁剪候选第一轮验证)。
- **LS-081 已验证**。区域、规则和排班保存后，下次检测使用新版配置。照片目录运行中删除规则后仍留检查记录，新增规则后按新等级生成告警。区域设置页显示分析任务的实际生效状态。已锁定的人员权重通过校验，临时损坏缓存触发重新准备，严格构建在编译前停止。退出正常的临时 ZIP 冒烟包含新前端、PPE 回退、规则热更新和视频失败对照。
- **验证命令**。Rust 三连通过（205 项通过、4 项忽略）。`npx vitest run` 101 项通过、`npm run build` 通过。`contract_audit.py` 匹配 141 条前端路径，`rust_route_coverage.py --exe C:/Users/User/devtools/ls-target/debug/labsentinel-server.exe --fail-under 100` 为 126/126。前端仍有既有 Vite 配置、表格结构和 jsdom Canvas 提示。
- **安装器**。Inno Setup 7 从同一临时暂存目录编译通过，退出码 0，验证产物 154.2 MB。交互安装仍在后续验收范围。
- **投递与处置**。启动时恢复历史的待投递项，保留重试退避和已发送记录。测试检查记录可查、告警统计排除检查、AI 复核的展示与筛选一致。Rust 三连通过（207 项通过、4 项忽略），前端 101 项和构建通过。
- **值守验收**。前端实时预览共享五个流媒体名额，超额使用快照。断线后按持久化事件 ID 有序重放并去重。大屏可显示各阶段 P95 耗时，三档灵敏度同时调整人员和 PPE 阈值。Rust 路由定向 17 项、前端定向 26 项通过。
- **Next First Command**。`Get-CimInstance Win32_VideoController`，核验设备后准备可复现的 GPU 基准输入和后端。

## Previous Handoff Snapshot · LS-076

- **Task-ID / Task-Name**。`LS-076`（`DONE`），`v0.6.0` 基线缺陷审查与 1.0.0 路线图补充。本地 `main @ 8dca517`，产品基线 `2fef1b4`。
- **交付**。在 [正式路线图 §1.3](../plan/v1.0.0-roadmap.md#13-ls-076-缺陷与发布验收) 登记 9 项缺陷，写入修复归属和发布验收条件。[缺陷证据](findings.md#ls-076-v060-基线缺陷审查) 包含代码位置和实测结果。开始新模型对照前，先完成 `LS-080` 的文件推理、规则和 PPE 状态回归。`LS-081` 处理规则热生效、投递恢复、统计以及双产物验收。
- **实测**。执行 `python -X utf8 -B "$env:LOCALAPPDATA/Temp/labsentinel-ls076-audit.py" baseline`、同脚本 `video` 和 `delivery` 三个子运行。均以回环服务、临时库和项目验收素材复现预期缺陷。正常模型 15 帧均检到人员，输出不兼容的模型 15 帧推理均失败仍显示完成。照片忽略区域、排班和等级。新增规则至重启才生效，删除后仍触发。另复现 PPE 状态误报、AI 复核不可见、检查留痕污染处置计数和遗留 `pending` 行的手动重试 404。
- **发行核验**。以 Python 临时文件调用 `prepare_person_model(..., force=False)`，无效缓存被接受。通过 ZIP 清单和 SHA-256 核对，实测二进制与 ZIP 一致，当前人员模型符合锁定值，便携包及安装器来源目录缺少 WinSW。源码另确认告警事件及投递写入间隔内的崩溃窗口。
- **文档核验**。`git diff --check` 通过。路线图与发现表的 9 个编号一致，任务表中 `LS-076` 为 `DONE`，22 个实施任务为 `READY`。
- **范围**。本轮只更新本地规划与交接文档。复现采用已有二进制和 CPU。现场相机、DirectML 长稳、外部渠道与交互安装保留到对应任务验收。实现、打包、训练和 GitHub 操作暂停。
- **Next First Command**。待用户恢复后执行 `git status --short --branch`，再按路线图 §1.3 复现 `LS-076-01/03/04` 并锁定 `LS-080`。`LS-080`～`LS-101` 当前为 `READY`。

## Previous Handoff Snapshot · 1.0.0 规划入库

- **Task-ID / Task-Name**：`v0.6.x → v1.0.0 GA` 长线规划入库（`LS-080` ～ `LS-101` 登记为 `READY`；现网基线维持 `v0.6.0` / `LS-075` `DONE`）。`main`，本地工作区。
- **本轮交付**：完成 18 个指定项目/论文/文章 + 15+ 全网顶级标杆（Frigate v0.18、AmberSight、Moonfire/Retina、Construction-Hazard-Detection、YOLO26/YOLOE/PP-LCNet、Qwen3-VL/SmolVLM2、`sqlite-vec`）的深度对标；经 Swarm 三轨架构规划与实码核验，正式入库 [docs/plan/v1.0.0-roadmap.md](../plan/v1.0.0-roadmap.md)（9 个渐进版本 `v0.6.1 → v1.0.0 GA`、22 项任务 `LS-080`～`LS-101`、SQLite `v22 → v32`）、[docs/adr/0012-unified-vulkan-gpu.md](../adr/0012-unified-vulkan-gpu.md)（`Proposed`），并同步更新 `task_plan.md` 与 `task_registry.md`。
- **已锁定方向**：无 PDCA 隐患工单；统一推进 Vulkan GPU（CV + 离线托管 `llama-server.exe` VLM，保留 `DirectML + dml_lock → CPU` 回退）；细粒度逐工具可配高权限本地 MCP；性价比带识图 AI 辅助报告/多轮问答历史（DeepSeek/Qwen-VL-Flash 等）；现成合规权重拿来即验 + 7 大训练配方（严禁 SH17 等 NC 进随包权重）。
- **Next First Command**：确认 `v0.6.0`（`LS-070` 现场联测/`LS-068` 发版）收口时机后，锁定 `LS-080`（`v0.6.1`）启动实施。

## Previous Handoff Snapshot · LS-075

- **Task-ID / Task-Name**：`LS-075`（`DONE`）/ 候选模型对照、合并与淘汰。`main`，本地工作区。
- **交付**。将随包 PPE 切到 LAB PPE 专项版，使用 YOLOE 对已穿白大褂做复核。两者均在 Rust ONNX 路径运行；阳性框须唯一归属到人员，证据不足保留未知，明确未穿独立保留。近景口罩开关仍可单独试用。移除指向旧权重的下载入口。
- **整包对照**。同一批 59 张照片，旧包 57 个人框全部判已穿、0 条未穿；新包仍检出 57 个人框，已穿 2、未穿 3、未知 52，59 张均有检查记录。原监控视频完成检查并多产生 1 条带截图的未穿实验服事件。CPU 两次文件用时均约 28 秒。结果限定于这批样本。额外试用：近景口罩隔离开关对照通过；同批现场照片用新基础模型时，2 条口罩告警开启补判后数量相同。
- **淘汰与后续**。AIZOO 对俯拍小脸不稳定，Grounding DINO INT8 CPU 中位 2.15 秒／帧，人员 INT8 在现场照片只给出 24 个人框、当前模型 62（独立推理统一 0.35 阈值），且更慢。未选入发布。化学实验室 YOLO 作者权重下载超时，动作和身份替代模型仍待单独实测，未计入性能淘汰。
- **验证**。Rust fmt、Clippy、workspace 全特性 203 通过／4 忽略；Vitest 101 通过，Vite 构建、API 契约 141／178、Python 编译及 `git diff --check` 通过。`build_rust.py --strict --zip` 和 `build_setup_exe.py` 完成。最终 ZIP 和安装目录布局以 `smoke_rust.py --mask-fixture` 均通过检查完成、证据截图、规则和近景口罩补判对照。
- **产物**。`Resources/dist/labsentinel-rust-0.6.0-20261123.zip` SHA-256 `6220523B343132421B220E5EC73B718426CB65EB7418A65E63244F24BEEC5470`。`Resources/dist/LabSentinel-setup-0.6.0.exe` SHA-256 `8DC05227A27934829DAEC53A0FB040391CEE2FF73541DA41BFEBAABCCF3BA470`。
- **限制**。只做推理与导出，本机未训练。现场 DirectML 多路性能和覆盖安装仍待实测。生产实例未覆盖，GitHub 发布暂停。

## Previous Handoff Snapshot · LS-074

- **Task-ID / Task-Name**：`LS-074`（`DONE`）/ 现成检测权重对照与 0.6.0 本地随包验证。`main`，本地工作区。
- **交付**。通过 Ghost Downloader 下载并校验 MIT 权重 YuNet 与 MobileNetV2。两种发行产物均随包提供。设置可打开「近景口罩检测（试用）」，默认关闭。已有 PPE 无口罩证据时，近景模型按唯一的人框归属补判，未戴置信度至少 0.99，戴口罩至少 0.95。共用 Rust ONNX Runtime 与 DirectML 串行锁。
- **模型对照**。CPU 模型独立测试中，现场 59 张照片基础模型 0 张未戴，近景模型 7 张，经逐张核对；16 张具备可分类的人脸，其中 9 张信心不足。俯拍视频每 2 秒抽 1 帧，共 32 帧，整帧路径无可用人脸；按 91 个人上半部裁切找到 23 个脸框，只有 2 个达到 40 像素且均未达到判定阈值。Elisa/mask_kaggle 公开验证集 180 张中，未戴 45/60 检出，正确佩戴 0/60 被标为未戴。佩戴不规范样本未验证成独立识别类别。无经验证的实验服替代权重，未引入额外运行环境。
- **验证**。`cargo fmt --all -- --check`、Clippy 全特性无警告、Rust 202 通过／4 忽略，Vitest 102 通过，Vite 构建、`contract_audit.py` 141／178、三个打包脚本 Python 编译、`git diff --check` 均通过。`build_rust.py --strict --zip`、`build_setup_exe.py` 通过。`smoke_rust.py --zip` 和 `--install-dir` 均用 `--mask-fixture` 对照，照片和视频关闭时只写检查记录，打开后均产生带截图的未戴口罩告警。
- **产物**。`Resources/dist/labsentinel-rust-0.6.0-20261123.zip` SHA-256 `FA654FDA55C7A37EBD1D7A2B22BED65D5387FD500355E2133806FF11C259A261`。`Resources/dist/LabSentinel-setup-0.6.0.exe` SHA-256 `F2B99BCA9677B16100A39B060CD913492F1867107D91E6B42F02D47984D0522F`。
- **待现场**。保留 GitHub 发布暂停。仅隔离推理与打包，生产实例未覆盖，本机未训练。DirectML 多路和真实覆盖安装待现场安排。

## Previous Handoff Snapshot · LS-073

- **Task-ID / Task-Name**：`LS-073`（`DONE`）/ 视觉模型与数据调研、标注训练集分组导出。
- **范围**。本地学习使用。复用官方模型和数据候选，增加标注分组、分组或统一导出、排除及确认删除。GitHub 发布暂停。
- **现场观察**。用户开启乐橙实时预览，见实验台高位俯拍、门口、办公区，以及彩色、灰度与特殊光照片段。遮挡与小目标支持按机位选择规则、近景口罩和人脸。本轮未保存监控素材、未加入标注池、未训练模型。
- **验证**。Rust fmt／Clippy／202 项测试通过，4 项忽略。Vitest 101 项、Vite 构建、训练工具 13 项、契约 141／178。Chromium 联测训练集选择、排除下载和确认删除。ZIP 与安装目录布局冒烟均通过。
- **本地产物**。ZIP SHA-256 `8FC0C1A6CF5FB746BE02B85918C72C8519606769608913AFB8680C3ABD348042`。安装器 SHA-256 `4CE29173784F6E4DA91D1BEF9611A922F017ED76CE118A2C2EAB7F1F1DCD88DE`。候选模型未替换现用权重，训练及真机模型验收留待按用户指定条件执行。

## Previous Handoff Snapshot · LS-072

- **Task-ID / Task-Name**：`LS-072`（`DONE`）/ 事件详情过渡、检查图片预览与标注。
- **范围**。用户反馈详情闪白闪黑，检查事件无法预览标注。隔离验收，GitHub 发布暂停。
- **实现**。移除抽屉的整页 View Transition，保留遮罩变暗与面板滑动，支持减少动态效果。图片检查事务写入原图关联，旧库按来源、时间与文件名恢复唯一关联。视频重跑后保留当次截图。图片找不到时提示不可用。
- **验证**。旧前端回归复现调用整页动画，旧 ZIP 冒烟复现照片截图接口 404。新前端 Chromium 深浅色均通过，Vitest 101 项。Rust fmt／Clippy／201 项测试通过，4 项忽略，契约 141／178。ZIP 和安装目录布局均通过预览、标注及视频留痕冒烟。独立 v21 数据库升级后，旧图片事件可取原图和缩略图，重复标注返回同一样本。
- **本地产物**。ZIP SHA-256 `9CFC02F55FB0DC0A8193430BDE7995DB79272E9546E4CC090CAB61DCB7C56C01`。安装器 SHA-256 `29B2B86CD48900346F1299AEBC99667D6EA008C1A0EA3E561FD2DF135B385751`。仅本地重打包，现场未覆盖安装。

## Previous Handoff Snapshot · LS-071

- **Task-ID / Task-Name**：`LS-071`（`DONE`）/ 本地图片和视频检查留痕及事件分类。
- **目标**。用户要求正常检查也写入事件。检查与告警独立分类，图片与视频任务核对真实写入和筛选。现场实例只读，GitHub 发布保持暂停。
- **实现**。图片检查结果与检查事件一起提交，完成、异常或已处理后被停止的视频分析留一条检查事件。已完成旧照片从持久化结果一次性补录，旧视频需重新分析。事件列表、计数和 CSV 共用分类条件，视频页展示已检查帧数及检查记录入口。
- **现场隔离验证**。旧库样本经新二进制自启动补入 59 条照片事件。原视频无规则重跑，生成一条检查记录，处理 321 帧，316 帧检出人员，0 条告警。按视频和检查分类导出只含该视频记录。现场库未改动。
- **回归**。`cargo fmt --all -- --check`、Clippy 全目标全功能、全量测试通过，201 项通过、4 项默认忽略。Vitest 100 项、Vite 生产构建、契约 141／178、Python 编译、`git diff --check` 通过。便携 ZIP 与安装目录布局均通过 `scripts/smoke_rust.py`，检查覆盖无规则照片、无规则视频、有规则视频、分类和优雅退出。
- **本地产物**。`Resources/dist/labsentinel-rust-0.6.0-20261123.zip` SHA-256 `9CD73F09CCB62A80A1A5C1EADA8C123458826D27ABB1041135C7C3AF49AB32FF`。ISCC 编译的 `Resources/dist/LabSentinel-setup-0.6.0.exe` SHA-256 `94960E0B461A1F03EDF725096B6C59C5E8FF58C924170C81AADB0F36C0B10DDE`。现场部署待用户操作，PPE 召回需单独改进。

## Previous Handoff Snapshot · LS-070

- **Task-ID / Task-Name**：`LS-070`（`READY`）/ 现场检测分类、人脸告警、大华接入与卸载清理。
- **范围**。继续 v0.6.0 本机验收。GitHub 发布暂停，本轮未操作现场安装和数据。
- **检测**。随包 PPE 模型同时输出实验服和口罩证据。视频支持独立未戴口罩规则，图片列表与详情显示实际分类。模型关闭或证据不足时不生成未穿实验服事件。
- **人脸与取流**。每路视频源可在授权范围内开启匹配告警。同设备同人 90 秒限频，管理员查看身份信息。大华新增局域网 RTSP 通道与主辅码流配置。
- **卸载**。现场扫描 283 行中 254 行来自三处开发测试临时目录。卸载按安装路径清理进程、启动项和快捷方式，默认保留现用及旧布局数据。照片、视频、安装卸载和数据选择已加入发布验收。
- **验证**。`cargo +stable-x86_64-pc-windows-gnu fmt --all -- --check`、`clippy --workspace --all-targets --all-features -- -D warnings`、`test --workspace --all-features --no-fail-fast` 通过，200 项通过、4 项默认忽略。`npx vitest run` 100 项、`npm run build`、`python -X utf8 scripts/contract_audit.py` 契约 141／178、Python 冒烟脚本编译通过。
- **包内回归**。`python -X utf8 scripts/smoke_rust.py --zip Resources/dist/labsentinel-rust-0.6.0-20261123.zip --timeout 40` 与 `--install-dir Resources/dist/rust/labsentinel-server` 均通过真实照片检人、视频首帧、检查完成、规则事件和优雅退出，两个临时目录均回收。`node web/scripts/smoke_webrtc.mjs Resources/dist/rust/labsentinel-server` 通过 RTSP 主辅流、WebRTC、快照回退、MSE、设备授权与密码会话。期间修复同一设置页分区深链未刷新的问题。
- **本地产物**。`Resources/dist/labsentinel-rust-0.6.0-20261123.zip` SHA-256 `5DC1DE6AE6411040EFC8159EC3F980DF8372561657B445B5D2F8D471910A21E4`。`Resources/dist/LabSentinel-setup-0.6.0.exe` SHA-256 `695B76D873C9D2353F3F5E8FC70C4E981979F8E354FA3310E77B6707505F25D9`。ISCC 7 编译通过，真实安装卸载组合命令被本机执行策略拒绝。
- **Next First Command**：用户本地联测大华实际设备、白大褂与口罩现场影像、人脸库和真实安装卸载。保持 GitHub 发布暂停。

## Previous Handoff Snapshot · LS-069

- **Task-ID / Task-Name**：`LS-069`（`DONE`）/ 现场 v0.5.0 图片结果、视频文件选择与安全区底图，含发布前检测门禁。
- **现场实证**。v0.5.0 的 59 张图片均解码成功。直接运行随包人员模型 634279b4 得到 opset 22 超过运行库上限 21 的错误。隔离的 v0.5.0 切换已验证模型 4758511e 后，59 张有 56 张检出人员。现场同一段 AV1 MP4 解码成功，隔离实例用验证模型和进入规则生成 27 条事件。生产 v0.5.0 通过现有模型配置接口切换至验证模型，重载了 1 路。已有图片记录仍保留先前的分析结果。
- **交付与门禁**。视频文件选择器、未启动可看的首帧、`检查完成`、展示模型加载失败和真实图片汇总。构建时把锁定的上游人员模型转换为 opset 21 并校验 SHA-256。便携包和安装目录两种入口均要求有人的照片检出人员，同时验证视频首帧、完成状态和真实规则事件。
- **验证**。旧二进制与原模型测试包在新门禁的检测步失败并退出 1。修正后的本地测试包通过 `python -X utf8 scripts/smoke_rust.py --zip engine-rs/target/ls-qa/ls069-fixed.zip --timeout 30`，同一产物以隔离安装目录布局运行 `--install-dir` 同样通过。Rust fmt／Clippy 和全量 197 项测试通过，4 项默认忽略。Vitest 100 项、生产构建、契约 141／178、Python 脚本编译与 diff 检查通过。
- **交接**。改动在本地工作区，生产实例继续使用原 v0.5.0 EXE。发布和余下 v0.6.0 开发仍暂停。恢复后从 `git status --short --branch` 核对本地差异，发布验收时重跑真正的安装器流水线。

## Previous Handoff Snapshot · LS-067

- **Task-ID / Task-Name**：`LS-067`（`DEFERRED`）/ 告警投递与值班交接、合规报表、小样本训练预检。
- **暂停点**。用户正在生产环境测试，要求发版前暂停。`main` 最新本地提交为 `92cfa67`，领先本地 `origin/main` 记录 7 条。`LS-067` 源码和手册保留在本地工作区，尚未提交。没有推送本批提交、打 `v0.6.0` 标签或启动发布。
- **当前进度**。工作区包含告警带图重试与历史、小样本 ZIP 和预检、无人 Cascade 夹具、断流与恢复事件、交接备注、按样本统计合规率的服务端改动和部分界面。规则演练、合规报表界面、事件投递与断流实证、API 契约和最终发布验收待续。
- **已执行**。`python -X utf8 -m unittest discover -s tools/tests` 为 13 项通过，相关脚本语法检查退出 0。最后一轮 Rust fmt 通过，Clippy 在日报发送接口缺少新参数处停止，已补接口调用，待重新运行强制三连。当前 Web 改动待 Vitest、构建和浏览器实测。
- **现场接入**。优先取授权录像机或摄像头的内网 RTSP，用实际通道验证首帧。已核对大华和乐橙官方接入资料及当前 Rust 的 RTSP／RTSPS 入口，待现场确认型号和网络。
- **Next First Command**：恢复后执行 `git status --short --branch`。先把摄像头离线筛选移入列表并核对交接和统计界面，再跑 Rust 强制三连及其余回归。正式包和现场验证归 `LS-068`。

---

## Previous Handoff Snapshot · LS-066

- **实现**。关闭、本地 YuNet／SFace、多个 CompreFace 配置可热切换。网页支持录入、撤回、阈值设置、试识别及模型回退，默认关闭。授权相机可为事件附带身份线索，人员名单与匹配结果限管理端。本地加密模板、有效期和 API 删除重试随 v19 启用。
- **实测**。Windows GNU `cargo fmt --all -- --check`、Clippy 全目标全特性及 `cargo test --workspace --all-features --no-fail-fast` 通过，196 项，3 项默认忽略。旧库缺少分析码流列时仍能读取及导入相机。单独运行本地人脸夹具的 CPU 和 Radeon 880M DirectML 场景均通过，同人分数 0.556、异人 0.163，CPU 57.80–76.87 ms，DirectML 47.93–77.93 ms。模拟 CompreFace 的录入、切换、撤回及删除重试通过。
- **前端与发布检查**。Vitest 99 项、生产构建、契约 137 条 API 与 175 条路由登记通过。真实 Chromium 在 375/1440 px 使用模拟接口验证本地选择、录入和试识别。正式包媒体数据路径、实际外部 CompreFace 和用于准确率、误接受率及误拒率的代表样本验证纳入 `LS-068`。

---

## Previous Handoff Snapshot · LS-065

- **实现**。RTSP／USB 最新帧单槽、覆盖和帧龄计数，JPEG 直接 BGR 编码，减少模型解码分配并继续串行保护 DirectML。按模型声明处理新版和旧版 RT-DETR，网页可选择 PPE 输出格式与候选格式。增加高画质预览与子流分析、主码流共享及回退。检测数据按相机广播，Canvas 随数据重绘，画面离开视口或标签页隐藏时停止播放与快照轮询，同帧快照返回 HTTP 304。
- **实测**。`cargo fmt --all -- --check` 和 Clippy 通过。`cargo test --workspace --all-features --no-fail-fast` 的 43 个实际执行用例通过，7 个目标被 Windows 应用控制以 4551 阻止。Web 98 项、生产构建、播放脚本语法、导出工具语法及契约（128 条调用，166 条路由）通过。
- **验收**。保持 `ceb1ebb` 作为优化前基线。包内浏览器脚本已加入子码流、直连回退和共享中继。真实 RT-DETR 导出、完整 CPU／DirectML 对照、关键事件真值、多路和 24 小时压力及正式 EXE 联测待可执行环境，当前没有可宣称的整链提速幅度。

---

## Previous Handoff Snapshot · LS-064

- **实现**。有界阶段采样、RTSP 1／4／9 路基准、识别真值回放、Windows 进程树资源采样与复跑命令。
- **实测**。Windows `fmt --check` 与 Clippy 通过。FFmpeg 生成 20 秒 1080p H.264 视频，ffprobe 确认 240 帧。包内 go2rtc 提供 RTSP，1、4、9 并发的 FFmpeg 均正常完成，10 秒媒体窗口输出 46–50 帧，实际耗时 11.772–11.910 秒。详情见 `docs/guide/性能验收.md`。
- **回归**。`cargo +stable-x86_64-pc-windows-gnu test --workspace --all-features --no-fail-fast` 实际执行的 118 项通过，两个性能用例默认忽略，六个目标被应用控制以 4551 拒绝启动。
- **阻塞**。Windows 4551 拦截 Release 的依赖编译脚本和 Debug 基准程序，批准执行的开发 Python 加载独立 NumPy 模块时也遇到应用控制。完整 Rust CPU／DirectML 场景、识别真值与 24 小时压力复测保留为发行门禁。

---

## Previous Handoff Snapshot · LS-063

- **实现**。浏览器扫码和 6 位码认领，设备 Cookie、只读或处置角色、摄像头及分组范围、有效期、改名和撤销。v17 迁移保存设备授权，旧版令牌兼容。大屏记住分组和布局，允许授权设备处置事件。服务器检查 HTTP、事件 WebSocket、检测和视频，权限或分组变化后会断开存量连接。
- **本机门禁**。Windows `fmt --check` 与 Clippy 通过。`cargo test -p ls-server -p ls-store --all-features --no-fail-fast` 共 91 项通过。其中浏览器设备用例通过真实本机 WebSocket，验证只收到授权事件，更新授权后原连接断开，撤销后列表和新连接拒绝。全量 `cargo test --workspace --all-features --no-fail-fast` 172 项通过，另有 `dispatch_integration`、`ls-ingest`、`ls-jobs` 目标被 Windows 4551 拒绝执行。
- **前端实测**。Vitest 96 项与生产构建通过，生产依赖审计 0 项。Chromium 使用生产前端与模拟接口验证生成二维码、375 px 配对、分组、确认事件、到期提示和无横向溢出。契约审计为 127 条前端 API 匹配 165 条路由登记。
- **发行门禁**。`smoke_webrtc.mjs` 已加入真实相机画面与设备撤销场景。最终双产物运行时完成浏览器端联测，当前本机独立 EXE 有 Code Integrity 阻塞。

---

## Previous Handoff Snapshot · LS-062

- **实现**。访问控制网页支持本机与局域网分开开启密码、Argon2id 密码摘要、HTTPS 反代、12 小时浏览器会话、注销与批量撤销、仅本机一次性恢复码。媒体与 WebSocket 继承鉴权，浏览器会话使用可撤销中继，原 API 令牌仍适用。
- **实测**。首次 Windows GNU `fmt --check`、Clippy 和 `cargo test --workspace --all-features` 通过，共 190 项。包含本机密码、既有令牌、会话撤销、代理边界和恢复码的三项新增路由测试。最终改动后 `fmt --check` 和 Clippy 通过，Web 95 项、生产构建和 `node --check scripts/smoke_webrtc.mjs` 通过，契约审计为 123 条前端 API 匹配 162 条路线登记。
- **验收阻塞**。`node web/scripts/smoke_webrtc.mjs <staging>` 和独立 HTTPS Chromium 联测无法启动新 EXE。`python -X utf8 scripts/rust_route_coverage.py --exe <debug> --fail-under 95` 返回 Windows 4551。Code Integrity `3033` / `3077` 确认签名策略拒绝。此后的全量 `cargo test` 执行 27 项后，运行 `ls_core` 测试程序亦返回 4551。仓库没有自托管 GitHub Windows runner 或已知签名连接。最终 Windows 浏览器、路由覆盖和安装验收须在可运行该产物的环境重跑。

---

## Previous Handoff Snapshot · LS-061

- **界面**。首页展示运行、连接、离线和停止相机数，当前版本读 `/api/system`，更新入口直达 GitHub Release。模型与检测设置显示最近加载的推理设备，报表和侧栏移除了遗留标签。告警配置采用渠道列表与同位置编辑器，SMTP 并排字段、Webhook 行内开关、各渠道草稿与测试状态独立。
- **门禁**。Windows GNU Rust 三连退出 0，共 187 项。`npx.cmd vitest run` 16 文件、94 项通过，`npm.cmd run build` 退出 0。`python -X utf8 scripts/contract_audit.py` 为 116/155 全匹配。
- **Chromium**。隔离的 Rust 服务运行前端生产构建，375、768、1440 px 均覆盖浅色和深色。SMTP 字段位置、Webhook 开关、键盘焦点、保存后草稿、无页面横溢出、版本外链与摄像头入口通过。实际发送测试消息使用 `LS-067` 的受控投递场景。

---

## Previous Handoff Snapshot · LS-060

- **实现**。资产锁定 go2rtc 1.9.15 和 MIT 许可，打包从干净资源目录取得网关。Rust 监管进程和注册列表，修正 FFmpeg RTSP 的 `-timeout`，按真实首帧切换播放。只读分享使用服务端可中断的 MSE 中继。首次升级按实际 `VERSION` 恢复旧版，网络设置写入数据目录，端口冲突保留监听，开发启动改用托管网关。
- **门禁**。Windows GNU `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`cargo test --workspace --all-features` 均退出 0，共 187 项 Rust 测试。`npx.cmd vitest run` 93 项通过，`npm.cmd run build` 退出 0。`python -X utf8 scripts/contract_audit.py` 为 117/155 全匹配且 Inno BOM 正常，`rust_route_coverage.py --fail-under 95` 为 106/106，`--fail-under 101` 按预期退出 1。
- **浏览器与部署实测**。`node web/scripts/smoke_webrtc.mjs <隔离 staging>` 用随包 FFmpeg、go2rtc、ORT 和实验服模型验证 RTSP 解码、WebRTC 首帧、网关故障降级和恢复、分享 MSE 解码与服务端吊销，吊销后的快照返回 401，管理员画面继续播放。基于 v0.5.0 官方 ZIP 的隔离往返验证旧 EXE、前端和数据保留，通过占用目标端口与有效端口重启复测。资产下载和文件哈希已验证。
- **发行门禁**。正式 CI 的干净构建、双包 Chromium 以及安装升级回滚卸载留到 `LS-068` 以最终构建运行，标准账户安装现场同属发布验收。

---

## Previous Handoff Snapshot · LS-059

- **Task-ID / Task-Name**：`LS-059`（`DONE`）/ 后续功能、UI 与高性能版本规划。
- **基线**：`main @ a992a42`，延续上一轮尚未提交的五份接力文档。按本轮 `/plan` 更新接力文档，实施任务分批登记为 `READY`。
- **规划**：拟定 `0.5.1` 修发版、实时播放、回滚和首轮 UI，`0.6.0` 完成密码、设备配对、运维与性能基线，`0.7.0` 做多路优化和人脸本地／API 模型管理。上一轮 12 项发现纳入实施任务。详细依赖与通过条件见 `task_plan.md` 的 `LS-060`～`LS-067`。
- **实测**：正式 ZIP 在独立数据目录启动。未设 `LS_UPDATE_MANIFEST_URL` 时，`GET /api/update/check?refresh=1` 返回 `enabled=false`、`current=0.5.0`、`latest=null`、`url=null`，随后服务正常退出。`gh release view` 确认 `v0.5.0` 为最新正式发布。检查更新的网页默认路径规划为直达 Release。
- **参考核验**：`Get-Item`、`Get-AuthenticodeSignature` 和 `Get-FileHash` 确认 EXE 元数据为 FaceWinUnlock-Tauri 0.4.0、未签名。`gh api repos/zs1083339604/FaceWinUnlock-Tauri/releases/tags/0.4.0` 的资产 SHA-256 与本机一致。参考限于文件属性和官方公开的操作流程。
- **Next First Command**：锁定 `LS-060` 后运行 `rg -n 'go2rtc|route coverage' scripts/prepare_rust_assets.py scripts/build_rust.py .github/workflows`，从正式包资产与发布门禁切入。

---

## Previous Handoff Snapshot · LS-058

- **Task-ID / Task-Name**：`LS-058`（`DONE`）/ v0.5.0 接手、全仓复审与正式产物独立验收。
- **基线**：启动时 `main @ a992a42`，工作区干净，Git 提交人 `windyxyz`。重新查询确认远端 `main` 与远端 `v0.5.0` 指向同一提交。[公开 Release](https://github.com/windyxyz/LabSentinel/releases/tag/v0.5.0) 于香港时间 2026-11-22 18:38 发布。
- **审查判断**：源码和发版门禁基本通过。正式产物缺 go2rtc，现有 WebRTC 信令与 1.9.15 不兼容，首次覆盖升级后回滚可失败。本批完成复审与交接记录，待修复事项及验收条件落在 `findings.md` 的 LS-058 小节。
- **本机门禁**：在 `engine-rs` 按 Windows GNU 和纯 ASCII target 配方依次运行 `cargo +stable-x86_64-pc-windows-gnu fmt --all -- --check`、`clippy --workspace --all-targets --all-features -- -D warnings`、`test --workspace --all-features`，三步均退出 0，Rust 184 项通过。Web `npx.cmd vitest run` 15 文件、93 项通过，`npm.cmd run build` 退出 0。`python -X utf8 scripts/contract_audit.py` 通过，117 条前端 API 匹配 155 条后端清单项，安装脚本 BOM 正常。`python -m unittest discover -s tools/tests -v` 11 项通过，`python -m py_compile` 对 `tools/*.py` 退出 0。
- **路由与 CI**：`python -X utf8 scripts/rust_route_coverage.py --exe Resources/dist/rust/labsentinel-server/labsentinel-server.exe --fail-under 95` 退出 0。该统计与 `gh run download 45010145971 --name rust-route-coverage` 一致，106/106，另有 46 条 OpenAPI 清单外路由。`gh run list --commit a992a425a40e0eb49bc72b3543e3ad80b3e77df0` 查到 `tests`、`rust`、`release` 均成功。发布工作流记录包含依赖审计、Windows 静默安装、探活与卸载。本次 Windows 本机执行的是解包及命令行回滚复测。
- **正式产物**：用 `gh release download v0.5.0` 独立下载两件产物和校验清单，`Get-FileHash -Algorithm SHA256` 与发布校验清单一致。ZIP 89,253,263 字节，`1c6abe7755a68a63265a4f18ca7a44461af8da9571cf8a4a6dff4c6080f52f53`。安装器 69,108,068 字节，`3e7472c44798a216641478a5c238a70aa149bd4fbd297d6de15beef72f4ff83b`，Authenticode 为 `NotSigned`。
- **本机同名产物**：ZIP 100,430,452 字节，`5c2d9bdb9f27057cfe8470c7f4fa06d8dffc6169732a330239b4c735dfdafe01`。安装器 75,127,189 字节，`ce06f58a1a61559082eb1d6df0fbb01c4c554c48d0d1714053a6cfa79c5ca526`。时间戳早于 `a992a42`。两份 ZIP 均通过 `ZipFile.testzip()` 和 `python -X utf8 scripts/smoke_rust.py --zip <文件> --timeout 90` 的 5 项启动、API、界面与退出检查。
- **隔离复现**：本机随包 go2rtc 1.9.15 对现有前端 offer 返回 JSON 类型不匹配。正式 ZIP 的服务在 CPU/ORT 上完成同一随包 PPE 模型双路评测，HTTP 200，两个模型会话均返回 `640×640` 和检出。单个已核验样本经正式包 API 导出 HTTP 200，`train=0, val=1`。Python 小样本切分 1～4 张均没有验证样本。正式 exe 在模拟升级目录中归档 0.4.0 返回 0，写入 0.5.0 后回滚返回 2。
- **VLM 本机模拟**：隔离运行 `scripts/acceptance_vlm.py` 的验收逻辑，服务器使用正式 ZIP 二进制，配置 CPU、回环 mock 和独立 DB，通过启动包装去掉打开浏览器的 `--launch` 参数。原素材版第 11b、12 步失败，检查发现 `empty-frame.jpg` 含人员。临时用合成无人图替换后整套通过，空场景 mock 调用保持 `2→2`，加入有人图后 `2→3`。
- **复审过程记录**：首次回滚探针传入 `--console` 为首参数，应用进入服务路径并创建默认空数据配置。两个测试进程已停止。确认相机、事件、设备、样本、设置及投递为零后，将测试创建的整个目录移至临时目录，默认路径复核为不存在。后续回滚探针使用独立测试路径。
- **现场待验证**：真实 RTSP／DirectML 多路与长跑、外部 VLM 和渠道、真实 RT-DETR 导出与全面训练，以及本机交互安装／升级／卸载、标准用户权限和本机设备矩阵。
- **Next First Command**：`rg -n 'LS-058-0[1-6]' findings.md`，从发版资产、真实 WebRTC、回滚和分享连接生命周期缺陷确定修复批次。

---

## Previous Handoff Snapshot · LS-050-057

- **Task-ID**: `LS-050-057`（`DONE`）
- **Task-Name**: LabSentinel v0.5.0 补齐与本机发布验收
- **基线**: `main @ 3ebda37`，启动时有 28 个未提交文件，含 v0.5.0 版本号和前次 Rust 修补。已在这些修改上继续。
- **本轮实现**:
  - LS-050 / LS-051：复验 Motion Gate、PPE 滑块、单人留守与 VLM 提示词。修复签名缩略图访问、JPEG 存证输入、go2rtc 参数和内存流同步、WS 鉴权与检测元数据、值守快照回退。
  - LS-052 / LS-053：已有安全区顶点可撤销与重做，保留可视化排班与事件快审。告警重试尊重退避并可立即消费，局域网配置与自重启生效，Windows 自启写入 HKCU Run 的命名值。
  - LS-054：快捷标注、人员预标注、样本隔离与 YOLO ZIP、诊断脱敏、只读令牌隔离与吊销。
  - LS-055 / LS-056：读取 ONNX NCHW 尺寸、FP16 输入输出、进程级 ORT 环境、逐类和逐图 A/B 评测，以及公开数据源、词表映射、混合训练和 FP16 导出。
- **Windows 实测**:
  - `cargo +stable-x86_64-pc-windows-gnu fmt --all -- --check` 与 `cargo +stable-x86_64-pc-windows-gnu clippy --workspace --all-targets --all-features -- -D warnings` 均退出 0。
  - `cargo +stable-x86_64-pc-windows-gnu test --workspace --all-features --no-fail-fast` 通过 184 项，0 失败。显式使用仓库随包 FFmpeg 和 ONNX Runtime DLL。
  - `npx.cmd vitest run` 通过 15 个文件、93 项测试。`npm.cmd run build` 生成 61 个转换模块，492ms。
  - `python -X utf8 scripts/contract_audit.py` 通过。117 条前端 API 匹配后端 155 条清单项，安装脚本 BOM 正常。
  - `python -m unittest discover -s tools/tests -v` 通过 11 项。`python -m py_compile` 对 `tools/*.py` 通过。
  - 隔离本机实测中，FP32 和 FP16 模型同进程运行，FP16 评测返回 512×256 和 1 个检出。无令牌开启局域网返回 422，更换端口后的子进程按新读取保护配置就绪。
  - 隔离缩略图实测通过。无凭证访问返回 401；使用对应路径签名获得 320×180 JPEG，第二次请求命中缓存，签名用于其他路径时返回 401。
  - `python -X utf8 scripts/bump_version.py 0.5.0` 输出同步成功。
  - `python -X utf8 scripts/build_rust.py --strict --zip` 退出 0，生成 100.4 MB ZIP。
  - `python -X utf8 scripts/rust_route_coverage.py --exe <release exe> --fail-under 95` 输出 `106/106 = 100.0%`，已实现 154 个唯一方法和路径组合。
  - `python -X utf8 scripts/build_setup_exe.py` 调用 Inno Setup 7，`rc: 0`，生成 75.1 MB 安装器。
  - `python -X utf8 scripts/smoke_rust.py --zip Resources/dist/labsentinel-rust-0.5.0-20261122.zip --timeout 90` 通过 5/5。返回 `ready`，`backend=rust`，`version=0.5.0`，`ppe_enabled=True`，SPA 静态资源可达，`shutdown=200`，进程退出码为 0。
- **发布产物**:
  - `Resources/dist/labsentinel-rust-0.5.0-20261122.zip`，100426313 字节，SHA256 `92E1AB7E9F3795345FF49B5AD814F5253A3B0BDE88B4337B25CF95C6C37D3147`。
  - `Resources/dist/LabSentinel-setup-0.5.0.exe`，75119342 字节，SHA256 `B056462260602A2044B32E44E873EFD6F35722D20FCC26FAFD59A3B4D71D625D`。
- **现场覆盖边界**: 本轮运行了 CPU FP16 合成模型、随包 go2rtc 注册和注销、ZIP 本机冒烟。真实 RTSP 长跑、GPU/DirectML 专项压力、外部 VLM 和渠道交付、公开数据全量训练，以及安装器交互安装/升级/卸载需要相应的外部环境。Vitest 输出 Vite native loader 提示和 jsdom Canvas 提示，Vite 构建退出 0。
- **Next First Command**: `git log -1 --stat` 审阅本地 v0.5.0 提交。正式发行资产由 CI 生成和上传。

---

## LS-050-057 本轮改动文件清单

```text
CHANGELOG.md
README.md
VERSION
deploy/windows/installer.iss
docs/dev/data-flywheel.md
docs/guide/安装与卸载.md
docs/plan/status.md
engine-rs/Cargo.lock
engine-rs/Cargo.toml
engine-rs/crates/ls-analyzer/Cargo.toml
engine-rs/crates/ls-analyzer/src/evidence.rs
engine-rs/crates/ls-analyzer/src/motion.rs
engine-rs/crates/ls-analyzer/src/pipeline.rs
engine-rs/crates/ls-analyzer/src/ppe.rs
engine-rs/crates/ls-dispatch/Cargo.toml
engine-rs/crates/ls-dispatch/src/lib.rs
engine-rs/crates/ls-infer/Cargo.toml
engine-rs/crates/ls-infer/src/post.rs
engine-rs/crates/ls-infer/src/preprocess.rs
engine-rs/crates/ls-infer/src/yolo.rs
engine-rs/crates/ls-ingest/Cargo.toml
engine-rs/crates/ls-ingest/src/child_guard.rs
engine-rs/crates/ls-ingest/src/winjob.rs
engine-rs/crates/ls-jobs/Cargo.toml
engine-rs/crates/ls-jobs/src/lib.rs
engine-rs/crates/ls-jobs/tests/jobs.rs
engine-rs/crates/ls-server/Cargo.toml
engine-rs/crates/ls-server/src/detect.rs
engine-rs/crates/ls-server/src/envfile.rs
engine-rs/crates/ls-server/src/go2rtc.rs
engine-rs/crates/ls-server/src/handlers/cameras.rs
engine-rs/crates/ls-server/src/handlers/dataset.rs
engine-rs/crates/ls-server/src/handlers/events.rs
engine-rs/crates/ls-server/src/handlers/misc_status.rs
engine-rs/crates/ls-server/src/handlers/ops.rs
engine-rs/crates/ls-server/src/handlers/zones_rules.rs
engine-rs/crates/ls-server/src/main.rs
engine-rs/crates/ls-server/src/middleware.rs
engine-rs/crates/ls-server/src/router.rs
engine-rs/crates/ls-server/src/tests/routes_guard.rs
engine-rs/crates/ls-server/src/util.rs
engine-rs/crates/ls-server/src/worker.rs
engine-rs/crates/ls-store/Cargo.toml
engine-rs/crates/ls-store/src/lib.rs
findings.md
progress.md
scripts/build_rust.py
scripts/build_setup_exe.py
scripts/rust_route_coverage.py
scripts/smoke_rust.py
task_plan.md
task_registry.md
tools/README.md
tools/dataset_hub.py
tools/datasets_registry.json
tools/eval_report.py
tools/export_onnx.py
tools/merge_datasets.py
tools/tests/test_dataset_tools.py
tools/train_ppe.py
web/package-lock.json
web/package.json
web/src/App.vue
web/src/api.js
web/src/components/AnnotatorModal.vue
web/src/components/VideoOverlayCanvas.vue
web/src/components/WebrtcPlayer.vue
web/src/overlay.js
web/src/views/Events.vue
web/src/views/FolderImages.vue
web/src/views/Models.vue
web/src/views/Settings.vue
web/src/views/Wall.vue
web/src/views/Zones.vue
web/tests/api.test.js
web/tests/ls042_regressions.test.js
web/tests/wall_overlay.test.js
web/tests/zones.test.js
修改记录_会话备忘.md
```

## 会话日志

### 2026-11-22 · `LS-040` 全仓四线审阅与接力文件入库
- 完成 Rust 管线 / Rust 控制面与安全 / Vue 前端与契约 / CI 与打包安装器四线只读审阅。
- 识别 20+ 项 P0 级缺陷与 35+ 项 P1 级缺陷，产出 6 批次实施计划并获用户全量批准。
- 更新接力 7 件套并提交至 `main`，随即锁定 `LS-041` 开工。
