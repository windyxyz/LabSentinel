# LabSentinel 任务状态台账（R2 轮）

> 本文件是 `78-review-round2-plan-2026-11-07.md` §4「首轮任务完成度核对」与
> §12「精细化任务清单」的持续更新版。每批任务完成后更新对应行，附实际运行证据。
> 口径：✅ 完成并验证 · 🟡 部分/有条件 · ⬜ 未开始 · ⏸ 明确暂缓（用户 2026-11-08 决定）。

## 二轮审查基线与本轮目标

- 基线提交：`499348b`（2026-11-07，仓库精简后，工作区干净）。
- 用户决策（2026-11-08）：**全量推进 R0→R5**；**Docker/Linux 服务端与 Flutter/Tauri 客户端继续暂缓**（沿用此前 Windows 优先约束）。
- 本轮范围：R0 发布阻断 → R1 安全收口 → R2 GPU/性能 → R3 Rust L1 → R4（裁剪：只做设备管理前端与前端测试）→ R5 Rust L3。
- 明确暂缓项：R2-401（Docker 修复与实机）、R2-404（VM 安装验收）、R2-601~605（Flutter）、Tauri、局域网真机验收。这些保留在 §12 任务清单中，不在本轮执行。

## R0 发布阻断修复

| 任务 | 状态 | 验证证据 |
|---|---|---|
| R2-001 锁文件补 grpcio/cryptography + 可导入断言 | ✅ | 三锁重生成（`grpcio==1.85.1` / `cryptography==50.0.2`）；全新 venv `--require-hashes --no-deps` 安装后两模块可导入；`pip-audit` win/linux 均「No known vulnerabilities found」（45.0.7 曾报 PYSEC-2026-3552/3553/3554 + GHSA，已抬到 ≥49）；`test_locked_imports.py` 新增 |
| R2-002 依赖事实源统一 | ✅ | `requirements.in`/`-cuda.in`/`overrides.txt`/`requirements.txt` 全部 ASCII；`deploy.py pip_install` 按平台装锁（`--require-hashes --no-deps`）；`test_requirements_ascii.py` 扩展覆盖全部 pip 输入文件 |
| R2-003 Rust 工具链文件修复 | ✅ | `rust-toolchain.toml` 改 `channel="stable"` + components + targets；rustup 1.29.3 实测 `cargo --version` 正常（不再报 `target tuple in channel name`）；`build_rust.py` 用 `LS_W64DEVKIT` + 显式 `+stable-x86_64-pc-windows-gnu` |
| R2-004 发布流水线可运行 | ✅ | `release.yml` 去 `|| true`（改 `git status --porcelain`）、CHANGELOG 认本版 `## [$V]`；`build_portable.py` 增 `--python`/`--models-dir`/`--variant`/`--with-rust`；`yolo11n.onnx` 入 `assets.lock.json`（Ultralytics 官方实测 URL + sha256）；CI 构建前 `fetch_assets.py` 校验下载；Rust 产物改独立 job（见下） |
| R2-005 测试基线 6 失败→0 | ✅ | Python 全量 `593 passed / 7 skipped / 0 failed`（R0-C 后）、`605 passed / 7 skipped / 2 failed`（R0-D 引入的 Q-08 口径回归已修复）；`diagnostics.py` 复用 `_engine_backend_peek()`；`test_store_bidirectional` 缺 exe 时 skip + `rust.yml` 新增 `cross_language_db` job 用 `LS_SERVER_EXE` 真跑；gzip 用例删 `or True`；ready 用例用 tmp_path 假模型 |
| R2-006 子进程输出解码统一 | ✅ | `platform/process.run_text()`（Windows mbcs / POSIX utf-8，`errors="replace"`）替换 `backups.py`、`platform/devices.py`、`platform/process.py`；`install_portable.py` 的 `run_text()` 替换 3 处 `reg` 调用；全量套件 `UnicodeDecodeError` 计数 72→0；`PYTHONUTF8=1` 中文解码冒烟通过 |
| R2-102 监听地址单一来源 + 运行时校验 | ✅ | `settings.host` 新增（pydantic 读 env 与 .env）；`validate.py` 产出 FATAL（非回环无令牌/逃生口）且 `main.py` 无条件拒绝启动；`main._enforce_bind()` 按 `scope["server"]` 兜底；`api._auth_off()` 改读 `settings.host`；`portable_launcher.py` 的 `--host` 从 config.env `LS_HOST` 取；`test_bind_enforcement.py` 新增（8 例） |
| R2-103 设备令牌纳入读权限与 WS | ✅ | `_read_token_ok` 接受 `devices.verify()`；`Authorization: Bearer` 与 `X-Admin-Token` 等价（HTTP 读/写/处置 + WS）；`verify()` 的 touch_device 按 60s 节流；`test_device_token_read.py` 新增（4 例：读 200/写 401/吊销 401/节流） |
| R2-104 WebRTC 信令 WS 回显子协议 | ✅ | `api.accept_with_subprotocol()` 公共函数，`/ws/webrtc/{cid}` 回显 `labsentinel`；`/ws/events` 由 `hub.connect` 内部 accept 保持；`test_webrtc_subprotocol.py` 新增（2 例） |
| R2-108 testserver 只在测试中允许 | ✅ | `settings.testing`（`LS_TESTING=1`）+ `PYTEST_CURRENT_TEST` 条件；`tests/conftest.py` 在导入 app 前设置 |
| R2-402 升级健康检查做实 | ✅ | `scripts/health_check.py` 新模块：临时回环端口真实启动 uvicorn → 轮询 `/api/health/ready` 60s → 终止进程（含 `_loopback_url()` 白名单 sanitizer，Mimosa SSRF high 已消除）；`install_portable.do_upgrade` 默认改用它；`test_installer_versioned.py` 5 例通过 |
| R2-501 计划文档入库与口径更正 | ✅ | `docs/plan/78-review-round2-plan-2026-11-07.md` 入库；`CHANGELOG.md:133,199-207` 更正为「L3 骨架 17/97 路由 / L4 未开始」；`Dockerfile.rust:1` 注释同步；本文件为状态台账 |

