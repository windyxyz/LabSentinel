# Task Plan — LabSentinel v0.4.0 全仓审阅修复与演进计划

## Goal

基于 2026-11-22 四线全仓代码审阅结果，按「**P0 止血 → 前后端契约对齐 → PPE 灰度图调优与 Worker 健壮性 → 安全与后台可靠性加固 → CI/打包/安装器/文档收口 → 本地打包冒烟与 v0.5.0 就绪**」分 6 个独立可验证的 `Task-ID` 批次（`LS-041`～`LS-046`）全面清零高危缺陷并夯实工程防线。

## 已确认设计决策（2026-11-22 用户批准全部推荐方案 A）

1. **接力文件入库（`LS-040`）**：7 个接力文件同步最新审阅结果后提交至 `main`。
2. **实时画面 `/ws/webrtc/:cid`（`LS-042`）**：Rust 补齐到 `go2rtc` 的 WS 代理；`go2rtc` 不可用或为 USB/文件源时 `/api/cameras/:id/webrtc` 返回 `mode: "snapshot"`，前端 `Cameras.vue`/`Wall.vue` 无缝降级快照轮播。
3. **局域网监听 `LS_HOST` / `.env`（`LS-045`）**：`main.rs` 真正实现启动读 `.env` 与 `LS_HOST`，非回环地址强制要求 `LS_ADMIN_TOKEN`，否则拒绝启动。

## Current Phase

- **`LS-040`**：`DONE`（接力文件已同步并提交入库 `main`）
- **当前锁定执行**：**`LS-041`（Phase 1 · P0 止血批）**

---

## Active Phases

### Phase 1 · `LS-041`（P0 止血批）— `ACTIVE`
- **DirectML 安全 & `EP_USED`**：`ls-infer/src/yolo.rs` 加进程级 `DML_LOCK`（覆盖建会话/`run`/`Drop`）+ `with_memory_pattern(false)` + `with_parallel_execution(false)` + 单例 `ORT` Environment + `EP_USED` 改 `RwLock`；`ls-infer/src/lib.rs` 放宽 `#[cfg(any(feature="tract", feature="ort"))]`。
- **SQLite WAL + 规则修复 + 本地时区统一（替 `chrono`）**：`ls-store` 开 `PRAGMA journal_mode=WAL`、`add_rule` 写 `'1,2,3,4,5,6,7'`、`migrate()` 升级 v15 修存量 `days_of_week='0'`；`ls-core::time` 引入 `jiff` 统一全仓本地时区、移除 `chrono`（解除 `RUSTSEC-2026-0543`）；`worker.rs` 实时流禁用 `2026-01-01` 合成时间戳。
- **自定义 PPE 上传全通**：`router.rs` 为 3 个上传路由加 `DefaultBodyLimit` + 流式限额；`worker.rs` 补读 `app_settings["ppe_model"]`；`ppe_upload` 校验 ONNX 并写入/继承 `ppe-custom.classes.txt`；`ppe_status` 回显 `classes`/`warning` 并缓存 SHA256；`build_rust.py` 去掉 `LabSentinel.bat` 强注 `LS_PPE_MODEL`。
- **VLM & 多接入档案修复**：`vlm.rs` `openai_compat` 补 `Authorization: Bearer` 与 `/v1` 去重；`gemini` 改 `x-goog-api-key` 请求头 + `/v1beta` + 唯一临时文件 + 错误脱敏；`ureq` 加 `http_status_as_error(false)`；`detect.rs` 修 `usage_try_consume` 原子化/off-by-one/合规前置、虚拟档案迁移保密钥、消除影子覆盖与双重重启；`worker.rs` 修 `cascade + suppress` 吞事件与放行丢截图。
- **告警渠道 P0 修复**：`alerts.rs` 补 PushPlus/Qmsg `url`、`smtp_port` 数字解析、`smtp_security`、模板清空清理内存、`alert_channels_snapshot` 合并 env 回退、`alert_test` 0 发送返 `ok: false`。

### Phase 2 · `LS-042`（前后端契约对齐与前端 P0 修复）— `READY`
- 后端对齐：`stats/hourly` 24 项数组、`folder-images` 补 URL/区域/违规、`events` 筛选/计数/批量/证据包、`config/import+export` `warnings`+`zone_name`、`rule-templates` 双键兼容、`alert-silences` `duration_s`+`remaining_s`、`alerts/status` 真 `channels`、`camera-groups` 对象数组、`cameras`/`rules` 全字段落库、`/ws/webrtc/:cid` 代理与降级。
- 前端全视图修复：`ReportsView`、`Events`、`FolderImages`、`Settings`、`Zones`、`DetectPanel`、`Models`、`Welcome`、`FsPicker`、`Devices`、`Cameras`、`Wall`、`Face`、`api.js`、`style.css` 及多根组件收口。
- 补齐 8 组前端回归单测（基于 Rust 真实响应结构）。

