# 55 · 交接快照（2026-11-04，告警配置 140 完成，flash idle）

> 本文件是**上下文压缩的落点**：新会话/新协作者先读这一份，再按链接进细节。
> 当前状态：批次 122–139 已验收；新增 **任务 140 网页端告警配置**（保存即生效/单渠道测试）；
> 最终包 `LabSentinel-portable-20261104.zip` sha **4b2eeaf3…**；
> pytest **511** + vitest **26**、build **66.59KB**、smoke 8/8、doctor 10/0/0、rc 4/4。
> **119/120 `a8d8ca1`**；**121 `f7ba376`**；**106–108 `f07c0df`**；**122–140 待本次提交**。
> 发布硬阻塞：**真实 RTSP 摄像头 24h + 模型止损线** + Actions 实跑/签名。

## 〇、2026-11-03 主线进展（121）

- **新增图片文件夹源** `type=folder`：外部截图软件持续写入目录 → 自动识别人员/实验服/安全区，
  关键图片（有人）与违规图片（显式 No Lab Coat / 人数超限）分开展示；原视频/RTSP/USB 全部保留。
- 新表 `folder_images`（schema **v5**）；新页「图片识别」：汇总卡、文件夹/时间/关键/违规筛选、
  缩略图墙、原图+识别详情+关联事件；摄像头页支持 folder 类型/统计/立即扫描。
- 默认只为显式违规产生事件/告警（`LS_FOLDER_ENTER_EVENTS=false`，避免每张截图都像“新进入”）；
  原图只读；证据副本/缩略图 30 天保留清理；删除摄像头同步清理派生文件。
- 长跑：目录 mtime 增量扫描 + 5 分钟全量补扫、静置窗口、路径+大小+mtime 去重、单轮限量、
  DML 串行锁/限速；损坏图片只记一次错误。
- 使用手册 [docs/68-image-folder-ingestion.md](68-image-folder-ingestion.md)；
  任务/实现记录 [docs/50-tasks/121-folder-image-ingestion.md](50-tasks/121-folder-image-ingestion.md)。
- **子代理恢复**：122 已完成并验收（`aadabd5`）；用户确认 118 Hyper-V 复测已做，仅补记录。
- **106–108 已完成并提交 `f07c0df`**：
  - 106 状态有界：ZoneState.retain（10k→10）+ RuleEngine 冷却键定期 prune，6 项测试；
  - 107 孤儿媒体：1h 宽限、500/轮、事件/folder_images 引用保护、preview 只读，4 项测试；
  - 108 dispatcher 退出可打断 + send_test 并发（34 项告警回归）；
  - **pytest 343 passed**（329+14），仅后端改动，无前端 build 变化。
- **2026-11-04 复审与重排**：采纳 #20 防休眠(P1)/#12 VLM(#124, P2)/#11 MQTT(#125)/插件一期(#126)；
  #5/#7 折叠进 113/117；Rust/Electron/重型视觉否决。新队列见 docs/72 与看板。
- **批次 122–130 全部交付（2026-11-04）**：109/110/114/123/102/99（Phase A）、
  112/111/113/116/115/117/103（Phase B）、124/125/126（Phase C）、101/84/87/129（Phase D）、
  100/104（收口）；pytest **491→495**、build 60.11KB、zip 20261104（smoke/doctor/rc/release_check 全绿）。
  复审补丁：genai 中文路径、插件入口 `{python}`/is_relative_to、MQTT LWT 顺序、版本 alpha。
- **当前 131 已完成**：插件 UI；批次 130–139（131 插件 UI/132 AI 徽标/133 vitest 26/134 安全审计/
  135 View Transitions/136 时间线/130 最终 RC）全部交付；复审补 3 处；flash idle。
- 后续 backlog：137 事件前缓冲、138 Label Studio、139 4 路压测（可选）；
  桌面壳/托盘、深链、winget 仍延后。最终包需“130b”式重建。

## 一、总体状态