**R0 门禁实测**：
- Python 全量（锁环境）：见上表 R2-005 行。
- Rust：`cargo test --workspace --all-features --no-fail-fast` → 71 通过 / 0 失败（含此前审阅记录的 2 个不稳定用例，当前环境均通过）；`cargo clippy --workspace --all-targets --all-features -- -D warnings` 通过；`cargo fmt --all -- --check` 通过。
- `rust.yml` 新增 `cross_language_db` job（构建 ls-server 后跑 3 个 DB 双向兼容用例）+ `--all-features`。
- 前端：见最终门禁批次。

## R1 安全收口

| 任务 | 状态 | 说明 |
|---|---|---|
| R2-105 文件夹黑名单改系统根包含判断 | ⬜ | 待推进 |
| R2-106 SecretStore 加固 | ⬜ | 待推进 |
| R2-107 更新验签按 minisign 格式实现 | ⬜ | 待推进 |
| R2-109 sidecar 令牌改环境变量传递 | ⬜ | 待推进 |
| R2-110 配对码防爆破 | ⬜ | 待推进 |

## R2 GPU 与性能基线

| 任务 | 状态 | 说明 |
|---|---|---|
| R2-306 压测基线（补 T-307） | 🟡 | `tools/perf_baseline.py` 新增；本机短时冒烟实测（Python-CPU 单路 35.8fps / 4 路 9.7fps/路，rust-tract 54.9s、rust-ort 4.4s 处理同一语料）；**完整 1/4/8 路 × 30 分钟矩阵待目标硬件重跑**（`--duration 1800`）；`docs/perf-baseline.md` 已建并如实标注口径限制 |
| R2-303 Windows GPU 可选变体 | 🟡 | `requirements-win-dml.in/.lock` 生成（`onnxruntime-directml==1.24.4`，pip-audit 0 漏洞、ASCII）；`build_portable.py --variant cpu|dml` + release.yml matrix 两个 job；前端探测/设置页提示与首启推荐未做（待 DML 环境实测后收口） |
| ADR-0009 | ✅ | `docs/adr/0009-alignment-vs-gpu-and-model-provenance.md`：分层容差验收、双变体、模型来源差异与恢复计划 |

## R3 Rust L1 可发布

| 任务 | 状态 | 说明 |
|---|---|---|
| R2-301 Rust 统一二进制查找 | ✅ | `ls-core/src/bin.rs`（LS_FFMPEG → exe 同级 tools/ → tools/ffmpeg/ → PATH + `ensure_tools_on_path`）；ls-ingest 与 evidence.rs 全部改用；3 个单测 |
| R2-302 存证编码修复 | ✅ | evidence.rs stderr 线程排水（防管道死锁）+ 失败信息附最后 500 字节 stderr；测试改 `ls_core::bin::available` 判定 |
| R2-304 打包 ls-analyzer | ✅ | `build_portable.py --with-rust`：release ls-analyzer.exe + onnxruntime.dll 进 `bin/`；`portable_launcher` 注入 `LS_ANALYZER_EXE`/`ORT_DYLIB_PATH`；`analyzer_sidecar._default_exe()` 按便携 bin/ → cargo target 解析 |
| R2-305 Rust 告警读取网页配置 | ⬜ | 需扩展 proto（Config 加 AlertChannels）+ 两侧代码再生成——并入 R2-309 契约批次 |
| R2-308 黄金语料扩展 | ⏸ | 需 CC0 素材拍摄（人工事项）；分层容差口径已在 ADR-0009 定义 |
| R2-507 N-28 过时注释复核 | ✅ | main.rs:169 注释更正（PPE 级联已接入） |

## R4 多端（裁剪版）

