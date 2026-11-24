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
| LS-062 | 本机与局域网访问密码、浏览器会话和令牌迁移 | P1 | BLOCKED | 2026-11-23 | 在可运行构建产物的 Windows 环境执行 `node web/scripts/smoke_webrtc.mjs <staging>` 和 HTTPS 反代联测 |
| LS-063 | 浏览器设备配对、相机范围授权与撤销 | P1 | BLOCKED | 2026-11-23 | 在可运行构建产物的 Windows 环境重跑 `node web/scripts/smoke_webrtc.mjs <staging>` 的设备画面撤销场景 |
| LS-064 | 1／4／9 路性能、稳定性和识别质量基线 | P1 | BLOCKED | 2026-11-23 | 在可运行基准的 Windows 环境执行 `windows_rtsp_benchmark` 和 `windows_scene_truth_replay` |
| LS-065 | 有界采集与推理、模型格式适配、媒体与前端提速 | P1 | BLOCKED | 2026-11-23 | 在可运行构建产物的 Windows 环境对照 `ceb1ebb` 重跑基准、真值与正式包联测 |
| LS-066 | 人脸关闭／本地／API、模型管理与人员录入 | P1 | DONE | 2026-11-23 | 本地 CPU／DirectML、受控 API 和全量回归通过，现场与正式包验收合并到 `LS-068` |
| LS-067 | 告警投递与值班交接、合规报表、小样本训练预检 | P2 | DEFERRED | 2026-11-23 | 用户现场测试期间暂停。恢复后 `git status --short --branch`，按 `progress.md` 接续本地未提交改动 |
| LS-068 | v0.6.0 · 全量验收、版本同步与 GitHub 正式发布 | P0 | DEFERRED | 2026-11-23 | 现场测试期间暂停发布。恢复后先完成 `LS-067`，再核对全量验证与双产物 |
| LS-069 | 现场 v0.5.0 图片结果、视频文件选择与安全区底图 | P0 | DONE | 2026-11-23 | 旧模型实测拦截、兼容模型和两种冒烟入口通过，发版续见 `LS-068` |
| LS-070 | 现场检测分类、人脸告警、大华接入与卸载清理 | P0 | READY | 2026-11-23 | 使用本地 v0.6.0 双产物联测实拍 PPE、大华设备、安装卸载；等待用户确认发布 |
| LS-071 | 本地图片和视频检查留痕及事件分类 | P0 | DONE | 2026-11-23 | 本地复测包已更新，等用户覆盖测试并复跑视频，GitHub 发布暂停 |
| LS-072 | 事件详情过渡、检查图片预览与标注 | P0 | DONE | 2026-11-23 | 更新本地 0.6.0 安装器和 ZIP，待用户现场覆盖复测 |
| LS-073 | 视觉模型与数据调研、标注训练集分组导出 | P1 | DONE | 2026-11-23 | 本地复测包已更新。现场授权数据供日后独立验收，本机不训练 |
| LS-074 | 现成检测权重对照与 0.6.0 本地随包验证 | P1 | DONE | 2026-11-23 | `python -X utf8 scripts/smoke_rust.py --zip Resources/dist/labsentinel-rust-0.6.0-20261123.zip`，等待用户现场试用，GitHub 发布暂停 |
| LS-075 | 候选模型对照、合并与淘汰 | P1 | DONE | 2026-11-23 | 本地双产物已更新，`python -X utf8 scripts/smoke_rust.py --zip Resources/dist/labsentinel-rust-0.6.0-20261123.zip`；现场测试后再处理 GitHub 发布 |
| LS-076 | v0.6.0 基线缺陷审查与 1.0.0 路线图补充 | P0 | DONE | 2026-11-23 | 查 `docs/plan/v1.0.0-roadmap.md` §1.3 的 9 项发现与验收。待用户恢复后锁定 `LS-080`。 |
| LS-080 | v0.6.1 · `InferBackend` 抽象 + 现成权重对照整合（YOLOE 正负对照/`PP-LCNet`）+ 256×256 上身 PPE 精判/唯一归属/遮挡断轨/COCO 互证 | P0 | DONE | 2026-11-24 | 文件检测与后端改动通过。候选对照完成，256 试验结果保留在 findings，随包沿用达到验收要求的权重组合。 |
| LS-081 | v0.6.1 · 并发流硬上限（≤5）+ WS `?since=` 断线重放 + 分阶段耗时 HUD + 敏感度三档预设 | P0 | DONE | 2026-11-24 | 值守功能和基线缺陷通过定向验证，见 `progress.md`。 |
| LS-082 | v0.6.2 · 空间智能防误报（4 锚点/死区/12 帧离场宽限/Motion+Filter 双遮罩/静止降频/一键记住误报点/抑制审计） | P0 | DONE | 2026-11-24 | 旧库迁移、遮罩心跳、规则与误报点 API 及前端定向验证通过。 |
| LS-083 | v0.6.2 · 共享 `GpuScheduler` + Vulkan 物理设备枚举 + 干净 WebP 叠框 + Debug 叠层/Digital PTZ | P0 | DONE | 2026-11-24 | 实机 DirectML 共用会话、显卡枚举、证据转换及 ZIP 实测通过。接续 `LS-084` 时从事件段与持续重检开始。 |
| LS-084 | v0.7.0 · 违规事件段聚合（`duration_s`/峰值截图）+ 两级宽限倒计时 + 合规恢复自动消警（`alert_resolved`） | P0 | DONE | 2026-11-24 | 10 分钟受控序列、历史库迁移、宽限配置、断线重放和前端定向通过，现场回归归入 0.7.0。 |
| LS-085 | v0.7.0 · 统一 AI Review 可解释性抽屉与抑制审视面板 | P1 | DONE | 2026-11-24 | 解释、权限、审计截图和重复入池保护通过定向验证。 |
| LS-086 | v0.7.0 · 统一 Vulkan CV 引擎选型 Spike（`ncnn-vulkan` vs `ort-webgpu` vs `wgpu/Burn`）& 真机对比基准 | P0 | BLOCKED | 2026-11-24 | 本地验收完成。余 NVIDIA、Intel、四小时 CV+VLM 同卡门禁和增量包体优化。按 ADR-0012 用实验工具复测。 |
| LS-087 | v0.7.5 · 实验室专项规则（1D 通风橱 `ref_lines` / OR-of-ANDs PPE 策略矩阵 / 协议无关固定机位规则） | P1 | ACTIVE | 2026-11-24 | 检查 PPE 归属与事件段入口，先实现本地规则配置形状、阈值和证据状态（Schema `v26`）。 |
| LS-088 | v0.7.5 · 防幻觉 VLM 2.0 统一引擎 + 性价比带识图 AI 辅助报告、多轮历史问答与持久化记录（DeepSeek/Qwen-VL-Flash 等） | P0 | READY | 2026-11-24 | 人员时间窗口和单图复核通过两次回环精测；继续统一引擎、角色路由、持久化报告和问答（Schema `v26`）。 |
| LS-089 | v0.8.0 · 零 NC 数据集 COCO-Pose 姿态跌倒/昏迷检测 + 可选红外热成像温度遥测钩子 | P1 | READY | 2026-11-23 | 见 `docs/plan/v1.0.0-roadmap.md` `LS-089`（Schema `v27`） |
| LS-090 | v0.8.0 · 摄像头防篡改检测（`tamper.rs`）+ 可选 FFmpeg 音频尖峰旁路 | P1 | READY | 2026-11-23 | 见 `docs/plan/v1.0.0-roadmap.md` `LS-090` |
| LS-091 | v0.8.0 · 全链路模型/阈值/提示词 SHA-256 溯源（`open-nvr` 模式）+ 机位×时段热点热力图 | P1 | READY | 2026-11-23 | 见 `docs/plan/v1.0.0-roadmap.md` `LS-091`（Schema `v27`） |
| LS-092 | v0.8.0 · 托管离线 Vulkan `llama-server.exe`（反 NF4 硬拦截 + 显存配额防 TDR）+ `tools/` `ms-swift` 微调剧本 | P0 | READY | 2026-11-23 | 见 `docs/plan/v1.0.0-roadmap.md` `LS-092` |
| LS-093 | v0.8.5 · 统一 Vulkan CV 生产引擎灰度落地 + 下一代检测器（YOLO26n/s + Apache-2.0 可选线） | P0 | READY | 2026-11-23 | 见 `docs/plan/v1.0.0-roadmap.md` `LS-093` |
| LS-094 | v0.8.5 · 声明式「实验室安全技能包（Skill Packs）」4 套内置场景模板 | P1 | READY | 2026-11-23 | 见 `docs/plan/v1.0.0-roadmap.md` `LS-094`（Schema `v28`） |
| LS-095 | v0.9.0 · Vulkan 晋升统一默认 GPU + 数据飞轮 2.0（绿灯数据集 + NC 许可防火墙 + 7 大训练配方 + 人话评测） | P0 | READY | 2026-11-23 | 见 `docs/plan/v1.0.0-roadmap.md` `LS-095`（Schema `v29`） |
| LS-096 | v0.9.0 · 零外部服务中英文语义事件检索（嵌入式 `sqlite-vec` + `Jina-CLIP v2`/`SigLIP 2`） | P1 | READY | 2026-11-23 | 见 `docs/plan/v1.0.0-roadmap.md` `LS-096`（Schema `v30`） |
| LS-097 | v0.9.0 · 零转码视频底座（可选 `retina` RTSP + 关键帧环 + 按需 fMP4）+ 分层保留 + 事务性 Alert Outbox | P0 | READY | 2026-11-23 | 见 `docs/plan/v1.0.0-roadmap.md` `LS-097`（Schema `v31`） |
| LS-098 | v0.9.5 · 细粒度可配权限本地 MCP Server（默认只读 + 逐工具高权限勾选 + 独立令牌 + 审计 + HMAC 凭证） | P0 | READY | 2026-11-23 | 见 `docs/plan/v1.0.0-roadmap.md` `LS-098`（Schema `v32`） |
| LS-099 | v0.9.5 · 原生 `windows-service` 服务化 + 一键纯视觉审计证据包 ZIP + 可选物理声光报警 Actor | P1 | READY | 2026-11-23 | 见 `docs/plan/v1.0.0-roadmap.md` `LS-099` |
| LS-100 | v1.0.0 · 72h 统一 Vulkan（CV+VLM）并发长稳压测 + 全维度精度与 MCP 安全认证 | P0 | READY | 2026-11-23 | 见 `docs/plan/v1.0.0-roadmap.md` `LS-100` |
| LS-101 | v1.0.0 · 升级/回滚/干净卸载全矩阵 + 代码签名（`LS-900`）+ `v1.0.0` GA 双产物正式发布 | P0 | READY | 2026-11-23 | 见 `docs/plan/v1.0.0-roadmap.md` `LS-101` |
| LS-900 | 代码签名证书（T-403）与 GitHub uploads 网络通道（S302）处理 | P3 | BLOCKED | 2026-11-22 | 人工事项：待证书到位或用户调整本机网络/hosts |