| 维度 | 状态 |
|---|---|
| 服务端/Web | ⏸ 当前未启动；8000/8001/1984/8554/5173 均无监听 |
| 测试 | ✅ **pytest 511 passed + vitest 26 passed**；build gzip **66.59KB**；release_check 全 PASS |
| 模型 | ✅ v2 已接入：`m0-spike/yolo11n_ppe.onnx`（11 类）+ classes.txt；v1 备份 `*_v1.*` |
| 便携包 | ✅ 最终：`LabSentinel-portable-20261104.zip` 279.9MB/12620 条目/sha 4b2eeaf3…（smoke 8/8、doctor 10/0/0、rc 4/4）；含 140 告警配置与清理 |
| 启动 EXE | ✅ 任务 55 已交付并验收：`LabSentinel.exe`（C# winexe，5.5KB，无黑框；`--stop` 可用）；无 csc 构建自动降级到 `启动.bat`/pythonw |
| 部署 | ✅ 便携包（普通用户）+ `一键启动.bat`/`deploy.py`（开发模式，无 Python 时自动改启便携包） |
| Git | 最近 `3e9ea72`（140 文档）/`9aebed6`（140 代码）/`3dd8873`（清理+包）；工作区仅 `.zcode/` 未跟踪 |
| 协作 | ⚠️ **无定时自动化**（旧 5 分钟 cron 已删除）；派发方式 = 更新 `调度看板.md` + `派发状态.json`，flash 会话读取后执行 |
| 发布 | ⛔ 被真实摄像头 24h + 模型停止线阻塞；其余自动可验项全绿 |

## 二、任务状态（01–110）

- **01–50：已完成并验收**（详见各任务文档执行报告；40/47/41/42/48/49/50 于 10-28 初验）。
- **51–55：已完成、主线验收并提交（2026-10-30，commit `04cb605`）**：
  - **51 播种单源化**：`scripts/seed_demo.py` 单源（schema 与 store 对拍），deploy/portable/build 共用；
    `server/tests/test_seed_demo.py` 5 用例。
  - **52 CI 便携包流水线**：`.github/workflows/portable.yml`（workflow_dispatch + tags v*、windows-latest、
    build_portable --zip、SHA256SUMS、upload-artifact）；**仅静态验证，未在真实 runner 跑过**；
    启用前提：模型/tools 二进制不入库，需 LFS/artifact 通道。
  - **53 M2 方案备忘录**：`docs/63-m2-options.md`（七方向对比 + 推荐顺序 A→(E∥D)→G）。
  - **54 文档口径统一**：部署指南/FAQ/README 共 9 处 m0-venv 服务端命令改为 `.venv`/deploy 口径。
  - **55 启动 EXE**：`scripts/launcher_exe/LabSentinelLauncher.cs` + `build_launcher_exe.py`；
    build_portable 产 `LabSentinel.exe`；install_portable 快捷方式优先 exe；实测隔离启动 6s 就绪、
    `--stop` rc=0；pytest 91。
- **56 本机 USB/OBS 虚拟摄像头：主线完成**（10-28，commit `ec5443a`）：物理 HD Webcam index 0
  （黑屏/遮挡但 running、fps 12.5）与 OBS Virtual Camera index 1（fps 29.6、3 事件、截图 200）；
  修复 USB 源 `"0"/"1"` 解析与 FPS=-1；工具 `tools/list_cameras.py`、`tools/virtual_cam_feed.py`；
  报告 `docs/64-usb-virtual-camera-report.md`。**不替代真实 RTSP 24h**。
- **105–110：审阅回归/建议批次**；**105 已完成（`ac7d87e`）；106–108 已完成（`f07c0df`）**：
  - 106 状态字典 prune；107 孤儿媒体回收；108 告警退出/自测并发；
  - **109 deploy.py 三坑、110 停止 PID 复用 + smoke 清理**：待做（当前 109）。
- **119/120：已完成并提交 `a8d8ca1`**（2026-11-03 主线）：启动器 60s 可配/自动化不弹窗、
  便携 go2rtc.yaml 指向包根、doctor 源模式优先 server/.env；安装器/驱动拆分、控制面板卸载、
  启动脚本智能定位、Web 停止。pytest 312、smoke 8/8。
- **121：图片文件夹自动识别（已提交 `f7ba376`）**：`type=folder`、`folder_images` v5、
  新「图片识别」页、违规事件/证据、保留与删除清理；pytest 329、build 54.29KB、
  DirectML 端到端通过。详见 [任务 121](./50-tasks/121-folder-image-ingestion.md) 与
  [使用手册 68](./68-image-folder-ingestion.md)。
- **122：便携包重建（已验收 `aadabd5`）**：`LabSentinel-portable-20261103.zip` 含 121/106–108；
  smoke 8/8、doctor 10/0/0、rc 4/4、冷启动 20.0s。
- **118：Hyper-V 干净 VM（用户确认已复测）**：本机 VM **`Win10 测试环境`**；后续仅补记录，
  不再重跑；**禁止 VM 内推理/压测/24h/GPU**；VM/WSL 无密码，不猜密码。
