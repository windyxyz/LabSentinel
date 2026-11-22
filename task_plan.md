# Task Plan — LabSentinel v0.5.0 功能/交互/性能与模型能力全面升级计划

## Goal

在 `v0.4.0` 全仓加固（`LS-040`～`LS-046` 已全部收口，本地 `main @ cafb102`）基础上，推进 **`v0.5.0` 大版本升级**，覆盖：性能底座、识别与规则增强、实时画面与全屏大屏、交互体验、告警与运维闭环、网页端便携标注（数据飞轮）、新架构推理引擎与多源公开数据集训练管道。

> **协作模式**：用户将 `LS-050`～`LS-057` 各批次分发给其他 AI 并行/分批实现，由主线 AI 按 [docs/plan/v0.5.0-ai-dispatch-prompts.md](docs/plan/v0.5.0-ai-dispatch-prompts.md) 中的审阅清单进行代码审查、缺陷修补、门禁复验与合入。

## 已确认设计决策（2026-11-22 `/grill-me` 全部对齐）

1. **发布节奏**：`LS-050`～`LS-056` 全部纳入 `v0.5.0`，每批全绿提交 `main`，最后由 `LS-057` 统一版本同步、双产物打包与发版就绪。
2. **识别与规则（`LS-050`）**：新增本地「单人留守（`solo_after_hours`）」规则 + 动态映射自定义 PPE 模型类别（`no_gloves`/`no_goggles` 等）+ 网页端可编辑自定义 VLM 提示词模板 + 前端开放 PPE 阈值与灰度诊断（零包体膨胀）。
3. **实时画面与大屏（`LS-051`）**：前端 `<canvas>` 叠加层 + WebSocket 实时检测框/跟踪 ID/合规态（同时兼容 WebRTC 与快照轮播，零重编码开销）+ Rust 进程内托管 `go2rtc`（Windows Job Object 防孤儿）+ 全屏值守大屏与提示音。
4. **运维闭环（`LS-053`）**：告警失败重试后台消费者 + 定时日报/周报自动推送 + 网页端局域网配置（自动生成强随机令牌 + 写 `.env` + 一键优雅自重启）+ Windows `HKCU\...\Run` 开机自启（免管理员、卸载自动清理）。
5. **数据飞轮与便携标注（`LS-054`）**：网页端内置轻量画框标注（AI 预标注打底、缩放平移、数字键切类、SQLite v16 落库）+ 一键导出标准 YOLO 训练集 ZIP（`images/` + `labels/` + `data.yaml`）+ 一键脱敏诊断包 + 复用 `devices` 只读令牌的值守分享链接（24h/7d/30d TTL + 吊销）。
6. **模型架构与训练管道（`LS-055` / `LS-056`）**：`ls-infer` 支持 NMS-free（RT-DETR/YOLO26/YOLOv10）、动态分辨率（640/960/1280）、FP16 与灰度 CLAHE + 网页端模型 A/B 评测；`tools/` 新增多源公开数据集聚合下载（优先 HF 镜像免 Key）、统一词表清洗合并、灰度/远景增强与 `--user-zip` 1:3 混合训练脚本。

## Current Phase

- **上一阶段（`LS-040`～`LS-046`）**：`DONE`（全部验证通过并提交至本地 `main @ cafb102`，`zip` 100.2MB + `setup.exe` 75.0MB + `smoke_rust.py` 5/5 PASS）。
- **当前阶段（`v0.5.0`）**：`LS-050`～`LS-057` 已通过本机验收。双产物、实际命令结果和现场验证范围见 `progress.md`。

---

## Active Phases（`v0.5.0` · `LS-050` ～ `LS-057`）

### Phase 1 · `LS-050`（性能底座 + PPE/VLM 识别与规则增强）— `DONE`
- Motion Gate 运动检测预筛（静止空画面跳过 YOLO/PPE，含活跃 track 守卫 + 周期心跳全检防漏判，网页可开关/调阈值）。
- 事件/文件夹缩略图后端按需生成缓存（纳入 `retention.rs` 清理）+ 前端 `loading="lazy"` 懒加载。
- 前端开放 `ppe.violation_conf`/`ppe.wearing_conf` 滑块与灰度/远景诊断；新增 `solo_after_hours` 本地规则；动态支持自定义 PPE 模型类别；网页端可编辑自定义 VLM 提示词模板。

### Phase 2 · `LS-051`（实时画面、Rust 托管 `go2rtc` 与全屏值守大屏）— `DONE`
- Rust 进程内托管 `go2rtc.exe`（`CREATE_NO_WINDOW` + Windows Job Object + 同步 RTSP 流 + `graceful_shutdown` 退出）。
- 新增 `/ws/detections/:cid` 实时检测元数据广播 + 前端 `<canvas>` 叠加层（兼容 WebRTC 与快照轮播，一键显隐）。
- `Wall.vue` 全屏值守大屏（1/4/9 宫格、当日合规率条、新告警浮窗与提示音/桌面通知开关）。

