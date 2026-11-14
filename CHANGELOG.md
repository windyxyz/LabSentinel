# Changelog

本文件记录 LabSentinel 的用户可见变更。格式参照
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)；版本号遵循 SemVer
（决策 77-Q-05：下一版 0.2.0；构建元数据 `+<YYYYMMDD>.<shortsha>`）。

## [0.2.0] - 2026-11-14（Rust 形态重发布）

> 同一版本号二次发布：0.2.0 初版（2026-11-12，Python 双形态）发布后，
> ADR-0011 过渡期完成、Python 运行时退役，正式形态改为 Rust 单二进制 zip。
> 本段起为重发布内容；原 0.2.0（2026-11-12）段落保留在下方作历史记录。

### Inno Setup 7 安装器恢复为必发布资产（2026-11-14 补）
- **发行形态补齐**：`LabSentinel-setup-<版本>.exe` 与 zip 同为必发布产物。
  `deploy/windows/installer.iss` 重写为 Rust 形态：源为
  `Resources/dist/rust/labsentinel-server` staging 树，Inno Setup 7.1.1 编译
  （`scripts/build_setup_exe.py`，版本取 `VERSION`，ISCC 支持环境变量覆盖）。
- **安装器能力**：中英双语；双模式（默认「仅为我安装」→ `%LOCALAPPDATA%\Programs\LabSentinel`，
  提权「为所有用户」→ `%ProgramFiles%\LabSentinel`）；完成页可选开始菜单/桌面快捷方式；
  升级前 `labsentinel-server --archive` 归档旧版、安装后 `--selftest-http` 真实探活
  （失败可 `--rollback`）；卸载先 `taskkill` 结束常驻服务再删文件，并清理自建快捷方式
  （`CreateShellLink` 产物不在 Inno 安装日志内）；数据目录不随卸载删除。
- **CI**：`release.yml` 的 build-rust 增加「安装 Inno Setup 7.1.1（官方安装器 + SHA256
  钉死，choco 仅有 6.x）」→「编译 setup.exe」→「安装→探活→卸载零残留冒烟」；
  `SHA256SUMS-rust.txt` 同时列出 zip 与 setup.exe，artifact/publish 清单同步。
  此前随 Python 退役一并移除安装器属误删，本次恢复。

### CI 门禁重构收口 + RustSec 漏洞清零（2026-11-14）
- **CI 全绿（main @ 4568a45）**：`tests`（Rust nextest 双平台 + Web）与 `rust`
  （test/clippy/fmt + audit + route_coverage）全绿。
- **依赖安全升级**：rusqlite/libsqlite3-sys 0.31→0.41（bundled SQLite ≥3.54.1，
  RUSTSEC-2026-0459）；ureq 2→3（rustls 0.23.46，RUSTSEC-2026-0346/0349；
  全调用点迁移到 `Agent::config_builder().timeout_global` API）；zip →9.5.1
  （RUSTSEC-2026-0367~0429 解压内存放大/符号链接提权批）；rumqttc 0.24→0.25
  （rustls 0.23 + ring provider 显式安装）。
- **cargo-deny 钉 0.19.9**：0.20 的 spdx 升级使 allow 列表复合 SPDX 表达式逐项
  拒绝（全量误报），0.18.3 解析不了 advisory 库的 CVSS 4.0 条目；deny.toml 重写为
  单 id 白名单 + 显式 RUSTSEC ID 豁免（rumqttc 钉死的 rustls-webpki@0.102.8 与
  rustls-pemfile@2.2.0，理由入库）；cargo audit 同口径 `--ignore`。
- **CI 修复**：rust-toolchain 显式 `toolchain: stable`；ls-proto build.rs 旁路目录
  改 OUT_DIR ASCII（修 Linux protoc）；sccache 移除；无工具链的交叉编译矩阵 job 删除。

### RA4.2：Python 运行时退役，Rust 单二进制成为唯一发布形态（2026-11-14）
- **发行形态切换**：`release.yml` 只构建 `labsentinel-rust-<版本>-<日期>.zip` +
  `SHA256SUMS-rust.txt`（包内自带检测模型、ffmpeg/ffprobe、onnxruntime、前端与
  `LabSentinel.bat` 启动器）；`tests.yml` 门禁改为 Rust workspace + Web，
  Python pytest/ruff/openapi 门禁随运行时下线。
- **退役清理**：删除 `server/`（FastAPI 运行时与 631 例测试）及 Python 便携打包链
  （`build_portable` / `install_portable` / `installer` / `portable_launcher` /
  `smoke_portable` / `deploy` / `doctor` / `health_check` / `backup_data` /
  `seed_demo` / `export_openapi` / `release_manifest` / `release_check` /
  `rc_regression` / `probe_runtime` / `secrets_rekey` / `verify_sidecar_evidence` /
  `opentip_hashes` / `make_sbom` / `soak_mixed_gray` / `build_launcher_exe`）；
  Inno 安装器随后按「必发布资产」决定以 Rust 形态恢复（见上节）；`tools/` 训练工具链
  与 `scripts/` 构建/验收支具保留（Python 仅作开发工具，不进分发）。
- **替代能力**：`labsentinel-server --archive/--rollback` 承接版本归档/回滚
  （排除 data/、拒绝路径穿越、回写 current.txt）；`prepare_rust_assets.py` 按哈希锁
  准备模型/ffmpeg/onnxruntime，`build_rust.py --strict` 资产齐全才出包；
  `/api/system` 版本读包内 `VERSION`；默认数据目录改为 `data/`（跟随 `LS_DATA_DIR`）。
- **退役前全量门禁**：Python 631 passed / 7 skipped；Rust workspace **122 passed**；
  Web **44 passed** + build；`acceptance_rust_only` **39/39**；`acceptance_vlm`
  **9/9**；`smoke_rust` PASS。
- **验收发现并修复**：配对 `claim` 限流桶与普通写共用（写满 10 次后配对必 429）→
  独立桶；引导向导/目录选择器误用未定义 CSS 变量导致浅色主题黑字压深底不可见 →
  统一设计令牌，并修复向导「浏览…」不弹窗、完成后不关闭。

### RA5：外部联网 VLM 检测后端——模型模块化（ADR-0011 增量，2026-11-14）
- **底座/模型解耦**：检测后端可插拔三选一——① 本地训练 ONNX（YOLO/tract，
  LS_MODEL_PATH）② 用户自备第三方 ONNX（任意路径，/api/models/local 入册）
  ③ **外部联网模型**（新增 `vlm.rs`：openai_compat 通用协议 / ollama 本地 /
  **gemini 原生视频**三 provider）。
- **图片检测接 VLM**：folder worker `LS_ANALYZER_BACKEND=vlm`（或 per-camera
  `PUT /cameras/{id}/analyzer {"analyzer":"vlm"}`）→ 单图交外部模型判定
  persons/no_ppe/violations 真值回填 + 违规事件。判定失败降级落 error 不误报。
- **视频检测接 VLM（"AI 看视频"）**：两条通路（2025 调研：原生视频输入是
  Gemini 类特例，多帧采样是通用主流）——**gemini provider 原生视频**（帧序列
  → ffmpeg 临时 mp4 → inline_data base64，<20MB 限制如实报错）+ 多帧采样
  （openai_compat/ollama，≤8 帧均匀采样首尾保留）。视频 worker 按路限流
  （默认 6 次/分钟）从证据环抽帧判定 → 违规事件走全链（入库/WS/MQTT/首发）。
- **独立部署自举**：`ls-store.ensure_schema`——全新库（零 Python 部署）由
  Rust 建 v13 全量 schema（幂等 IF NOT EXISTS；迁移链仍归 Python）。
- **模拟验收 9/9 PASS**：`scripts/mock_vlm_server.py`（OpenAI 兼容协议替身，
  内容启发式判定）+ `scripts/acceptance_vlm.py`（真实素材：演示视频抽帧
  有人帧+空场景帧）——mock 协议帧 ×2 / 判定回填真值 / no_ppe 事件入库 /
  models.local 清单 / --launch 消解件全链验证。真外部模型（需 API key）
  质量验收同口径复跑 acceptance_vlm.py 即可。
- 门禁：workspace **118 passed / 0 failed**（+3 vlm 测试）+ clippy/fmt/ruff
  全绿。

### 全面安全/性能审计修复（2026-11-14，反思自查 + 双专项）
- **P0-1 WebSocket 握手零鉴权（CSWSH）**：`/ws/events` 此前无 Origin 校验、
  无令牌校验——恶意网页 drive-by 可实时读取事件流并绕过读保护。修复：Origin
  同源/白名单校验 + read_protected 时子协议 `bearer.<token>`/Bearer 令牌校验
  （语义对齐 Python `_ws_auth_check`），并 echo `labsentinel` 子协议。
- **P0-2 首发派发阻塞帧循环**：dispatch_event 是同步 ureq（单渠道最坏 ~40s
  ×8 渠道），跑在相机帧循环线程上会让该路停摆数分钟——改投后台线程
  （对齐 Python ThreadPoolExecutor 异步语义）。
