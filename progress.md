# Progress — LabSentinel 会话进度与交接快照

## Latest Handoff Snapshot

- **当前任务**。`LS-081`，规则生效、告警恢复与值守基础。总目标为完成 `LS-080` 至 `LS-083` 并推送 `v0.6.2` 主线，后续版本暂停 GitHub Release。
- **已验证的第一批实现**。修复照片人数规则的区域、排班和等级判定，视频区分推理失败和有效零检测，PPE 状态与分析共用模型选择。抽出推理后端接口，增加同次 COCO 上下文取框、PPE 唯一归属、YOLOE 正负复核、灰度裁剪增强和遮挡断轨。
- **验证**。使用本地发行目录 ONNX Runtime 1.20.1 执行 Rust fmt、Clippy 和工作区全特性测试，205 项通过、4 项忽略。默认安装环境的旧 ONNX Runtime 首次拒绝 IR 10，切换随包 DLL 后通过。重新构建 `ls-server`，在临时安装布局运行 `python -X utf8 -B scripts/smoke_rust.py --install-dir <临时目录> --timeout 45`，照片规则、视频完成与推理失败对照、事件预览及退出清理通过。
- **GitHub**。首批修复 `55ec098` 已直连推送，`git ls-remote --heads origin main` 已确认远端提交。版本号仍为 0.6.0，推送限主线。
- **补充验收**。Rust 三连复验通过。扩展临时安装目录的 `smoke_rust.py`，实测自定义 PPE 文件丢失后随包模型真正加载，人员模型完全缺失及逐帧失败均留下检查记录和错误终态。
- **模型试验**。已有 YOLOE 的裁剪方案仍有误判。原生 256 双窗导出及 CPU 对照成功，14 个人框中确认 10 个，随包方案确认 12 个，双窗未补出额外结果。候选权重和一次性实验环境位于忽略的 `Resources/runs/ls080-native-crop/`，开发始终采用推理和导出。见 [裁剪候选](findings.md#ls-080-裁剪候选第一轮验证)。
- **LS-081 已验证**。区域、规则和排班保存后，下次检测使用新版配置。照片目录运行中删除规则后仍留检查记录，新增规则后按新等级生成告警。区域设置页显示分析任务的实际生效状态。已锁定的人员权重通过校验，临时损坏缓存触发重新准备，严格构建在编译前停止。退出正常的临时 ZIP 冒烟包含新前端、PPE 回退、规则热更新和视频失败对照。
- **验证命令**。Rust 三连通过（205 项通过、4 项忽略）。`npx vitest run` 101 项通过、`npm run build` 通过。`contract_audit.py` 匹配 141 条前端路径，`rust_route_coverage.py --exe C:/Users/User/devtools/ls-target/debug/labsentinel-server.exe --fail-under 100` 为 126/126。前端仍有既有 Vite 配置、表格结构和 jsdom Canvas 提示。
- **安装器**。Inno Setup 7 从同一临时暂存目录编译通过，退出码 0，验证产物 154.2 MB。交互安装仍在后续验收范围。
- **投递与处置**。启动时恢复历史的待投递项，保留重试退避和已发送记录。测试检查记录可查、告警统计排除检查、AI 复核的展示与筛选一致。Rust 三连通过（207 项通过、4 项忽略），前端 101 项和构建通过。
- **Next First Command**。`git status --short --branch`，继续 `LS-081` 的大屏流量上限、事件重连、耗时界面和灵敏度设置。达到使用门槛的 256 候选仍待核验。

## Previous Handoff Snapshot · LS-076

- **Task-ID / Task-Name**。`LS-076`（`DONE`），`v0.6.0` 基线缺陷审查与 1.0.0 路线图补充。本地 `main @ 8dca517`，产品基线 `2fef1b4`。
- **交付**。在 [正式路线图 §1.3](docs/plan/v1.0.0-roadmap.md#13-ls-076-缺陷与发布验收) 登记 9 项缺陷，写入修复归属和发布验收条件。[缺陷证据](findings.md#ls-076-v060-基线缺陷审查) 包含代码位置和实测结果。开始新模型对照前，先完成 `LS-080` 的文件推理、规则和 PPE 状态回归。`LS-081` 处理规则热生效、投递恢复、统计以及双产物验收。
- **实测**。执行 `python -X utf8 -B "$env:LOCALAPPDATA/Temp/labsentinel-ls076-audit.py" baseline`、同脚本 `video` 和 `delivery` 三个子运行。均以回环服务、临时库和项目验收素材复现预期缺陷。正常模型 15 帧均检到人员，输出不兼容的模型 15 帧推理均失败仍显示完成。照片忽略区域、排班和等级。新增规则至重启才生效，删除后仍触发。另复现 PPE 状态误报、AI 复核不可见、检查留痕污染处置计数和遗留 `pending` 行的手动重试 404。
- **发行核验**。以 Python 临时文件调用 `prepare_person_model(..., force=False)`，无效缓存被接受。通过 ZIP 清单和 SHA-256 核对，实测二进制与 ZIP 一致，当前人员模型符合锁定值，便携包及安装器来源目录缺少 WinSW。源码另确认告警事件及投递写入间隔内的崩溃窗口。
- **文档核验**。`git diff --check` 通过。路线图与发现表的 9 个编号一致，任务表中 `LS-076` 为 `DONE`，22 个实施任务为 `READY`。
- **范围**。本轮只更新本地规划与交接文档。复现采用已有二进制和 CPU。现场相机、DirectML 长稳、外部渠道与交互安装保留到对应任务验收。实现、打包、训练和 GitHub 操作暂停。
- **Next First Command**。待用户恢复后执行 `git status --short --branch`，再按路线图 §1.3 复现 `LS-076-01/03/04` 并锁定 `LS-080`。`LS-080`～`LS-101` 当前为 `READY`。

## Previous Handoff Snapshot · 1.0.0 规划入库

- **Task-ID / Task-Name**：`v0.6.x → v1.0.0 GA` 长线规划入库（`LS-080` ～ `LS-101` 登记为 `READY`；现网基线维持 `v0.6.0` / `LS-075` `DONE`）。`main`，本地工作区。
- **本轮交付**：完成 18 个指定项目/论文/文章 + 15+ 全网顶级标杆（Frigate v0.18、AmberSight、Moonfire/Retina、Construction-Hazard-Detection、YOLO26/YOLOE/PP-LCNet、Qwen3-VL/SmolVLM2、`sqlite-vec`）的深度对标；经 Swarm 三轨架构规划与实码核验，正式入库 [docs/plan/v1.0.0-roadmap.md](docs/plan/v1.0.0-roadmap.md)（9 个渐进版本 `v0.6.1 → v1.0.0 GA`、22 项任务 `LS-080`～`LS-101`、SQLite `v22 → v32`）、[docs/adr/0012-unified-vulkan-gpu.md](docs/adr/0012-unified-vulkan-gpu.md)（`Proposed`），并同步更新 `task_plan.md` 与 `task_registry.md`。
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
