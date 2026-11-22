# Task Registry — LabSentinel 任务总表

> 口径：`DONE` 已验证完成 · `ACTIVE` 当前锁定执行中 · `READY` 计划就绪待锁定 · `BLOCKED` 阻塞待人工/条件 · `DEFERRED` 明确暂缓。
> 规则：同一时间只锁定**一个** `ACTIVE` 任务；完成后须附验证证据再置 `DONE`。

| Task-ID | Task-Name | Priority | Status | Last Update | Next First Command |
|---|---|---|---|---|---|
| LS-040 | 接手 v0.4.0 基线、全仓四线审阅、初始化并入库接力文件 | P0 | DONE | 2026-11-22 | 已完成并入库，转锁定 `LS-041` |
| LS-041 | P0 止血批：DirectML 并发锁(A1) + WAL + 规则 `days_of_week` + 本地时区/`chrono` 替换 + VLM 鉴权/脱敏 + 自定义 PPE 上传 + PushPlus/Qmsg | P0 | DONE | 2026-11-22 | 已完成并全绿提交，转锁定 `LS-042` |
| LS-042 | 前后端契约全面对齐 + 前端 11 处 P0 修复 + `/ws/webrtc/:cid` 代理与快照降级 + 8 组回归测试 | P0 | DONE | 2026-11-22 | 已完成并全绿提交，转锁定 `LS-043` |
| LS-043 | PPE 单次多类推理 2× 提速 + 灰度图 0 检出调优/诊断 + GUI 日志落盘 + Worker/ffmpeg/规则/跟踪器加固 | P1 | DONE | 2026-11-22 | 已完成并全绿提交，转锁定 `LS-044` |
| LS-044 | 安全加固 + `retention` 相对路径防误删 + 派发重试/模板/MQTT 退避 + 完整 `graceful_shutdown` + `spawn_blocking` | P1 | DONE | 2026-11-22 | 已完成并全绿提交，转锁定 `LS-045` |
| LS-045 | CI 门禁加固（`rust.yml`/Node 22/契约审计）+ 安装器 BOM/停服/Prefetch/`--rollback` + `LS_HOST`/`.env` + 依赖与 `NOTICE`/文档收口 | P1 | DONE | 2026-11-22 | 已完成并全绿提交，转锁定 `LS-046` |
| LS-046 | 本机打包与安装/卸载冒烟实证；真机 24h RTSP & 外部真实 VLM 验收准备（`v0.5.0`） | P2 | DONE | 2026-11-22 | 本机 `zip`（100.2MB）+ `setup.exe`（75.0MB）+ `smoke_rust.py` 全绿 PASS；真机 24h RTSP 与外部真实 VLM API Key 留待现场人工验收 |
| LS-050 | v0.5.0 · 性能底座（Motion Gate 运动预筛 + 缩略图缓存懒加载）+ 识别与规则增强（PPE 阈值 UI + 单人留守 + 动态 PPE 类 + 自定义 VLM 提示词） | P0 | DONE | 2026-11-22 | Motion Gate、规则、PPE 配置与提示词单测通过，见 `progress.md` |
| LS-051 | v0.5.0 · Rust 托管 `go2rtc`（Job Object）+ WS 实时检测框 Canvas 叠加 + 全屏值守大屏与提示音 | P0 | DONE | 2026-11-22 | 随包 go2rtc、WS 守卫与大屏单测通过，见 `progress.md` |
| LS-052 | v0.5.0 · 交互体验升级（安全区拖拽顶点/撤销 + 可视化周时段排班 + 事件流键盘快审） | P1 | DONE | 2026-11-22 | 安全区拖拽/撤销、排班与键盘快审测试通过，见 `progress.md` |
| LS-053 | v0.5.0 · 告警与日常运维闭环（失败重试消费者 + 定时日报/周报 + 网页局域网配置/自重启 + Windows 开机自启） | P1 | DONE | 2026-11-22 | 重试消费、调度测试与隔离自重启实测通过，见 `progress.md` |
| LS-054 | v0.5.0 · 网页端便携标注工作台（数据飞轮 + AI 预标注 + YOLO ZIP 导出）+ 一键脱敏诊断包 + 只读值守链接 | P0 | DONE | 2026-11-22 | 样本池、YOLO ZIP、诊断脱敏与分享令牌测试通过，见 `progress.md` |
| LS-055 | v0.5.0 · 推理引擎新架构升级（NMS-free RT-DETR/YOLO26/YOLO12 + 动态尺寸/FP16 + 灰度 CLAHE）+ 模型 A/B 对比评测 | P1 | DONE | 2026-11-22 | 真实双模型 ONNX Runtime CPU 运行、FP16 I/O 与单测通过，见 `progress.md` |
| LS-056 | v0.5.0 · 多源公开数据集聚合下载（HF 镜像免 Key）+ 统一清洗合并 + 灰度/远景增强混合训练与自动评测（`tools/`） | P1 | DONE | 2026-11-22 | 数据清洗与混合的离线单测及语法编译通过，见 `progress.md` |
| LS-057 | v0.5.0 · 全量验收、版本同步（`0.5.0`）、`CHANGELOG.md` 与本机双产物打包冒烟实证 | P0 | DONE | 2026-11-22 | `git log -1 --stat`；双产物、SHA256 与冒烟结果见 `progress.md` |
| LS-058 | v0.5.0 接手、全仓复审与正式产物独立验收 | P1 | DONE | 2026-11-22 | `rg -n 'LS-058-0[1-6]' findings.md`；按发布影响确定下一修复批 |
| LS-059 | 稳定性修复、后续功能、UI 与高性能版本规划 | P1 | DONE | 2026-11-22 | `rg -n '^### LS-060' task_plan.md`，进入下一个实施任务 |
| LS-060 | v0.6.0 · 正式包与值守修复，含播放、回滚、局域网、分享吊销和 CI | P0 | DONE | 2026-11-23 | `LS-068` 执行 Windows 发布流水线的安装器和 ZIP 冒烟 |
| LS-061 | 首页状态、Release 入口、报表文案与告警布局 | P1 | DONE | 2026-11-23 | `LS-068` 在正式产物复查首页和告警配置 |
| LS-062 | 本机与局域网访问密码、浏览器会话和令牌迁移 | P1 | ACTIVE | 2026-11-23 | `rg -n 'check_read_token|provided_token|LS_ADMIN_TOKEN' engine-rs/crates/ls-server/src` |
| LS-063 | 浏览器设备配对、相机范围授权与撤销 | P1 | READY | 2026-11-22 | `rg -n 'create_pairing|claim_pairing|revoke_device' engine-rs/crates/ls-server/src/handlers/pairing.rs` |
| LS-064 | 1／4／9 路性能、稳定性和识别质量基线 | P1 | READY | 2026-11-22 | `rg -n 'analysis_fps|evidence_fps|MotionGate' engine-rs/crates/ls-server/src/worker.rs` |
| LS-065 | 有界采集与推理、模型格式适配、媒体与前端提速 | P1 | READY | 2026-11-22 | 先读 `LS-064` 基准结果，再锁定影响最大的热路径 |
| LS-066 | 人脸关闭／本地／API、模型管理与人员录入 | P1 | READY | 2026-11-22 | `rg -n 'FaceClient|face_guard' engine-rs/crates/ls-server/src` |
| LS-067 | 告警投递与值班交接、合规报表、小样本训练预检 | P2 | READY | 2026-11-22 | `rg -n 'LS-058-(07|08|11|12)' findings.md` |
| LS-068 | v0.6.0 · 全量验收、版本同步与 GitHub 正式发布 | P0 | READY | 2026-11-22 | 核对 `LS-060`～`LS-067` 证据，再执行 `python -X utf8 scripts/bump_version.py 0.6.0` |
| LS-900 | 代码签名证书（T-403）与 GitHub uploads 网络通道（S302）处理 | P3 | BLOCKED | 2026-11-22 | 人工事项：待证书到位或用户调整本机网络/hosts |