- **P1 批**：设备令牌降级 triage-only（此前可 shutdown/config import/plugins
  start——权限放大）；插件子进程环境白名单重建（不再继承 LS_ADMIN_TOKEN）+
  start 重入拒绝（防进程增殖/句柄孤儿）；folder 原图端点 containment 校验
  （DB path 被污染时防任意文件读取）。
- **P2 批**：常数时间令牌比较（ct_eq）；`/api/logs` 脱敏 + 2MB 尾部读取 +
  不回显绝对路径；GET 管理分级（audit/devices/config/plugins/face-persons）；
  写限流 120/min + pairing/claim 10/min/IP（testing 豁免）；request-id 随机化
  （原为可预测纳秒时间戳）；ppe_download 流式下载按期望大小截断；diagnostics/
  ppe 状态不再回显内部绝对路径。
- **反思自查批**：folder worker 每 tick（3s）重复加载 YOLO 模型 → 提到循环外；
  genai reviewed/failed/queue_depth 计数恒 0（stats 未共享给消费线程）→ Arc
  共享修复；face 客户端显式校验 HTTP 状态码（ureq 2.x 非 2xx 返回 Ok）；face
  注册流式读取（10MB 上限在读取中断，不再全量进内存）；插件 state.json 原子写；
  `{python}` 占位不再替换成本服务自身 exe；launch 模式 bind 失败优雅退出。
- **Python 侧回归修复**（性能审计发现）：`pipeline.py:257` T-308 把录像 done
  检查留在 `for rec` 循环外（引用最后一个 rec）——多录像时除最后一个外永不
  置 done，**视频永不回填**；移回循环体内（2026-11-06 回归）。
- 门禁：workspace 115 passed + clippy `-D warnings` + fmt 全绿；Python 存证
  12 例 + 全量回归跑测；安全专项 + 性能专项两轮独立审计覆盖认证覆盖/注入面/
  路径穿越/SSRF/信息泄露/DoS/热路径/锁并发/死代码/双实现漂移十个维度。

### Rust 重构 RA4：部署/启动链替换评估 + 核心消解件（ADR-0011，2026-11-13）
- **逐项判定落地**（见 roadmap RA4 能力矩阵）：Inno 消解 5 项（文件/快捷方式/
  卸载键/自启/覆盖升级）；Rust 重写 2 项 ✅；豁免 2 项（deploy.py/tools/）；
  升级归档/回滚保留 Python 至 RA4.2（ADR-0010 过渡周期）。
- **`--launch` 消解件**：端口试探防双实例（已被监听 → 开页面即退）+
  launcher.lock "pid port" 契约对齐（位置 data 上一级 logs/）+ 就绪开浏览器。
- **`--selftest-http` 消解件**：health_check.py 的 Rust 等价——真起服务于
  127.0.0.1:0（隔离临时数据目录），轮询 ready ≤60s；本机实测 ready=true
  exit=0；Inno finalize 后续切此子命令（Python 安装器保留期内并存）。
- 门禁：workspace 115 passed + clippy `-D warnings` + fmt 全绿。

### Rust 重构 RA3：L4 单二进制形态（ADR-0011，2026-11-13）
- **Rust 形态便携包转正式产物**：release workflow nightly-rust job 重写为
  web build → `build_rust.py --zip` → **冒烟** → artifact + `gh release upload`
  （独立 job，Rust 失败不阻塞 Windows 便携主链）。
- **冒烟**：`scripts/smoke_rust.py`——隔离解包 → 启动 → 就绪轮询
  `/api/health/ready` → `/api/system` 断言 `backend=rust` → 前端 SPA
  （根标记 + immutable 资产抽验）→ 停止清理；本机实测 4/4 PASS。
- **体积实证**：Rust 形态 **30.8MB** vs Python 便携包 198MB——嵌入式
  CPython runtime 退役（RA4 的部署链替换以此为基线）。
- build_rust.py README/.env.example 更新（RA3 正式口径 + worker 环境变量说明）。

### Rust 重构 RA2：S5~S9 批次——503 占位清零（ADR-0011，2026-11-13）
- **S5 GenAI 事件复核**：`genai.rs`——默认 provider=none 零开销；单线程有界
  队列（max=100 满丢计数）；ollama/openai_compat 双 provider（ureq）；严格
  JSON 解析（verdict 白名单/reason≤200/confidence 夹取/围栏剥离）；回写
  `events.ai_review*` 绝不改 status；回环红线（非回环需
  LS_GENAI_ALLOW_REMOTE）；`/genai/status` 九字段真实。
- **S6 face 控制面**：`face.rs` CompreFace REST 客户端（ureq 手工 multipart，
  中文 subject URL 编码）；5 端点摘 `not_available_standalone`——注册
  （consent=true 强制/10MB 上限/同意落 face_consents 表）、列表（subjects ∩
  同意记录）、删除/一键清除（errors 数组）；合规门 403/503 分支保持。
- **S7 plugins 管理器**：`plugins.rs`——manifest 严格校验（deny_unknown_fields/
  id 正则+目录名一致/permissions 白名单/env 禁 LS_*）、state.json 持久化、
  start（{python} 占位/PATH/目录锚定 + CREATE_NO_WINDOW + 日志追加）/
  stop（强杀）/logs tail；6 端点摘 404 占位；坏清单 error 项不崩。
- **S8 update/check**：清单拉取真实现（1h 缓存、错误不缓存可重试、https
  强制 + LS_UPDATE_URL_HOSTS 白名单、只升不降版本比较）；**minisign 签名
  验证未实现如实标注**（不引新 crate 的分水岭决策，T-403 证书到位时一并接）。
- **S9 model/status 形状对齐**：补 backend/imgsz/conf/analysis_fps/classes
  （file/exists/count/names，回退 yolo11n_ppe.classes.txt）/ppe 参数组/
  face_enabled。
- 门禁：workspace **115 passed / 0 failed**（+5 RA2 契约测试）+ clippy
  `-D warnings` + fmt 全绿。RA2 除豁免项外 503 占位清零。

### Rust 重构 RA2：S1/S2/S3 批次（ADR-0011，2026-11-13）
- **S1 PPE 参数化进 lib**：`YoloPpeRunner`/`PpeConfig`/`anchored_child`/
  `load_class_names` 从 bin 迁入 `ls-analyzer/src/ppe.rs`（sidecar 走 env 包装，
  协议不变）；ls-server worker 与 folder worker 经 `cfg.ppe` 注入管线
  （未配模型 → 仅人员检测，ppe 恒 Unknown）。
- **S2 MQTT 事件桥接线**：`AppState.mqtt = ls_jobs::mqtt_from_env()`（默认关零
  行为变化）；`EventSink::record` 落库后发布 payload v1（独立于静默/渠道过滤，
  与 Python alerts.dispatch 同位）；`/mqtt/status` 从 `MqttStats.snapshot()`
  出真实 published/dropped/failed/connected 等九字段。
- **S3 folder 识别管线**：`ls-ingest::decode_image`（ffmpeg 单图解码 BGR
  1280×720，零新依赖/中文路径安全）；folder worker 从「仅扫描登记」升级为
  解码 → YOLO 检测 → 区域锚点判定 → PPE 分级 → zones/violations/key →
  违规 JPEG 快照 → 全字段回填 → 违规事件经 EventSink（入库+WS+MQTT+首发派发）
  → `set_folder_image_event` 关联；每 tick 限额 `LS_FOLDER_MAX_PER_TICK`。
  ls-store 增 `update_folder_image_analysis`/`set_folder_image_event`。
  解码失败行如实落 `error` 字段；已识别行按 width 跳过防重复建事件。
- 门禁：workspace **110 passed / 0 failed** + clippy `-D warnings` + fmt 全绿。

### Rust 重构 RA2 开工：/api/system 契约补齐（ADR-0011，2026-11-13）
- `today_counts`/`unhandled_today`/`alerts` 从空占位变真值：ls-store 新增
  `today_event_counts`/`count_events_today_handled`/`delivery_counts`
  （SQL 口径对齐 Python 同名方法，today 用 SQLite `localtime` 取 OS 时区）。
- 已知差异如实记录：folder worker 写 UTC ts（std 无本地时区），UTC 16:00 后
  落「昨天」的口径偏移待 RA2 统一（同一路相机同一时刻只由一个实现写入）。

### Rust 重构 RA1.3：告警首发派发 + folders/scan（ADR-0011，2026-11-13）
- **告警首发派发**：`ls-jobs::Jobs::dispatch_event`——worker 候选事件落地后经
  与 escalation 同一链路组件（注入渠道快照/env 回退 → 静默匹配 → 渠道过滤 →
  限流门 → add_delivery → deliver → update_delivery）推送 8 渠道；WS 广播
  （events_tx）与外部投递解耦——投递失败不影响事件已入库/已广播。
- **folders/scan 摘 503**（最后一条 503）：`POST /api/folders/{cid}/scan`
  wake-or-start（Python folder_scan 契约）；**folder worker**（Rust 紧凑版）——
  轮询 + wake 即扫 + 目录缺失 error 持续重试 + stop → stopped，扫描
  （ls-ingest `scan_candidates`，Python `_candidates` 同口径：扩展名白名单/
  大小上限/静置期/递归/数据目录豁免）→ `add_folder_image` 登记入库
  （唯一键幂等）。**如实口径**：识别字段 0 占位——推理需图像解码依赖，RA2
  接识别管线后回填（Python 侧本就声明 folder 源暂不在 Rust 数据面范围）。
