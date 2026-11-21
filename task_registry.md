# Task Registry — LabSentinel 任务总表

> 口径：`DONE` 已验证完成 · `ACTIVE` 当前锁定执行中 · `READY` 计划就绪待锁定 · `BLOCKED` 阻塞待人工/条件 · `DEFERRED` 明确暂缓。
> 规则：同一时间只锁定**一个** `ACTIVE` 任务；完成后须附验证证据再置 `DONE`。

| Task-ID | Task-Name | Priority | Status | Last Update | Next First Command |
|---|---|---|---|---|---|
| LS-040 | 接手 v0.4.0 基线、全仓四线审阅、初始化并入库接力文件 | P0 | DONE | 2026-11-22 | 已完成并入库，转锁定 `LS-041` |
| LS-041 | P0 止血批：DirectML 并发锁(A1) + WAL + 规则 `days_of_week` + 本地时区/`chrono` 替换 + VLM 鉴权/脱敏 + 自定义 PPE 上传 + PushPlus/Qmsg | P0 | DONE | 2026-11-22 | 已完成并全绿提交，转锁定 `LS-042` |
| LS-042 | 前后端契约全面对齐 + 前端 11 处 P0 修复 + `/ws/webrtc/:cid` 代理与快照降级 + 8 组回归测试 | P0 | ACTIVE | 2026-11-22 | 对齐 `handlers/{stats,folder_images,events,config_io,zones_rules,ops,alerts,cameras}.rs` 与 `web/src/` 全视图，跑 `npx vitest run && npm run build` |
| LS-043 | PPE 单次多类推理 2× 提速 + 灰度图 0 检出调优/诊断 + GUI 日志落盘 + Worker/ffmpeg/规则/跟踪器加固 | P1 | READY | 2026-11-22 | 在 `ls-infer` 实现 `detect_classes`、在 `ls-analyzer/ppe.rs` 接 ROI 精检与 `log` 门面，修复 `worker.rs`/`ls-ingest`/`pipeline.rs`/`ls-track` |
| LS-044 | 安全加固 + `retention` 相对路径防误删 + 派发重试/模板/MQTT 退避 + 完整 `graceful_shutdown` + `spawn_blocking` | P1 | READY | 2026-11-22 | 修复 `middleware.rs`/`plugins.rs`/`fernet.rs`/`retention.rs`/`mqtt.rs`/`tray.rs` 并补安全单测 |
| LS-045 | CI 门禁加固（`rust.yml`/Node 22/契约审计）+ 安装器 BOM/停服/Prefetch/`--rollback` + `LS_HOST`/`.env` + 依赖与 `NOTICE`/文档收口 | P1 | READY | 2026-11-22 | 编写 `scripts/contract_audit.py`，更新 `.github/workflows/*.yml`、`installer.iss`、`deploy.rs`、`build_rust.py`、`Cargo.toml` 与 `NOTICE`/`README`/`status.md` |
| LS-046 | 本机打包与安装/卸载冒烟实证；真机 24h RTSP & 外部真实 VLM 验收准备（`v0.5.0`） | P2 | READY | 2026-11-22 | 执行 `build_rust.py --strict --zip`、`build_setup_exe.py`、`smoke_rust.py` 与本地验收脚本 |
| LS-900 | 代码签名证书（T-403）与 GitHub uploads 网络通道（S302）处理 | P3 | BLOCKED | 2026-11-22 | 人工事项：待证书到位或用户调整本机网络/hosts |
