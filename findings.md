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