- **路由覆盖 106/106 = 100%**（L3 门槛 95% 早已越过；`rust_route_coverage.py`
  实测「未实现 0 条」；超出 OpenAPI 的 `GET /api/ws/events` 为既有契约文档项）。
- 门禁：ls-server 37 例（+4）、workspace **109 passed / 0 failed** + clippy
  `-D warnings` + fmt 全绿。

### Rust 重构 RA1.2：worker 生命周期（ADR-0011，2026-11-13）
- **ls-server `start_camera` 摘 503**：`WorkerConfig::from_db` 从 DB 组装
  （cameras/zones（polygon 解析）/rules（含 RA1.1 的 days_of_week）/calendar），
  专用线程跑帧循环（取流 → 检测 → 管线 → EventSink 落地 + 存证收尾线程回填
  MP4）——Rust 控制面独立模式首次可真实启动摄像头分析。
- **重启策略对齐 Python CameraManager**：退避序列 1/5/30/60s、存活 >5s 重置
  计数、超 `max_restarts` → `cameras.status='error'`（given_up）+ 注册表自清理；
  文件非循环源播完自然 `stopped`；stop 路径强杀 ffmpeg slot 即时打断退避。
  已知粒度差异（RA1.2 接受）：Python 为「worker 线程 + Manager 外层重启」两级，
  Rust 合并为单线程内重启（外部可观测行为一致）。
- **detector 参数化**（make_detector env 依赖摘除）：`DetectorConfig` 迁入
  ls-analyzer lib（`detector.rs`），sidecar 走 `from_env` 包装（env 协议不变），
  ls-server 从 env `LS_MODEL_PATH`/`LS_ANALYSIS_FPS` 等组装（与 Python 全局
  设置同源同默认）。
- **补齐 DELETE /cameras/{cid}**：清单里有此路由但 router 从未注册（请求恒
  404）——现在停 worker → ls-store 级联删 → `{"ok":true}`（Python 契约）。
- bulk start/stop 接 worker；`camera_snapshot` 返回 worker 最新抽帧 JPEG
  （503 no_worker 语义保留给未运行状态）。
- 门禁：ls-server 33 例（+6 生命周期/组装/快照测试）、workspace **105 passed /
  0 failed** + clippy `-D warnings` + fmt 全绿。

### Rust 重构 RA1 首切（ADR-0011，2026-11-13）
- **ls-analyzer 拆 lib 目标**：pipeline（规则/候选/迟滞冷却）与 evidence（存证
  环/encode_mp4）从 bin 拆入 `src/lib.rs`（零逻辑改动），供 ls-server 进程内
  复用——RA3「去 sidecar gRPC 自环」的路径铺垫；gRPC 服务本体仍在 main.rs。
- **ls-store 补两口径**：`set_event_video`（对齐 Python store.py，视频回填）；
  `rules_all` 补选 `days_of_week`（原静默缺口会让时段规则周末误触发），
  `rule_json` 同步输出该字段。
- **ls-server `worker.rs`（RA1.2 骨架）**：`EventSink` 事件落地——4096 去重
  窗口（evidence_key|behavior|track_id，对齐 Python 桥）、截图落
  data/snapshots、`add_event` → `event_json` WS 广播（无绝对路径泄漏）、
  `finalize` 复用 encode_mp4 写 `videos/ev_{eid}.mp4` 后回填 DB。3 个新测试
  覆盖去重/广播/空帧错误路径；`start_camera` 503 摘除留给 RA1.3（worker
  生命周期接线）。
- 门禁：workspace `cargo test` **99 passed / 0 failed**（基线 ≥96）+ clippy
  `-D warnings` + fmt 全绿。

### P3 收尾批（2026-11-13）
- 编码簇 14 处 `text=True` 无 errors 的 subprocess 调用统一加 `errors="replace"`
  （中文路径/GBK 输出在非中文 locale 下中途崩）。
- sidecar 通道就绪超时就地回收进程+复位端口（此前等 watchdog 下轮兜底，
  直调路径复用被占端口连环失败）；watchdog 状态补 `pid` 字段。
- plugins 双 start 竞态：并发启动折叠（杀重复进程，绝不覆盖句柄成孤儿）。
- 旧版 JSON 锁（go2rtc 覆盖期产物）在 `--stop` 无锁路径回收内嵌 go2rtc；
  launcher.lock 写入原子化（临时文件+os.replace）。
- soak 崩溃演练改按服务上报 PID 精确 kill（原按映像名全机杀会误伤其他安装）；
  升级前 `--stop` 补 30s 超时（挂死不再永久卡住升级）。
- 已知留置（RA 重构消解）：`_free_port` bind→close→子进程 bind 的 TOCTOU
  窗口——正确修法是 fd 传递，随 RA1 进程内 worker 自然消失。

### 全面审查加固（2026-11-13）
- **进程生命周期 P1**：卸载兜底强杀补身份核实（PID 复用绝不误杀）+消亡窗+锁保留；
  `deploy.py kill_ports` 按端口盲杀改杀前核实命令行；`--purge-data` 锚点校验加
  「目录内真的像 LabSentinel 数据根」第二道门（自定义安装目录同名无关目录绝不删）。
- **P2**：陈旧锁先停旧实例再清（防双实例共写库）；go2rtc 三条失败路径统一回收；
  health_check 探针超时按落盘子进程 PID 兜底强杀；finalize 健康检查补 90s 超时
  （Inno 安装不再永久卡住）；升级前备份核 rc+zip 非空；冒烟步骤 4 捕 httpx 异常族。
- **P3 快赢**：reg delete rc 如实上报；portable_stop.bat 失败回退+rc 回传；
  rc_regression 去 shell=True；冒烟 --stop 超时捕获；ffprobe 输出校验。
- **一致性**：rust.yml 补 `workflow_call`（security-weekly 周审计此前解析期即失败、
  从未真正跑过）；release_check 断链扫描递归全 docs（archive 豁免）+ 版本核对路径
  与 setup 期望名修正；**修复 21 条断链（18 文件 37 处，复跑 0 FAIL）**；
  孤儿文档收录索引、scripts/README 补 12 条目、release-notes 头部改实际发布口径。

### 停止链可靠性（2026-11-13）
- **`--stop` 不再说谎**：`portable_launcher.do_stop` 此前吞掉 taskkill 退出码，
  强杀失败也删锁、返回 0（用户看到「已停止」但服务仍活着、端口仍占用）——
  现在强杀后核实进程真死（5s 消亡窗 + 重试一次）才删锁；仍存活则锁保留
  （下次 `--stop` 可重试）并如实返回非 0。`stop_process`（启动超时清理）
  同步核实并落日志。CI dml 冒烟步骤 6 的「残留 PID」实证触发本次修复。
- 冒烟收尾防误报：`smoke_portable.reap_leftover_pids` 强杀后立即查活会把
  垂死进程误判为残留（taskkill 异步终止）——加消亡窗 + 重试，消除 CI 抖动。
- 安装/卸载链感知停止结果：覆盖升级前置停止与卸载三级降级均检查 `--stop`
  返回码，失败时降级重试并明确告警（升级遇文件占用不再莫名其妙）。
- 附带修复：`do_stop` 旧版 go2rtc 残留回收先删 launcher.lock 再回读导致
  legacy 路径永远读不到 go2rtc PID——改为先读后删。
- 测试契约同步：强杀核实路径 + 幸存者保留锁返回 1 的新增用例
  （test_stop_pid_safety 15 例全绿；安装/冒烟/停止组 57 例全绿）。

### 文档重组（2026-11-13）
- `docs/` 按用途分组：`guide/`（部署/安装与卸载/运维/Runbook/FAQ/杀软说明/快速开始）、
  `dev/`（架构设计/API 参考/MQTT/插件/模型接入/数据飞轮/需求/术语表）、
  `releases/`（发布说明/风险登记册/性能基线）、`archive/`（历史留存：愿景/M0 路线图/
  v0.1 发布草稿/S7 验证记录/预研/opentip 申诉预备）；`plan/`（活跃台账）不变。
- 新增 `docs/README.md` 文档索引；重写仓库根 README（0.2.0 口径：安装包/便携包/
  源码三种方式、Rust 控制面状态、已知限制）；架构文档并入 dev/ 并更新模块地图
  （ls-server handlers/ 11 域拆分后）。
- 全仓引用同步（25 文件）；release_check 断链 81 条全部可达（0 FAIL）。
- 安装器（ADR-0010）双模式收口：向导可选「仅为我/为所有用户」（后者开始菜单写
  {commonprograms}、卸载键落 HKLM，其余用户可见），多语言（简体中文/English），
  完成页开始菜单+桌面快捷方式双勾选（默认全勾）；ALLUSERS/per-user 双沙箱
  安装-卸载实测通过；控制面板条目由 Inno 原生键维护（去除手写重复键）。