- **123–129（2026-11-04 复审新增）**：123 防休眠（P1）、124 VLM 误报预判（P2）、
  125 MQTT 事件总线（P2）、126 插件一期（P2）、129 发布卫生（P3）；127/128 留 backlog。
  路线见 [docs/72](./72-project-review-2026-11-04.md)。
- **111–117：Windows 功能补齐已派发（2026-10-31）**：111 摄像头分组/批量启停、112 规则日期/星期/假期、
  113 局域网发现助手、114 worker 守护自动重启、115 PWA/桌面通知、116 规则模板库、117 更新检查
  （只提示不自动装）。**Docker/Linux 部署不派发**；WSL Fedora 只用于开发调试。
- **76–97：已交付并主线验收（2026-10-31，commit `5166496`）**：路径/URI、gzip 保护、24h 准备包、
  配置导入导出、区域规则复制、告警静默/升级/模板/过滤、只读令牌、安全头、限流、模型状态、
  定时备份、日志查询、指标、配置校验、筛选导出、RC 脚本；pytest **286 passed**、build **50.55KB**、
  release_check PASS（2 TODO WARN）。**84/87 代码+测试完成，报告仍占位待补**。
- **75：已修复并由主线复验**（USB 双布局/包内脚本/脱敏/release_check/X-Request-ID，新 zip
  278.1MB）。**残留冷启动 ~29s 触发 30s 弹窗、go2rtc.yaml 路径、doctor 源模式假 FAIL → 任务 119**。
- **98–104：未开工**（4 路压测、Runbook、发布文案、OpenAPI、质量统计、单条证据、最终 RC）。
- **75–104：第二批 30 项已派发（2026-10-30）**；顺序/优先级/冲突见
  [批次75-104-夜间执行说明](./50-tasks/批次75-104-夜间执行说明.md)。P0 为 **75 便携包发布回归修复**
  与 **76 路径/URI 安全**（主线验收发现：USB ffmpeg 便携路径、缺 backup/diagnostics 脚本、诊断包
  SMTP 密码泄露、release_check 误报、`_safe_within` 子串误判、只读 SQLite 特殊字符路径）。
- **57–74：已完成、验收并提交（2026-10-30，commit `6aae48e`，pytest 165 passed）**；历史清单与规则见
  [批次57-74-夜间执行说明](./50-tasks/批次57-74-夜间执行说明.md)。串行顺序：
  1. **57 多路模拟 RTSP 压测**（`57-multi-rtsp-sim.md`）：go2rtc 3 路 + 8001，10 分钟受控；
     **执行报告仍为占位 = 尚未开工**；模拟≠真实 24h。
  2. **58 发布产物清单/校验和**（`58-release-manifest.md`）：`scripts/release_manifest.py`，
     只读现有 zip 产 `SHA256SUMS.txt` + `RELEASE-MANIFEST.json`，`--verify`。
  3. **59 便携包一键冒烟**（`59-portable-smoke.md`）：`scripts/smoke_portable.py`，
     完全隔离 LOCALAPPDATA，zip→启动→健康→首启播种→停止→清理。
  4. **60 USB 设备枚举 API+UI**（`60-usb-device-discovery.md`）：`GET /api/cameras/usb-devices`
     （ffmpeg dshow 解析、10s 缓存）+ Cameras.vue 下拉选择。
  5. **61 文件日志轮转+诊断包**（`61-logging-diagnostics.md`）：RotatingFileHandler（5MB×3）、
     `/api/diagnostics/summary`、`scripts/collect_diagnostics.py`（脱敏 zip）。
  6. **62 备份/恢复 CLI**（`62-backup-restore.md`）：SQLite backup API、verify、restore（默认保守、
     路径穿越防护、`.bak-<ts>`）。
  7. **63 健康探针/统一错误/请求 ID**（`63-health-errors-requestid.md`）：`/api/health/live|ready`、
     `X-Request-ID`、错误体 `{detail,request_id,code}`。
  8. **64 SQLite WAL + busy_timeout**（`64-sqlite-wal-concurrency.md`）：读写并行、并发写测试、62 回归。
  9. **65 摄像头可观测性**（`65-camera-observability.md`）：last_error/重连次数/帧龄、单路状态端点、UI。
  10. **66 前端加载/错误/空状态**（`66-frontend-states.md`）：四页三态、轮询失败不清空、按钮防重。
  11. **67 通用 Webhook 告警**（`67-generic-webhook.md`）：HMAC 签名、http 开关、本地接收器测试。
  12. **68 保留预演+磁盘预警**（`68-retention-preview-disk-alert.md`）：preview 只读、Dashboard 磁盘卡。
  13. **69 schema 版本/迁移框架**（`69-schema-migrations.md`）：schema_version、旧库兼容、失败回滚。
  14. **70 doctor 自检**（`70-doctor.md`）：只读检查模型/web/DB/端口/磁盘，入便携包。
  15. **71 安装器静默/自定义目录**（`71-installer-silent.md`）：修 `--install-dir` 失效、
      `--silent/--dry-run`、隔离测试。
  16. **72 静态缓存 + 选择性 gzip**（`72-static-cache-gzip.md`）：assets immutable、index no-cache、
      不压 zip/video。
  17. **73 运维手册**（`73-ops-manual.md`）：日常巡检/排障/升级/备份/安全一本通。
  18. **74 发布文档终检**（`74-release-final-check.md`）：`release_check.py` + 发布核对报告 + 发布清单。
