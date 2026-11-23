# Findings — LabSentinel 全仓审阅关键发现、风险与约束备忘

> 2026-11-22 四线全仓只读审阅基线：`main @ 10e7006`（`v0.4.0`）。每条均经源码逐行核验。

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