- 发布工程（CI 实证）：release workflow 双变体各自编译 setup.exe（dml 变体
  编译后重命名对齐 zip 命名，防 artifact merge 同名覆盖），SHA256SUMS 升级为
  zip+setup 双哈希，publish 上传清单补 setup；中文语言文件随仓库
  `deploy/windows/languages/`（choco 的 innosetup 不含 unofficial translations）。

## [0.2.0（初版历史）] - 2026-11-12

### 结构化拆分 + 安装包化 + 清理（第四轮，2026-11-12）
- **ls-server 细粒度拆分**：`engine-rs/crates/ls-server/src/main.rs` 5179→1349 行，
  新增共享层 `state.rs / router.rs / middleware.rs / util.rs` 与
  `handlers/` 11 个功能域模块（events/cameras/zones_rules/stats/pairing/ops/
  alerts/config_io/export/folder_images/misc_status），`alert_cfg` 内嵌 mod 随迁；
  机械搬移零逻辑改动（fn 统一 `pub(crate)`、根 re-export 门面），便于按域归档管理。
  门禁：ls-server 24/24、workspace 96/0 + clippy/fmt，路由覆盖率 105/106 不回退。
- **Inno Setup 双形态打包**（ADR-0010）：同一 staging 由
  `build_portable.py --zip --installer` 产出便携 zip 与 `LabSentinel-setup-<ver>.exe`；
  `deploy/windows/installer.iss`（按用户安装、覆盖升级、快捷方式/卸载项、可选服务
  勾选页、`finalize` 健康检查、SignTool 占位）；CI 加 `choco install innosetup`；
  59-portable-install/部署指南补双形态章节。调研覆盖 Jellyfin/mpv.net/Tautulli/
  Navidrome 等「本地服务 + Web UI」打包实践。
- **清理**：删除仓库根 78 号审阅原件（docs/plan 正本保留）与 `%SBOX%` 空目录；
  `docs/dev/架构设计.md` 重写为 0.2.0 双运行时现状；tools/README 补黄金语料
  归档工具章节；45-model-onboarding 悬空引用修正；TEMP 冒烟残留清理；
  release_check 移除已无对象的 openssl allowlist（复跑无 FAIL）。

### 便携包启动链修复（2026-11-12，冒烟 8/8 全绿）
- `smoke_portable.py` 首次完整跑通前连环暴露 4 个真实缺陷，全部修复：
  ① `health_check.py` 漏随包（R2-402 拆分后 `install_portable.py` 同目录导入失败，
  启动器一启动即 ModuleNotFoundError——冒烟「无 logs 目录」的真因）；
  ② go2rtc 锁 JSON 覆盖 `launcher.lock` 的 `"pid port"` 单实例契约（锁失效、
  go2rtc 进程泄漏）——拆独立 `go2rtc.lock` 并兼容旧格式；
  ③ 服务改探针 `--serve` 启动后 PID 复用防护 markers 匹配不到，`--stop` 拒绝
  停止真实服务（rc=0 但端口持续 LISTEN）——markers 补 `probe_runtime.py`，
  身份核实 PowerShell 改 SystemRoot 绝对路径（冒烟剥离的 PATH 下裸 `powershell`
  不可见）；
  ④ build/deploy 共 5 处 `python -c` 动态代码串全部文件化（`probe_runtime.py`
  探针统一承载，`--serve` 模式启动异常自动落盘 `serve-boot-error.txt`，
  无控制台进程链排障不再黑箱）。
- 冒烟断言对齐现状：T-406 无演示素材时 `cameras=0`+跳过播种日志为预期；
  httpx `trust_env=False`（回环绝不走系统代理）+ 网络异常家族捕获；
  `--stop` 后先等服务 PID 消失再走强杀复核。
- **更正**：本机未安装卡巴斯基，此前会话「杀软删除产物」系探测路径错误
  （产物嵌套 `portable/LabSentinel/`）；`docs/guide/杀毒软件误报说明.md` 与
  opentip 材料转为预防性参考（勿提交申诉）。

### Rust 控制面深审简化（2026-11-11/12）
- **`web_dist` 死字段接线**：`AppState.web_dist` 此前只有赋值从未被路由使用
  （文件头注释声称的「静态 web/dist（SPA 回退 + 缓存策略）」一直未兑现）——
  `build_router` 现按 `LS_WEB_DIST`（与 Python 同名）挂载静态服务：
  `ServeDir + .fallback(ServeFile(index.html))` 实现 SPA 回退；空值不挂载保持
  纯 API；静态资源在 guard 中间件内（Host 白名单同样覆盖）。**踩坑记录**：
  SPA 回退须用 `ServeDir::fallback` 而非 `not_found_service`——后者用
  `SetStatus` 把回退响应强制改写为 404（tower-http 0.5.2 serve_dir/mod.rs:236）。
- **R2-108 深审补齐**：ls-server 的 Host 白名单曾**无条件豁免** `testserver`
  主机名——Python 侧 R2-108 已按测试态门控（`settings.testing or
  PYTEST_CURRENT_TEST`），批次 4 曾声称「鉴权中间件对齐 R2-108 口径」但此处
  漏掉。现 `AppState.testing`（`LS_TESTING=1`）门控 testserver 豁免，与
  Python 一致；生产态 testserver 一律 400。
- 新增回归测试 2 例（testserver 门控三向断言 / 静态服务+SPA 回退+空值 404）；
  `cargo test -p ls-server` **23/23**。纯描述性历史注释与如实标注的能力边界
  占位（start_camera 503 / mqtt-genai 状态端点）经复核保留——均为 L3+ 迭代项，
  非死代码。
- **第二批（2026-11-12）**：① `detail_err` helper 收敛——手写
  `(StatusCode, Json(json!({"detail": "..."})))` 样板 70 处（纯字面量形态）
  批量收敛，带 `code` 字段的复合错误体保留；② 兑现文件头「缓存策略」——
  静态服务补 `static_cache_policy` 中间件：`/assets/*`（hash 文件名）
  `immutable, max-age=31536000`，index/SPA 回退 `no-cache`（发版即生效）；
  回归 1 例。`cargo test -p ls-server` **24/24**、workspace **96/0**。

### 杀毒软件误报治理（2026-11-11）
- **`LabSentinel.exe` 身份化**：注入程序集属性（产品/版本/公司/描述，版本随包
  单源 0.2.0）与 `/win32manifest` 原生清单（asInvoker——不请求 UAC 提权、
  supportedOS、PerMonitorV2 DPI）。无签名、无版本资源、无清单的裸 winexe
  拉起解释器是杀软启发式的典型画像。
- **启动链审计**：全部随包脚本无 `-c`/`exec`/`runpy` 动态执行；`urlopen` 仅
  回环健康检查；`runtime\python.exe` 来自 python.org 官方签名 embed（SHA256
  锁定）。
- 新增 `docs/guide/杀毒软件误报说明.md`：命中原因对照表、排除路径指引、误报申诉
  通道（opentip.kaspersky.com / Microsoft WDSI）；维护者本机复现注意事项
  （卡巴斯基会静默删除便携 runtime，构建期冒烟不受影响，发布产物 zip 经
  全量 CRC 校验完整）。

### 终止 Docker 支持（2026-11-11，维护者决策）
- 删除 `deploy/docker/`（compose.yaml / Dockerfile / Dockerfile.cuda / Dockerfile.rust），
  不再推进容器化开发——原已知问题（容器内监听回环、go2rtc 配置卷不共享、
  compose 双网络声明，78 号 N-07/N-08/N-19）随产物移除而闭合。
- `docs/guide/部署指南-Linux.md` 改写为 **systemd 原生部署唯一路线**（Docker 路线移除，
  附自行封装指引）；`docs/releases/release-notes-v0.2.0.md` 同步改为「Docker 支持终止」。
- 78 号文档 R2-401 状态由「暂缓」改为「终止」。

### 二轮余量收口（2026-11-11，78 号文档 §12 收尾批次）
- **R2-303 GPU 收口**：`/api/system` 透出 onnxruntime `providers`（只读探测，
  B-28 同口径，绝不触发模型加载）；Dashboard 后端卡显示 GPU 加速状态——
  providers 含 `DmlExecutionProvider` → 「GPU 加速：开启」，CPU 变体提示
  「可选 GPU(DML) 变体包随 0.3.0 提供」。DML 实机首测（AMD Radeon 880M 核显 /
  onnxruntime-directml 1.24.4 / `--duration 30 --cameras 1,4`）：单路 **69.43fps**
  （CPU 变体的 1.94×）、4 路 **17.5fps/路**（1.80×），远超 §14.3 DML 门槛
  4.5fps/路；4 路总吞吐与单路几乎持平证明 GPU 未饱和。`tools/perf_baseline.py`
  的实现标注改为按 Engine 实际后端输出（`python-<backend>`），python-dml 行
  仅在缺 DML EP 的环境才出现 n/a；`docs/releases/perf-baseline.md` 已并入实测数据。