### Phase 3 · `LS-043`（PPE 效果/性能与 Worker/管道加固）— `READY`
- `YoloModel::detect_classes` 单次前向多类解码（2× 提速 + 去 8400 次 Vec 分配）+ 空图/迟滞守卫 + 网页可调 `ppe.violation_conf`/`ppe.wearing_conf` + 人框 ROI 扩框二次精检 + `POST /api/models/ppe/diagnose` + `log` 门面落文件日志 + 修 `model.json` 覆盖与 `Unknown` 误报 `enter_no_ppe`。
- `worker.rs` 修 `folder_loop` `.take(20)` 饥饿、`given_up` 清理与 RAII 句柄守卫、`Instant` 下溢、`mode == "api"` 异步无阻塞、有界工作线程池；`ls-ingest`/`evidence.rs`/`discovery.rs` 补 `CREATE_NO_WINDOW`、RTSP `-rw_timeout`、USB 15s 超时、`alive_since` 首帧置位；`pipeline.rs`/`ls-core`/`ls-track` 修 `lost_track_buffer` 双重缩放、`cooldown_s` 单位、`occupancy_over` 串台、`zones.rs` `i128` 交叉乘、`ls-track` 除零与 env 缓存。

### Phase 4 · `LS-044`（安全加固、后台可靠性与优雅退出）— `READY`
- `/ws/events` 校验 `trusted_hosts`；限流改 `ConnectInfo` + 桶清理；补 `ADMIN_GETS`；`/api/cameras` 脱敏 RTSP 密码；路径 containment 统一 `Path::starts_with`；`plugins.rs` Zip 限额与原子安装；URL 防 `@` 绕过 + `max_redirects(0)`；`detect_models` 防 key 外泄；修 `urldecode`/`discovery`/`version_gt`/`genai` panic 与下溢；`fernet.rs` 并发建锁与标准兼容；`logs_tail` 真尾读与 JSON 脱敏。
- `retention.rs` 相对路径按 `data_dir` 解析防误删；`backup.rs` 原子 `.tmp`+真实 `db_path`；`alert-deliveries/:did/retry` 真重投；`dispatch_event` 接自定义模板；`mqtt.rs` 断线退避与 `try_publish`；`graceful_shutdown` 完整停 `jobs`/插件/`mqtt`/托盘；`models_active`/`ppe_download` 自动重启；长 IO 迁 `spawn_blocking`。

### Phase 5 · `LS-045`（CI、打包/安装器、依赖与文档合规）— `READY`
- CI：`release.yml` 挂 `rust.yml`、Node 22、删冗余 CPU ort、补 PPE/DML/升级/删数据冒烟；新增 `scripts/contract_audit.py` 进 `tests.yml`；`rust_route_coverage.py --fail-under 95`；统一 `deny.toml`（`all-features=true`）与 `cargo-audit`；补 `SECURITY-EXCEPTIONS.md` 过期检查。
- 安装器与打包：`installer.iss` UTF-8 BOM + 升级前停服 + `{win}\Prefetch` + `current.txt` + 探活失败提示；`deploy.rs` 修 `--rollback` 自覆盖与 SemVer；`build_rust.py` 清空 `DIST` + 去重 DLL + SHA256 强校；`bump_version.py` 五源同步；10 子 crate 补 `[lints] workspace = true`；gRPC 迁可选 `grpc` feature；替换 `fs2`；`main.rs` 实现 `.env` + `LS_HOST`（非回环强门控）；清死脚本。
- 文档：重写 `NOTICE`、同步 `docs/plan/status.md`、`README.md`、`docs/guide/安装与卸载.md`、`CHANGELOG.md`。

### Phase 6 · `LS-046`（本机打包与验收就绪）— `READY`
- 执行本机全量打包（`zip` + `setup.exe`）与 `smoke_rust.py` / `acceptance_rust_only.py` / `acceptance_vlm.py` 端到端实证；保留真机 24h RTSP 与外部真实 VLM API Key 人工验收清单待现场执行。