- **并行约束**：57 是重负载，必须独占；58→59 都动 `dist/` 与 `docs/59`；60/65/66 都动 `Cameras.vue`；
  61/63/72 都动 `main.py`；62/64/69 都动数据库相关；67/68 都动 config/Dashboard。
  一次只做一个，写前重读共享文件。

## 三、最近提交

```
f9dd9cb docs(dispatch): 批次57-62派发（压测/发布清单/冒烟/USB枚举/日志诊断/备份）
04cb605 feat: 批次51-55交付并验收（播种单源/CI/EXE/文档口径）
cb26e4d docs: 主线暂停交接快照 + 任务57多路RTSP压测派发
ec5443a feat: USB/OBS 虚拟摄像头接入验证 + usb 源解析修复
764094a docs(dispatch): 新增任务55 启动EXE（无黑框，脚本/EXE双入口）
60072ee docs(dispatch): 复审后新批 51-54（播种单源/CI便携包/M2备忘录/文档口径）
db19dc9 fix+docs: 项目复审与三处兜底修复（preannotate/一键启动/便携移动自愈）
```

## 四、本轮验收证据（2026-10-30）

1. `pytest tests/ -q` → **91 passed**（2 个 Pydantic 弃用警告，非阻塞）。
2. `cd web && npm run build` → 通过，`index-*.js` gzip **42.77KB**。
3. 便携包条目核对：`LabSentinel.exe`、`runtime/pythonw.exe`、`scripts/seed_demo.py`、
   `scripts/portable_launcher.py`、`media/*.mp4`、`启动.bat`、`安装.bat`、`app/VERSION` 全部存在；
   zip **264.8MiB / 12544 项**。
4. 51–55 代码/文档逐项核对后提交 `04cb605`；工作区干净。

## 五、未提交工作

**无**（2026-10-31 11:25 `git status` 仅 `.zcode/` 工具目录未跟踪）。76–97 代码+文档已提交
`5166496`（pytest 286、build 50.55、release_check PASS）；105 在 `ac7d87e`。
- 75 的便携修复已包含；残留冷启动/go2rtc 路径/doctor 源模式在 **119**，修完重跑 118。
- 84/87 代码+测试已完成，报告占位待补；不阻塞后续任务。
- 75/76 中已由审阅覆盖的条目不要重复改，报告注明“已覆盖”。

## 六、发布差距（v0.1.0-alpha）

1. **真实 RTSP 摄像头 24h**（唯一硬阻塞）+ 模型止损线（Lab Coat ≥60%、误报 ≤20%/天）。
   - 工具：`tools/camera_soak.py` + `docs/摄像头24h验收清单.md`。
   - 虚拟摄像头 / 3 路模拟 RTSP **不能**替代。
2. 干净 Windows VM：安装/启动/卸载/自启（46 在本机自启被安全软件阻塞）；无 Python/Node。
3. GitHub 远端 + Actions 实际跑 52（当前本地 git、无远端）；Release 附件/zip+sha256/公告定稿。
4. 代码签名（可选，影响 SmartScreen/杀软）；exe 无签名、无自定义图标。
5. 不在 alpha 范围：Android APK（仅 Flutter 骨架）、iOS/HarmonyOS、人脸实测（默认关）、
   真实多路吞吐、NPU 打包、GB28181；告警真实 webhook 未实测。
6. 57–74 已补上：多路压测数据、发布清单、冒烟脚本、USB 枚举、日志轮转/诊断、备份恢复、
   健康探针/请求 ID、SQLite WAL、相机可观测、前端三态、通用 Webhook、清理预演/磁盘预警、schema 迁移、
   doctor、安装器静默、静态缓存、运维手册、发布终检。75–104 继续补：便携发布链修复、路径安全、
   告警静默/升级、批量处置/搜索、审计/只读令牌/安全头/限流、自动备份、日志/指标、配置校验、
   24h 准备包、4 路压测、OpenAPI、质量统计、单条证据、最终 RC。
   剩余未完善：自动更新、多用户/角色、导出上限/PDF 页级解析（真实 24h 仍是硬阻塞）。