- **R2-502 收口**：WebrtcPlayer 信令全流程前端测试 8 例（子协议鉴权——有令牌
  `['labsentinel','bearer.<t>']`/无令牌不请求、T-102 相对路径同源组装、
  offer-answer base64 往返、ontrack 已连接与 attempts 清零、断线指数退避
  500/1000ms、offer 10s 超时重连、ICE failed 3s 观察窗与窗口内恢复、卸载清理），
  前端 34→**42 例**全绿。
- **R2-309 余量收尾**：`/api/export/monthly.pdf` 落地——`ls-server` 新增手写最小
  PDF 生成器 `pdf.rs`（零新增依赖，非嵌入 CIDFont `STSong-Light`/`UniGB-UCS2-H`
  中文方案，文本 UTF-16BE hex；版式与 Python reportlab 版逐项对齐：标题/统计卡/
  每日柱状/合规脚注），单测 5 例（结构/xref 偏移/版式对齐/空月/编码）+ 集成
  测试 1 例（422/400/200/闰月/Content-Disposition）；路由覆盖 104→**105/106
  = 99.1%**（scripts/rust_route_coverage.py）。`/api/folders/{cid}/scan` 保持
  503 如实标注（需把推理管线接进 ls-server，随 L3+ 迭代）。
- **遗留复核**：Mimosa 两处 medium「疑似跨文件污点」复核为误报并注释依据
  （ls-server 审计 resource 只保留纯数字 id / ls-analyzer 存证文件名由 event_id
  经 format! 固定模板生成，无穿越面）；Q-08 PPE 资产 release URL 实测 404
  （`Windy-LabSentinel/assets` 仓库不存在），Python/Rust 两侧下载失败报错改为
  指明来源 URL 与维护者动作（`scripts/assets.lock.json` 为唯一事实源）；
  ADR-0009 已含模型来源差异（官方 vs 本地导出）与恢复计划，核对通过。

### 二轮收口复核（2026-11-10）
- 状态台账 `docs/plan/status.md` 修正：R1 安全收口 5 项（R2-105/106/107/109/110）
  此前误记为「未开始」，实际已于 `3b1fcf9` 全部落地并附回归测试，逐项补证据；
  R2-502 两处重复且口径不一（28→30 / 28→34）合并为最终的 34 例口径；新增
  「二轮收口复核」段落记录 `main @ b8211dc` 的完成度核对与三套实测基线。
- 测试健壮性：`test_folder_system_roots.py` 增加 `no_system_roots` fixture。本机
  `TMP=c:\windows\temp` 使 pytest 的 `tmp_path` 落在 WINDIR 下，被 R2-105 正确地
  判为系统目录，导致「普通目录应放行」两例失败；固定系统根/AppData 取值后，
  两种 TMP 位置下 4 例均通过。安全检查本身与两个真实系统根拒绝用例不变。
  → 干净检出下 Python 全量由 624 passed / 2 failed 恢复为全绿。

### Rust 控制面（R2-309 批次 1+2+3，2026-11-09）
- **ls-server 路由 17→99/106（93.4%，scripts/rust_route_coverage.py 实测）**：
  批次 1（61 条）：cameras 扩展（DELETE/group/analyzer/start/webrtc/camera-groups/bulk/
  usb-devices）、zones 与 rules CRUD、rule-templates、events 全套（bulk-status/timeline/
  快照/视频）、stats 四件套（daily/weekly/hourly/quality）+ metrics + CSV 导出、
  配对与设备令牌（T-620：sha256 存库、Bearer 头、读保护下可读、touch 60s 节流、
  5 次失败作废全部配对码）、审计查询、投递/静默/日历、/api/v1/meta。
  批次 2（+15 条）：copy-config（按区域名映射+去重，Python 任务80 同语义）、
  config/export（RTSP 凭证脱敏）、config/validate、media/sign（HMAC-SHA256 短期签名）、
  model/status、mqtt/genai/plugins/update 状态端点（rust 独立模式如实标注能力边界）、
  alerts/status（投递汇总）、alert-template、folder-images 列表与汇总、logs、ppe/status。
  批次 3（+23 条）：config/import（merge/replace 按名去重）、**evidence.zip 存证打包**
  （手写 stored-ZIP + CRC32，单事件与批量导出）、plugins 404 真实语义、face 分支
  （env 驱动 403/503，与 Python _face_guard 同语义）、alert-test（ls-dispatch 真实发送）、
  **ppe/download**（锁文件白名单 + SHA256 校验 + 许可确认，Q-08 复刻）、
  folder-images 详情/原文件/缩略图/CSV、alert-template/preview（render_alert 复刻）、
  rule-templates/apply（先校验后写）、system/shutdown、cameras/snapshot 503 占位。
  鉴权中间件对齐 Python（R2-103/R2-108 口径：Bearer 等价、claim 公开、读保护豁免 health）。
  批次 4（+5 条，**越过 L3 门槛 ≥95%**）：alerts/config GET/PUT/reset
  （新模块 fernet.rs——Fernet 兼容 AES-128-CBC+HMAC-SHA256，与 Python cryptography
  互操作：POSIX secret.key 语义对齐，Windows DPAPI 密文如实降级「需重填」，
  Rust 写出密文 Python 总能解）、discovery/cameras 与 discovery/probe
  （新模块 discovery.rs——std UdpSocket WS-Discovery Probe、DOCTYPE/ENTITY 加固、
  LAN 白名单过滤、TCP+ffprobe 探测）。
  **余量 2 条（有真实外部依赖，如实标注）**：export/monthly.pdf（PDF 生成）、
  folders/{cid}/scan（需推理管线）。
- ls-store 扩展对应方法 + 修复 `update_delivery` 误用 `error` 列（Python 权威列为
  `last_error`，测试自建 schema 同步更正）。
- N-34/R2-505：ls-dispatch 退避可注入（`deliver_with_backoff`），重试语义集成测试
  从 20s 真实等待降到 2s 并消除桩计数竞态。
- 审计写入在中间件内 await spawn_blocking（响应返回前落库，异步等待不阻塞执行器）。

### Rust 告警读取网页配置（R2-305，2026-11-10）
- proto `Config` 扩展 `AlertChannels`（与 ls-dispatch ChannelConfig / Python
  alert_config.FIELDS 对齐的 8 渠道字段），Python `_proto_config` 在 ApplyConfig
  快照中下发**已解密**渠道（仅 127.0.0.1 gRPC + 一次性令牌，明文只进内存）；
  Rust `ls-analyzer` 解析快照进数据面状态（空快照保持既有渠道）。
- `ls-server` 启动时读 `app_settings` 的 `alert.*` 覆盖、经 Fernet 兼容解密后
  **注入 ls-jobs**（escalation 发送优先用网页配置渠道，无配置回退 env）——
  闭合「网页端保存的渠道在 Rust 路径完全不生效」（N-12）。
- `alerts/config/reset` 改为真实删除 app_settings 行（此前置空串会残留“空覆盖”）；
  Windows DPAPI 密文如实降级（解密失败 → 渠道需重填）。
- 两侧契约代码重生成（`pass` 字段改名 `password`——Python 关键字不可作 kwargs）。

### 修复
- T-202（B-05）：便携启动器托管 go2rtc.exe（端口探测防重复拉起、日志独立、
  锁文件记录 go2rtc pid 并随 --stop 同停）——便携版实时画面恢复可用。
- T-205（B-10/F-05）：截图文件名含摄像头 ID+毫秒+短随机（多路同毫秒不再互相覆盖），
  同一帧多事件共享同一截图；未穿实验服只发 enter_no_ppe 一条（级别更高事件合并），
  规则未配置时回退 enter。
- T-206（B-11）：按区人数统计——occ_by_zone 按区去重；规则 zone_id 非空=该区人数、
  空=任一区去重人数（画面口径）；folder worker 同口径。
- T-207（B-13）：升级提醒改走 dispatcher.dispatch_text 完整派发链（静默/渠道过滤/
  投递记录/重试/死信复用），不再绕过静默直发。
- T-209（B-20）：磁盘紧急清理分三档（已处置 video → 已处置 snapshot → 按 level 升序
  删行，未处置事件被删时 ERROR 告警）；alert_deliveries 按保留口径清理（原永不清理）。
- T-112：插件子进程环境改白名单（系统必需变量 + LS_PLUGIN_*），SMTP/GenAI/MQTT/
  告警 webhook 机密不再随 os.environ 全量继承。
- T-109：脱敏补路径内令牌——飞书 hook、钉钉/企微 robot|webhook、Server酱 SENDKEY、
  通用 webhook 路径随机段（≥16 位）全部掩码；键名匹配扩 WEBHOOK/SENDKEY。
- T-203（B-06）：quality() 统计时间窗改 isoformat（原空格分隔把当天事件全排除）。
- T-208（B-14）：MQTT has_snapshot 读派发布尔字段、camera_name 走 ConfigCache 注入。
- T-001：`server/requirements.txt` 纯 ASCII 化（GBK 机器 pip 解码不再崩）并补齐
  `reportlab` 依赖（月报 PDF 缺依赖时返回 503 而非 500）。
- T-005：全部二进制下载（OpenH264/Python embed/Node/ffmpeg/ffprobe）强制 SHA256
  校验（`scripts/assets.lock.json`），移除 http:// 明文下载源；ghfast 加速代理默认
  关闭（`LS_ALLOW_MIRROR_PROXY=1` 才启用）；Node/ffmpeg/ffprobe 版本钉死。