### Phase 3 · `LS-052`（交互体验升级）— `DONE`
- `Zones.vue` 安全区拖拽顶点、撤销/重做、多区域配色标签、刷新底图。
- 新增 `SchedulePicker.vue` 可视化周时段排班选择器（100% 兼容现有 `days_of_week`/`start_time`/`end_time` 契约）。
- `Events.vue` 键盘快审（`J/K/A/F/I/Space/Esc`，输入框聚焦时不误触发）+ 快捷键提示条。

### Phase 4 · `LS-053`（告警与日常运维闭环）— `DONE`
- `ls-jobs` 告警失败重试后台消费者（消费 `alert_deliveries`，支持手工 retry 立即唤醒与退避封顶）。
- 定时日报/周报自动汇总推送（复用现有告警渠道、同日去重、支持立即测试发送）。
- 网页端局域网配置（自动生成令牌 + 写 `.env` + 一键优雅自重启）+ Windows `HKCU\...\Run` 开机自启（同步 `installer.iss` 卸载清理）。

### Phase 5 · `LS-054`（网页端便携标注工作台 + 诊断包 + 只读值守链接）— `DONE`
- SQLite v16 `dataset_samples` 表 + 事件/文件夹/实时抓拍一键入池并跑 AI 预标注 + 前端 `AnnotatorModal.vue`（画框/改角/缩放平移/数字键切类/已核验标记）+ 导出标准 YOLO 训练集 ZIP（8:2 `train/val` + `data.yaml`）。
- `GET /api/diagnostics/bundle.zip` 一键脱敏诊断包（严格脱敏所有密钥）。
- 复用 `devices` 只读令牌生成 24h/7d/30d TTL 只读值守链接（`#/wall?readonly_token=...`），自动锁定只读大屏并支持随时吊销。

### Phase 6 · `LS-055`（推理引擎新架构升级与模型 A/B 评测）— `DONE`
- `ls-infer` 自动读取动态输入分辨率（640/960/1280）、自动识别 NMS-free `[1, N, 6]`（RT-DETR/YOLO26/YOLOv10）、兼容 DirectML FP16、新增纯 Rust 灰度 CLAHE ROI 预处理。
- `POST /api/models/ppe/benchmark` + 网页端新旧模型 A/B 对比评测（耗时与检出对比，满意再一键切换）。

### Phase 7 · `LS-056`（多源公开数据集聚合、现成权重复用与专项训练管道）— `DONE`
- `tools/dataset_hub.py` + `tools/datasets_registry.json`（优先 HF 镜像免 Key，兼容 Roboflow/本地，逐项登记许可与现成开源权重）。
- `tools/merge_datasets.py`（多源标签统一映射、坏图/越界框清洗、MD5 去重、`--user-zip` 与公开数据 1:3 混合）。
- `tools/train_ppe.py` 升级（多架构 `--arch`、灰度/低光/远景增强预设、限线程）+ `tools/eval_report.py` 自动输出对比表 + `tools/export_onnx.py` 支持 `--half` FP16。

### Phase 8 · `LS-057`（`v0.5.0` 全量验收、文档收口与本机双产物实证）— `DONE`
- `bump_version.py 0.5.0` + `CHANGELOG.md` + 全量门禁（Rust 三连 / Web / `contract_audit.py` / `rust_route_coverage.py --fail-under 95`）+ `build_rust.py --strict --zip` + `build_setup_exe.py` + `smoke_rust.py`。

---

## Completed Historical Phases（`LS-040` ～ `LS-046` · `DONE`）

- `LS-040`（`17da1fb`）：初始化接力 7 件套 + 四线全仓审阅。
- `LS-041`（`da1dbd1`）：P0 止血批（DirectML 全局锁、`jiff` 替 `chrono`、SQLite WAL、规则 `days_of_week` v15、VLM 鉴权脱敏、自定义 PPE 上传、PushPlus/Qmsg）。
- `LS-042`（`4722d8c`）：前后端契约全面对齐 + 前端 11 处 P0 修复 + `/ws/webrtc/:cid` 代理与快照降级 + 6 例回归测试。
- `LS-043`（`035e29e`）：PPE 单次多类前向 2× 提速 + 灰度 ROI 二次精检 + `CREATE_NO_WINDOW` + Worker/ffmpeg/规则加固。
- `LS-044`（`068c5f3`）：安全加固 + `retention` 相对路径防误删 + `backup` 原子写 + `mqtt` 退避 + 完整 `graceful_shutdown`。
- `LS-045`（`28e5701`）：CI 门禁 + `contract_audit.py` + 安装器 BOM/停服/Prefetch/`--rollback` + `.env`/`LS_HOST` + 10 crate `[lints]` + `NOTICE`/文档收口。
- `LS-046`（`cafb102`）：本机 `--release`（LTO）打包 `zip`（100.2MB）+ `setup.exe`（75.0MB）+ `smoke_rust.py` 5/5 PASS。