## 七、恢复/继续工作建议（新会话直接照做）

1. 读本文件 + `docs/50-tasks/调度看板.md` + `docs/50-tasks/派发状态.json`。
2. **当前 131**；按 [批次 130–139](./50-tasks/批次130-139-发布候选增强.md)：
   131 插件 UI→132 AI 徽标/筛选→133 前端 vitest→134 安全审计→135 View Transitions→136 时间线→
   130 最终 RC。阻塞记录后跳下一项，不等待主线；flash 不自行 commit。
3. 每项完成后主线验收：
   - 通用：`cd server && D:\LabSentinel\m0-venv\Scripts\python.exe -m pytest tests/ -q`；
     前端改动 `npm run build`，133 起加 `npm test`；
   - 131/132：UI 实跑 + 权限/空态 + build；
   - 133：`npm test` 全绿、CI workflow 含 npm test；
   - 134：审计报告 PASS/WARN 清单 + 高危修复；
   - 135/136：浏览器手工验证 + API 对拍。
4. **130 最终 RC**：`python scripts/build_portable.py --zip`；manifest --verify、smoke、doctor、rc_regression；
   更新发布核对报告（数字以 manifest 为准）。
5. 用户接真实摄像头后：按 `docs/摄像头24h验收清单.md`（配合任务 99 Runbook + 123 防休眠）跑 24h
   → 通过才发布 v0.1.0-alpha。
6. **不要**重新创建每 5 分钟自动巡检，除非先修"心跳"：任务文档 mtime 会误判（flash 写代码不写文档），
   应把 `server/app`、`web/src`、`scripts` 最新 mtime 也算进度；且运行中的 cron 无法自删，需主会话删。

## 八、环境要点

- 项目根 `D:\User\Desktop\TOOLS\监控识别`；首选 venv 项目 `.venv`（py3.12，`scripts\venv_path.txt` 相对路径）；
  旧装兼容 `D:\LabSentinel\m0-venv`（py3.14，pytest/训练/工具）；NPU venv `D:\LabSentinel\npu-venv`。
- 普通用户：便携包 `安装.bat` / `LabSentinel.exe` / `启动.bat`；开发：`一键启动.bat` / `scripts/start-all.bat`。
- 单端口：`npm run build` 后只开 8000；`LS_WEB_DIST` 可覆盖。
- USB/OBS：`tools/list_cameras.py` 枚举索引；`python tools/virtual_cam_feed.py` 喂 OBS 虚拟摄像头；
  服务端 `type=usb, source=1`。ffmpeg 在 `tools/ffmpeg/ffmpeg.exe`。
- go2rtc：`tools/go2rtc.exe`；1.9.15 **不支持** `ffmpeg:` 源，必须 `exec:ffmpeg ... -f mpegts pipe:1`。
- 铁律：DML 禁并发 run（`Engine._run` 串行锁）；重负载一次一个；supervision 0.25.1；onnxruntime 互杀；
  Git Bash curl 中文 JSON 损坏（用 httpx）。
- 镜像：pip 清华/阿里云/官方；npm npmmirror/官方；GitHub ghfast；HF hf-mirror；Roboflow 代理 7890。

## 九、文档地图

| 想知道什么 | 看哪里 |
|---|---|
| 协作/任务状态 | `50-tasks/调度看板.md` + `派发状态.json` |
| 本批任务规格 | `50-tasks/57-multi-rtsp-sim.md` … `62-backup-restore.md` |
| 项目复审（bug/优化） | `62-project-audit-2026-10-28.md` |
| USB/OBS 摄像头 | `64-usb-virtual-camera-report.md` + `50-tasks/56-...md` |
| 图片文件夹自动识别 | [68-image-folder-ingestion.md](68-image-folder-ingestion.md) + [121](./50-tasks/121-folder-image-ingestion.md) |
| 打包分发 | `58-packaging-strategy.md` + `59-portable-install.md` |
| M2 方向决策 | `63-m2-options.md` |
| 模型评估 | `52-model-eval.md` |
| 发布 | `70-release-plan.md` + `发布核对报告.md` + `发布公告.md` |
| 真实摄像头 24h | `摄像头24h验收清单.md` + `tools/camera_soak.py` |
| 架构/API | `30-architecture.md` / `35-api-reference.md` |