- B-28：`/api/system` 不再触发模型加载（模型缺失/损坏时返回 200，`components.engine`
  如实报告后台预热结果）；启动器健康检查改用 `/api/health/live`。干净检出下
  14+ 个连带测试失败随之清除。
- folder 源时钟粒度缺陷（T-003 基线排查中定位）：Windows `time.time()` 粗时钟可能
  落后内核 mtime，负年龄被误判"刚写入未静置"整轮延后——clamp 为 0 后全量测试
  不再漂移（此前每轮随机挂 1-2 个 folder_source 用例）。
- 结构化修复全仓存量高危（Mimosa）：SSRF 改回环校验 + `httpx.Client(base_url=)`、
  SQL 片段字面量化/变量累积、C# 启动器常量引导、卸载器双锚点校验、
  `tools/camera_soak.py` 缺 `import os`（B-17）。

### 变更
- T-329：Rust MQTT 事件总线（rumqttc）——payload v1 与 Python mqtt_bridge
  逐字段一致、非阻塞入队、LWT/online、系统信任源 TLS。
- T-325：Rust PPE 级联（违规类 0.30 / 穿戴类 0.20、交集判定、进场帧惰性推理）——
  rust 分析器路径补齐 enter_no_ppe 事件，文案与 Python 一致；无模型时仅人员检测。
- T-326：Rust 图片文件夹候选扫描器（扩展名/大小/静置期/自激排除/新图优先/
  去重键），为 L4 folder 源迁移铺路。
- T-330/T-328：后台作业迁移评估结论入档（genai/plugins 随 L3、folder 编排随
  L4）；分析器默认值版本策略（N/N+1/N+2）注释明确。
- T-332：ls-store schema 版本常量对齐 Python 权威编号（v13）。
- T-322：补 ADR-0006 视频栈选型决策记录（ffmpeg 子进程，GStreamer 硬解后续）。
- T-327：Rust sidecar 崩溃 ≤10s 自动恢复（watchdog：进程 liveness + 全量
  快照重放 + 事件桥重连，supervisor 与事件流双触发）；Store 配置版本变化
  自动推送 ApplyConfig；/api/system 暴露 sidecar watchdog 与 analyzer_version；
  混合灰度 soak 演练修复假阳性判定（要求显式 running 且重启计数递增）。
- Q-08/T-406：实验服模型改按需下载（许可确认 + assets.lock.json SHA256
  校验 + 设置页模型管理），便携包不再随包 PPE 权重；补 fetch_assets.py、
  media/SOURCES.md（CC0 占位模板）、签名公钥占位说明。
- Q-13：每周定时审计 workflow（失败自动开 issue）+ SECURITY-EXCEPTIONS.md
  书面豁免模板 + release_check 过期豁免阻断。
- Q-03/Q-02：非回环绑定强制开启读保护；LS_AUTH_DISABLED 逃生口进入鉴权
  主链路（仅回环）并在每次启动写审计。
- T-321/T-334/T-335/T-336：golden_run --impl python|rust 统一 CLI；
  eval_report --engine rust 跨引擎一致性；model.json 角色声明（Rust/Python
  同口径读取）；rust.yml 补 nextest/sccache/msvc 目标；status 增 analyzer_version。
- T-404：安装器新增 upgrade 子命令（zip/目录、防穿越解包）+ 健康检查失败
  自动回滚（数据目录不动、升级前 DB 备份）；VM 演练测试齐备。
- T-335：engine-rs/deny.toml（许可证白名单/来源/公告）+ rust.yml
  cargo-deny/audit/5 目标交叉编译矩阵。
- T-410：release.yml 构建后 Windows zip 冒烟 + 隔离 install/uninstall 冒烟。
- T-409：release_check 增 manifest 来源/依赖锁 ASCII/无明文源/NOTICE 覆盖/签名 5 项。
- T-327 验收：scripts/soak_mixed_gray.py 混合灰度 soak + sidecar 崩溃恢复演练。
- T-505：16 处 API 请求体改 Pydantic 模型（extra=forbid）——形状/枚举/长度约束
  自动 422（此前 400），跨实体校验保持 400/404；`/events/{id}/status` 非法状态
  Python/Rust 两实现同步 422。前端请求字段全部兼容。
- T-329：Rust 告警渠道 8 通道全量接线——补 PushPlus（业务码 200 修齐）/Qmsg
  （表单+key 编码）/Email（lettre rustls，ssl/starttls/私网 none）；HTTP 超时 5s。
- T-326：Rust 分析器（gray=rust 路径）补齐**视频存证**——事件帧快照、事件前后
  2s 片段（ffmpeg libx264/yuv420p，mpeg4 兜底）、LatestFrame 实时帧；sidecar
  stdout 持续排水（防管道写满整进程阻塞断流）。Python 侧修复存证视频慢放
  （writer 帧率误用源帧率，30fps 源 3× 慢放）。
- T-336：设置页**一键切换/回退单路分析器**（python↔rust，`PUT /api/cameras/{id}/analyzer`，
  运行中自动重启）；分析器切换后 24h 内事件率偏离前 7 日日均 ±50% 时 WARNING。
- T-324/L1：Rust ByteTrack（ls-track）对齐 supervision 0.25.1 至**黄金语料逐帧一致**
  （322/322 帧：轨迹 + KF 状态）——修 KF 预测 Q 速度项 10× 误系数、removed 列表
  滞后一帧账本 quirk、补齐 `update_with_detections` 末段独立 match-back（f64 IoU +
  LAP 阈 0.5）；回放测试转正为回归门禁（testdata/vectors/real-big.json）。
- T-710：MQTT payload v1 `zones` 填事件关联区域名（原恒空占位）；dispatcher 从 detail 反查。
- T-336：/api/metrics 增 `labsentinel_cameras{analyzer=...}`、`engine_state`、
  `events_total` ——迁移期按分析器实现对比 fps/事件率，支撑灰度决策。
- T-611：Windows 服务模式交付配置 `deploy/windows/labsentinel-service.xml`（WinSW，
  LocalService，ProgramData 数据目录；Session 0 无 USB 限制已注明）。
- T-709：值班大屏 `/wall`（多路 WebRTC 网格 ≤9 + 实时事件滚动，kiosk 模式文档）。
- T-621（§9.4）：API 版本化——`/api/v1` 为正式前缀（`/api` 保留别名两个版本）；
  `/api/v1/meta` 输出类型/级别/状态文案单一来源（客户端运行时拉取）。
- T-610（Q-10）：Linux 分发三件套——`deploy/docker/Dockerfile`（CPU+OpenVINO）、
  `Dockerfile.cuda`（TensorRT/CUDA + 独立哈希锁 requirements-cuda.lock）、
  `compose.yaml`（go2rtc 共享回环）、`deploy/systemd/labsentinel.service`（加固 unit）、
  `docs/guide/部署指南-Linux.md`（含 §13.7 验收清单）。
- T-335：Rust CI（rust.yml：双平台 cargo test + clippy -D warnings + fmt check，
  workspace 零告警——修 12 处手写代码告警、生成代码在 ls-proto 内豁免）。
- T-334：`tools/README-ENGINE-BOUNDARY.md`——Python 训练链与 Rust 引擎的唯一交付契约
  （onnx + classes.txt + model.json 角色声明），禁止运行时代码回调 Python。