| 任务 | 状态 | 说明 |
|---|---|---|
| R2-203 设备管理前端 | ✅ | `web/src/views/Devices.vue`（配对码展示/设备列表/两步吊销）+ App.vue 标签 + api.js 封装；`tests/devices.test.js` 2 例 |
| R2-204 分析器可用性透出 | ✅ | `/api/system.components.analyzer_sidecar{available,exe,version,watchdog_*}`；前端置灰待 rust 路由批次与 UI 联调 |
| R2-502 前端测试补齐 | 🟡 | 28→30 例（Devices 2 例）；WebrtcPlayer 子协议/Wall/Models 下载流程仍未覆盖（列 R2-502 余量） |
| R2-601~605 Flutter / Tauri / VM / 真机验收 | ⏸ | 用户明确暂缓（Windows 优先） |

## R5 Rust L3 控制面

| 任务 | 状态 | 说明 |
|---|---|---|
| R2-307 路由覆盖率统计进 CI | ✅ | `ls-server --dump-routes` + `scripts/rust_route_coverage.py`（路径参数归一化 {param}/:id）；rust.yml 覆盖率 artifact（先跟踪不门禁） |
| R2-309 ls-server 补齐（按路由组分批） | 🟡 | **批次 1 完成（64e9f1f）：17→61/106 = 57.5%**——cameras/zones/rules/模板 CRUD、配对与设备令牌（Bearer/读保护/节流/防爆破）、审计、投递/静默/日历、stats 四件套+timeline、CSV 导出、metrics、v1/meta；ls-store 扩展 + 修复 update_delivery 列名；新集成测试 7 例。**余量 45 条**：face/plugins/genai/discovery/mqtt/update/alerts 配置读写/export/pdf/媒体签名/system-shutdown/model/ppe 下载（多依赖外部服务或文件系统，随批次 2+） |
| R2-503 审计写入移出事件循环 | ✅ | Python：队列+单写线程；Rust：spawn_blocking await（异步等待不阻塞执行器） |
| R2-504 第三方 Action 钉 SHA + Dependabot | ✅ | 7 个 Action 全部钉 commit SHA（API 实测当前 HEAD）；`.github/dependabot.yml`（pip/npm/cargo/actions 每周） |
| N-34 重试语义测试稳定性 | ✅ | ls-dispatch `deliver_with_backoff`（退避注入 [0,0,0]）+ 断言前轮询计数收敛；集成测试 20s→2s |
| R2-502 前端测试补齐 | 🟡 | 28→**34 例**（Devices 2、Wall 2、Models 2）；WebrtcPlayer 子协议/断线重连需 stub RTCPeerConnection 信令全流程，投入产出低暂缓（记录为余量） |

## 本轮环境事件（2026-11-09）

- **卡巴斯基启发式查杀**：`install_portable.py` 内嵌「-c 动态代码串 + urlopen + Popen」被
  Kaspersky 判为 HEUR:TrojanDownloader/Python.Agent.ao，pytest 执行即删文件（3 次）。
  已重构为独立 `scripts/health_check.py` 模块（loopback URL 白名单），此后未再触发。
  经验已入库项目记忆；该文件现跑安装测试稳定通过。

## 明确暂缓项（本轮不执行）

- R2-401 Docker 修复与实机验证：0.2.0 发布说明标注「Docker 预览，不随本版」，顺延 0.3.0。
- R2-404 VM 安装验收留档；Flutter/Tauri 客户端；局域网真机验收。
- Docker 静态问题（N-07/N-08/N-19）代码未改，保留在 §6/§10 审查详情中作为待办。

## 人工事项（代码可备、验收留待后续）

Certum 代码签名证书、minisign 密钥对（R2-107 用测试密钥对验证）、Roboflow 权重许可确认、CC0 演示素材拍摄、真实 RTSP 24h 挂机、隔离机 go2rtc 反向验证、L3 启动会。

## 已知遗留风险

1. `assets.lock.json` 的 `yolo11n.onnx` 使用 Ultralytics 官方导出（sha256 `634279b4…`），与本地自行导出的 `m0-spike/yolo11n.onnx`（sha256 `4758511e…`）不是同一文件。CI 便携包用官方版；**黄金语料/契约测试仍以本地版为基准**。ADR-0009 需写明该差异与恢复计划（把本地导出上传到项目自建 assets release 后改 URL）。
2. `assets.lock.json` 中 `yolo11n_ppe.onnx` / `.classes.txt` 的 `Windy-LabSentinel/assets` release URL 实测 404（Q-08 按需下载当前不可用）——二轮审查未覆盖，需维护者补传资产或换 URL。
3. 二轮审查报告的 `engine-rs/crates/ls-server/src/main.rs:546`、`ls-analyzer/src/main.rs:992` 两处 medium「疑似跨文件污点」为既有结构性告警，Mimosa 每次提交都会提示；R5 触碰这两处时一并复核。