- T-601/T-602（S6）：平台适配层 app/platform/*——power（Win 委托 keepawake / mac caffeinate /
  linux systemd-inhibit）、devices（DirectShow / V4L2 / AVFoundation）、binaries
  （tools/<os>-<arch> → PATH）、paths（LOCALAPPDATA / Application Support / XDG）、
  process（spawn_hidden / is_our_process / kill_tree）、fonts（内置 Noto 优先）。
  pipeline 摄像头打开与视频编码按平台选后端（T-604/T-605）；plugins 用 spawn_hidden。
- T-603（X-01）：推理后端注册表（directml/vitisai/cuda/tensorrt/openvino/coreml/rocm/cpu）
  + 平台默认尝试顺序（LS_BACKEND_ORDER 空=平台默认）。
- T-620（Q-11）：设备配对与三级令牌——POST /api/pairing 一次性 6 位码（5min）→
  /api/pairing/claim 换设备令牌（只存 hash）；设备令牌可处置事件不可改配置；
  GET/DELETE /api/devices 列表与吊销；迁移 v13。
- T-404/T-405（R8.3）：安装器版本化——升级归档 versions/<ver>（普通复制，避免硬链接
  被覆盖穿越）+ current.txt + 升级前自动备份 DB + install_portable rollback 子命令；
  安装标记 .labsentinel-install.json；卸载拒绝非受管非空目录（防 rd 误删）。
- T-407/T-408：SBOM（CycloneDX python/web）与签名更新清单 latest.json（minisign 占位）生成脚本。
- T-107（S-06）：SecretStore——告警密钥入库存密文（Windows DPAPI / POSIX Fernet，
  enc:v1:base64），历史明文兼容自动升级；备份中不再扩散可解密密钥。实测 DPAPI 往返。
- T-108（S-07）：短期签名媒体 URL（/api/media/sign HMAC 5 分钟）——前端 mediaUrl
  异步批量签名，令牌不再进 URL/访问日志；?token= 保留兼容开关 LS_ALLOW_URL_TOKEN。
- T-110（S-09）：更新清单 Ed25519 签名（app/keys/update.pub 存在即强制）、语义版本比较
  （只升不降 + 旧 CalVer 兼容）、url 仅 https + LS_UPDATE_URL_HOSTS 白名单。
- T-204（B-07/08/09）：int8 条目报 model_int8 / LS_BACKUP_INCLUDE_MEDIA 生效 /
  backups.trigger_async 原子触发（手动与定时共用，消除竞态）。
- **T-330（L2）后台作业 Rust 化**：新增 `ls-jobs` crate——retention（过期事件/证据
  文件/审计/投递记录/图片记录/孤儿媒体/磁盘紧急三档，所有删除均锚定 data_dir 真实
  路径）、备份（rusqlite 热快照 + 与 `scripts/backup_data.py` 完全同格式的 zip/manifest，
  Python verify 实测可直接读取，verify/restore 保守语义）、escalation（超时未处置
  提醒，静默/渠道过滤/限流/投递记录/重试复用 ls-dispatch）；ls-server 托管三作业调度
  并新增 `/api/maintenance/{status,preview,cleanup}`、`/api/backups/{status,run}`、
  `/api/escalation/status`，L3 契约测试对**已实现路由**做 Python/Rust 双实现同套断言通过
  （覆盖率口径见下文 T-331 更正说明：17/97 路径，非全量覆盖）。
  ls-dispatch 补通用 Webhook HMAC-SHA256 签名（X-LS-Signature）与 alert_silences
  `global` 口径；ls-store 暴露只读连接供维护作业分页查询。
- **T-505 第一步（纯搬移打样）**：`api.py` 的 stats/export 段（10 路由）原样移入
  `app/routers/stats.py`，共享状态经 `routers/_context.py` 惰性代理（无导入期循环依赖）；
  `include_router` 挂在原位置 → **OpenAPI 快照与路径顺序逐字不变**（`--check` 通过），
  鉴权依赖保持 `Request` 注解避免参数集合漂移。为后续按组拆分建立可复用模板。
- **T-331 灰度切换闭环（L3 完全替代 Python worker 的最后一环）**：`analyzer=rust`
  的摄像头由 `CameraManager` 路由到 `SidecarAnalyzer`（全量 zones/rules 快照下发），
  不再创建 Python CameraWorker；新增 `sidecar_events.py` 订阅 gRPC Events 流 →
  `store.add_event` → 快照落盘 → `dispatcher.dispatch` 完整派发链（静默/过滤/投递/
  WS/MQTT），evidence_key+behavior 去重防流重连重放；sidecar 状态并入
  `/api/cameras/status` 与守护逻辑；默认全 python 零侵入，sidecar 不可用自动回退
  Python worker。默认 Python 路径行为完全不变（全量回归验证）。
- **T-508 副作用收敛 create_app 工厂**：路由挂载/中间件/异常处理/WS/静态托管全部
  收进 `create_app()`；模块级 `app` 成为工厂产物（53 处 `from app.main import app`
  零改动），中间件与 handler 去装饰器改显式注册；工厂可重入（二次实例化路由集合
  与 OpenAPI 94 路径顺序逐字一致）；lifespan 语义不变；全量 565 passed。
- **ls-track 四个移植语义修复（L1 轨迹对齐）**：① Kalman 从 f32 提升到 f64
  （supervision 全程 float64，f32 在 300+ 帧真实序列累积 2px 级误差）；②
  remove_duplicate_tracks 阈值语义写反（Python IoU > 0.85 才去重，Rust 误成
  IoU ≥ 0.15 就去重，拥挤场景大量轨迹被误删）；③ 空检测帧不调 ByteTrack
  （与 Python `_analyze` 对齐，track 状态冻结）；④ update 输出改为**输入检测框
  原样** × tracker_id（supervision 语义：IoU ≥ 0.5 匹配回填、未关联检测丢弃、
  新建 track 首帧不输出）——zone anchor/PPE 匹配随之对齐。轨迹对齐诊断工具链
  入库（export_real_track_vector.py + ls-analyzer --dets-from/--dump-tracks/
  --dump-states + ByteTrack::state_snapshot），分歧帧从 4 推进到 52（真实 322 帧
  语料，差异帧 214→183，继续收敛中）。
- **T-328 方案 A（行为变更）：Windows 推理统一 onnxruntime 1.31.1**——requirements.in
  win32 分支由 `onnxruntime-directml>=1.17` 改为 `onnxruntime==1.31.1`（directml 包无
  1.31 wheel），Windows 推理回落 **CPU EP**（Engine 注册表自动降级，DirectML GPU 加速
  随包移除；性能敏感场景可用 `LS_BACKEND_ORDER` 显式配置）。目的：与 Rust ort 后端
  同引擎版本，黄金语料对齐实测**检测层逐位一致**（40 帧 conf diff=0.000000，见
  docs/验证记录-S7-L1黄金语料对齐.md）。
- **T-321（L1）黄金语料 Rust 运行器与对齐实测**：`ls-analyzer --golden` 离线模式
  （同 Python golden_run 口径：帧序号抽帧 step=round(vfps/fps)、合成基准时间戳、
  全画幅三规则；`--dump-dets` 输出检测框诊断）；三语料实测——lab-equip 与
  lab-walk-v2 事件一致率 **100%**，lab-walk-24076 为 87.5%（18 事件合计 94.4%），
  唯一差异为 conf 阈值边界的单帧插值漏检（逐条复核入档
  docs/验证记录-S7-L1黄金语料对齐.md）；检测框对齐 **IoU min=0.9959 / p50=0.9994，
  58/58 ≥0.99**（§13.8 L1 框对齐标准达标），score 差 max=0.0045；
  tools/detect_compare.py 双实现同帧 IoU 对比工具入库。
- **T-332 三路径 DB 双向兼容自动化**：`labsentinel-server --selftest-db`（ls-store
  selftest 模块，对既有库完整读写自检）；test_store_bidirectional.py 重写为真跨语言
  三路径（Python 全量迁移库→Rust 读写→Python 复核 / 最小合法库→Rust→Python /
  WAL 并发可见性）。
- **T-329 告警派发 Rust 集成测试**：本地 HTTP 桩验证 HMAC 签名（与 Python hmac
  同算法同输出）、失败重试 1+3 次、2xx 即成功语义（Webhook）、厂商业务码判定、
  URL 白名单 SSRF 阻断；Rust workspace 47 tests 全绿。
  ONNX 全 CPU 推理（letterbox 114 灰边 + 双线性 + BGR→RGB + NCHW 与 Python 对齐，
  解码/NMS 复用 post.rs），**不依赖 libclang/ORT 动态库**；`ls-analyzer` 按实例注入
  模型路径（LS_MODEL_FP32）替换 NullDetector，SidecarAnalyzer 向子进程显式传递
  模型/阈值/迟滞参数。真实检测闭环实测：lab-walk 演示视频 → YOLO → ByteTrack →
  区域规则 → gRPC EventCandidate（enter 事件）；模型缺失自动回退空检测，CI 零阻塞。
  76 文档原定 ort 绑定因本机缺 libclang 暂缓，同一 Detector 接口后续可替换。
- **本波修复的真 bug**：① ls-track 矩形 KM 指派填充 0 抢列/越界 panic（真实检测
  触发，改大代价填充 + 只接受真实行列，补 1×3/2×3 回归）；② 事件广播持 state 锁
  blocking_send 慢订阅者死锁（改克隆列表 + try_send）；③ stop_camera 时数据面线程
  阻塞在 ffmpeg 管道 read 无法退出（共享 ChildSlot 强杀 → EOF）；④ 测试用例共享
  进程级 LS_MODEL_FP32 竞态导致 fps 测试加载 CPU 模型超时（改按实例注入）。
- **T-331（L2）分析器数据面接线**：ls-analyzer 落地单路管线——ApplyConfig 全量快照 →
  StartCamera 启动 ffmpeg 取流（ls-ingest）→ Detector（T-325 前为 NullDetector）→
  ByteTrack → ZoneState 进出迟滞 → 规则（时段/星期/日历/冷却）→ EventCandidate gRPC
  事件流；Status 实时返回 fps/帧龄/重连/错误。Python SidecarAnalyzer 改为下发真实
  摄像头/区域/规则快照（原为空配置占位）。管线单测 5 例 + gRPC 端到端（合成视频
  fps>0 → stop→stopped）实测通过。
- **L2 / L3 骨架 / L4 预备（Rust 替换 Python 运行时的分阶段进展）**：
  - ls-store（T-332）：rusqlite 打开 Python 同库（schema v13 双向兼容，ADR-0008）；
  - ls-dispatch（T-329）：8 渠道等价语义、域名白名单、静默/过滤/限流/重试/死信；
  - **ls-server（T-331，L3 控制面骨架）**：axum 单二进制，已实现约 17 条路由
    （health/system/events/cameras/stats/backups/处置等子集）。契约测试覆盖的是
    **已实现的路由**，不是 OpenAPI 全部路径；2026-11-07 二轮审查实测
    **17/97 路径（约 18%）覆盖**，距 L3 门槛（≥95%）仍有约 80 条路由待补。
    R5 阶段（R2-307/R2-309）以 scripts/rust_route_coverage.py 自动统计为准。
  - **L4 尚未开始**：Python 运行时仍是唯一可用的产品形态；
    scripts/build_rust.py 与 deploy/docker/Dockerfile.rust 是实验性 nightly 产物，
    不得进入正式 Release。tools/ 训练链按重构计划长期保留（不在 L4 移除范围）。
  - 口径更正依据：docs/plan/78-review-round2-plan-2026-11-07.md（N-27/N-30）。
- T-504（B-22）：stdout 行级 tee——各模块历史 print 以 INFO 双写进 logging（→
  server.log 轮转），/api/logs 可查后端日志；LS_LOG_TEE=0 可关。pytest 捕获流不接。
- T-326（L1/Rust）：ls-ingest——ffmpeg 子进程取流（RTSP/文件通用，rawvideo 管道，
  fps+scale 交给 ffmpeg 滤镜链），断流指数退避（0.5s×2^n 上限 30s）；合成视频实测
  2s@5fps 抽 ~10 帧 BGR。
- T-325（L1/Rust）：ls-infer 后处理纯 Rust 层——letterbox 几何（与 Python 逐位对齐）、
  YOLO 反变换解码、类别最优分、NMS；ort 推理层 feature=ort 留接口（libclang 环境具备
  后启用）。Rust workspace 全测试通过（ls-core/ls-track/ls-ingest/ls-infer/ls-proto/ls-analyzer）。
- S5 可维护性第一批：T-501 discovery 改 defusedxml（防 XXE，依赖入锁）；T-502 CSV
  公式注入防护统一到 app/csvsafe.py（事件/证据包导出补齐）；T-506 Service Worker
  只缓存 2xx 导航响应（B-25）；T-507 pydantic v2 ConfigDict（弃用告警清零）；
  T-509 ruff 自动修复 68 项（unused import/导入排序等）——顺带抓出 webrtc 代理
  WebSocketDisconnect 未导入的真 bug（F821）；T-510 部署指南同步 S1 后拓扑
  （端口/防火墙/首启令牌/WS 子协议）。
- T-401/T-409（B-15）：版本单一来源——仓库根 VERSION=0.2.0（SemVer，Q-05），
  main/api 运行时读取（version.py），app/VERSION 由构建落包，install_portable/
  web/package.json 同步；scripts/bump_version.py 一键同步；release_check 断言
  VERSION 与 web 一致 + 单源语义。更新检查的旧 CalVer 特例按 Q-05 处理。
- T-406/Q-07：便携包不再内含 Mixkit 演示视频（版权限制）——media/ 空目录 +
  SOURCES.md（存在时）；seed 播种在 media 缺失时自动跳过（首启无演示摄像头，
  引导添加自己的源）。
- T-402：release.yml 发布流水线（tag→verify→tests 复用门禁→build-win 含锁安装+
  冒烟+SHA256SUMS+manifest→draft publish）；sign/provenance 作业占位待 Certum
  证书与 minisign 密钥（77 文档人工事项）。
- T-301（PF-01）：PPE 检测单次整图推理——违规类+全部穿戴类共享同一次 letterbox+run
  （旧实现逐类别各跑一次，3 人同帧 ×2 类 = 6 次整图）；视频 worker 帧级惰性缓存
  （每分析帧最多一次），folder 源同口径复用。
- T-304（PF-05）：ORT SessionOptions 调优（全后端 ORT_ENABLE_ALL；CPU intra_op=
  核数/2；DML enable_mem_pattern=False 官方建议）。
- T-305（PF-06/F-11）：告警渠道滑动窗口限流（20 条/分钟，企微/钉钉官方口径），
  超限合并——窗口放行后首条文本附「上一窗口合并 N 条」，不丢弃不刷屏；线程池 2→4。
- T-308（PF-02）：视频存证内存优化——环形缓冲与录像任务全链 JPEG@80 + evidence_fps
  （默认 10fps）抽帧：30fps 源 720p ≈190MB/路 → ≈8MB/路；写视频时统一解码；
  低帧率源容量按源计。
- T-210（F-08）：邮件支持 LS_SMTP_SECURITY=ssl|starttls|none（默认 ssl 兼容现状；
  none 仅允许私网 SMTP，防明文口令出网）。
- T-327/T-336（L1）：ls-analyzer gRPC sidecar（tonic/prost，一次性令牌认证 + READY
  握手 + 配置快照/摄像头生命周期/事件流/状态/存证占位）+ Python SidecarAnalyzer
  （spawn/gRPC/生命周期，AnalyzerBackend 实现）+ 集成测试 3 例（exe 缺失自动 skip）
  + /api/cameras/{id}/status 增 analyzer 字段；grpcio 依赖与 analyzer_pb2 生成物入库。
- T-310（L1 前置）：AnalyzerBackend 抽象（InProcessAnalyzer 包装现有管线，行为不变）
  + cameras.analyzer 灰度列（迁移 v12）+ LS_ANALYZER_DEFAULT 配置——Rust sidecar
  （T-327 ls-analyzer）实现同一接口后即可按摄像头灰度切换。
- T-324（L1/Rust）：ls-track ByteTrack 移植——8 维卡尔曼（f32 与 numpy 舍入对齐）、
  Kuhn-Munkres 最优指派（含 (0,N) 空池 unmatched_b 全量语义）、两轮关联 + unconfirmed
  第三轮 + fuse_score + 去重，与 supervision 0.25.1 逐函数对齐；共享轨迹向量
  （tools/export_track_vectors.py 由 Python 真值导出五场景）Rust 消费逐帧断言
  轨迹划分一致；clippy -D warnings 全净。
- T-320 补：ls-analyzer gRPC 契约（proto/analyzer.proto）双向生成留待 T-327。
- S1 安全波（T-101/T-102/T-103/T-104/T-105/T-106/T-111/T-113/T-201）：
  go2rtc 只绑回环 + 随机 Basic 凭据 + 去 origin:* + 媒体端口独立 :8555 + RTSP 再分发关闭；
  WebRTC 信令改 FastAPI /ws/webrtc/{cid} 鉴权代理（浏览器不再直连 1984）；
  source 白名单（exec:/ffmpeg:#raw/YAML 注入全拒）+ JSON 转义 YAML + 允许根/系统目录黑名单；
  首启/升级自动生成管理令牌（Q-02，只动 LS_ADMIN_TOKEN 一行）；Host 白名单 +
  Sec-Fetch-Site/Origin CSRF 拦截；WS 口径与 HTTP 读一致 + 强制同源 + 子协议 bearer 令牌；
  hub try/finally 清理与并发广播超时；500 分支安全头与审计 actor 真实化。
- S3 性能止血同波（T-302/T-303/T-309）：Store 线程本地连接（with 语义不变、silences
  悬空连接修复）；ConfigCache（实例绑定 + config_version 失效，zones/rules/日历/摄像头名
  热路径不再每帧查库）；v11 索引（events.snapshot / alert_deliveries）。
- T-306：进出迟滞——连续 LS_ZONE_ENTER_FRAMES(2)/LS_ZONE_EXIT_FRAMES(3) 帧才翻转
  进出判定（=1 退化旧行为），空帧/部分消失按 exit_frames 去抖；ByteTrack 轨迹记忆
  与分析帧率匹配；FaceGate 冷却表有界。单帧漏检不再产生重复 enter/exit 告警。
- T-321/L0：确定性模式 LS_DETERMINISTIC=1（文件源跳过墙钟节流、事件 ts/规则时间窗/
  冷却全部取视频时间）；tools/golden_run.py + golden_diff.py 黄金语料工具。
  实测 lab-walk 语料两次真模型运行事件一致率 100%（L0 退出标准）。
- T-002：构建可复现——`uv pip compile --generate-hashes` 生成 win/linux 带哈希锁文件
  （opencv override 去重，只留 headless）；便携构建 `ensure_deps` 改锁驱动全新安装
  （stamp 失配即清空重装，修 B-03"依赖已存在即跳过"）；smoke 增加依赖与单发行版断言。
- T-006：前端 dev 依赖升级（vite 8 / vitest 5 / plugin-vue 6），生产依赖 0 漏洞。
- T-007/T-008：新增 .editorconfig/.gitattributes/CHANGELOG；`deploy.py find_venv`
  增加 venv 可运行性验证（损坏自动重建）。
- 目录重构：根目录只保留项目文件；效果演示/插件示例/数据集/训练产物/日志/分发包
  统一移入 `Resources/`（见 `Resources/README.md`）；GLOSSARY.md 移入 docs/。
- T-004：CI 合并为单 `tests.yml`（ubuntu+windows 矩阵、锁文件安装、ruff 门禁、
  pip-audit、npm audit --omit=dev、OpenAPI 快照校验；actions 钉 commit SHA）。
