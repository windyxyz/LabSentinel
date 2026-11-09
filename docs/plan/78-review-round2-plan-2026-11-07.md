# LabSentinel 第二轮全面审查与推进计划（2026-11-07）

> 本轮审查方式与首轮相同：**只读**。代码、配置、依赖锁、脚本都没有改动；唯一写入 = 本文件。
> 测试、静态分析、构建都在 `git archive HEAD` 导出的**临时副本**中执行，不碰工作区。
>
>基线：`D:\111`，分支 `main`，HEAD `499348b`（2026-11-07），工作区干净，共 112 次提交、397 个入库文件。
> 级别：**P0** 阻断/高危 · **P1** 高 · **P2** 中 · **P3** 低。"待验证"表示有代码证据但未实机复现。

---

## 目录
1. 本轮结论摘要
2. 项目现状概览（与首轮相比的变化）
3. 审查范围与方法
4. 首轮任务完成度核对（逐项：已完成 / 部分 / 未开始 / 声明与实际不符）
5. 新发现问题总览表（N-xx）
6. 安全审查详情
7. Bug 审查详情
8. 功能与多端审查详情
9. 性能与 Rust 迁移（S7）审查详情
10. 发布与安装工程审查详情
11. 推进策略：阶段、优先级、依赖
12. 精细化任务清单（R2-xxx）
13. 子代理执行拆分
14. 测试与验收标准（更新）
15. 发布检查清单（0.2.0 版）
16. 待确认问题与风险
17. 附录：命令、证据、代码片段

---

## 1. 本轮结论摘要

**进度**：首轮 101 项任务里，S0–S5（基线、安全、缺陷、性能止血、发布工程、可维护性）**大部分已落地且质量不错**：
go2rtc 收回环并加鉴权、摄像头 source 白名单、首启令牌、Host/CSRF/WS 同源、密钥加密、签名媒体 URL、
PPE 单次推理、存证内存 190MB→8MB/路、锁文件构建、版本单源、`api.py` 拆分（2000 行 → 990 行 + 8 个 router）等都已核实存在。
Rust 工作区 `engine-rs/`（10 个 crate）已建立，ByteTrack 与检测层做到了黄金语料逐帧/逐位对齐，这是本轮最大的进展。

**测试现状**：Python 587 通过 / 6 失败（首轮为 17 失败）；前端 28/28 通过；Rust 69 通过 / 2 失败，clippy、fmt 通过；pip-audit 与生产 npm 依赖均无已知漏洞。
其中 3 个 Python 失败来自"缺 Rust 产物时用 assert 而不是 skip"，CI 的 pytest 因此必红（N-36）。

**但有 4 类问题会直接阻断 0.2.0 发布**：
1. **锁文件漏依赖**：`grpcio`、`cryptography` 不在任何锁文件里（实测：按锁安装后 `import grpc`/`import cryptography` 都报 `ModuleNotFoundError`）。
   结果：Rust 分析器灰度路径不可用；Linux/Docker 上告警密钥**静默降级为明文**；更新清单验签不可用。
2. **部署形态不可用**：Docker 镜像里 uvicorn 绑 `127.0.0.1`，端口映射进不来；`compose.yaml` 同时写了 `network_mode` 和 `networks`（Compose 拒绝启动）；
   `Dockerfile.rust` 的产物路径写错；`rust-toolchain.toml` 的写法被当前 rustup（1.29）拒绝（实测报错），Rust CI 与 Docker Rust 构建都会失败。
3. **发布流水线仍然跑不通**：`build_portable.py` 依然要求本机 `.venv`，人体模型 `yolo11n.onnx` 仍被 gitignore 且流水线没有下载步骤；"干净树检查"是 `|| true` 空操作；签名仍是占位。
4. **安全链路的新缺口**：
   - 安全判断依据的是环境变量 `LS_HOST`，而不是 uvicorn 实际监听的地址；用户直接 `--host 0.0.0.0` 就绕过了"局域网必须有令牌、强制读保护"。
   - 校验 FAIL 并不阻止启动：只有 `LS_STRICT_CONFIG=1` 时才拒绝启动。
   - WebRTC 信令代理不回显子协议：开了令牌（现在默认就开）后，浏览器实时画面握手失败。
   - 配对设备令牌在读保护下不被接受：手机局域网场景整体不可用。

**性能方面要特别指出**：T-328 方案 A 为了让 Python 与 Rust 检测"逐位一致"，把 Windows 推理从 **DirectML（GPU）改成了 CPU**；Rust 默认后端 tract 也是纯 CPU。
对齐目标达成了，但产品在 Windows 上失去了 GPU 加速，多路吞吐会明显下降（N-20）。这需要用"容差对齐"替代"逐位对齐"，把 GPU 加回来。

**Rust 迁移的真实进度**：数据面 L1 基本可用（未打包分发）；控制面 `ls-server` 只实现了 **17 条路由，对比 OpenAPI 的 97 条路径**，覆盖约 18%。
CHANGELOG 和 `Dockerfile.rust` 注释中"L3/L4 已替换 Python 运行时"的说法**与实际不符**（N-27），需要更正口径并重新排期。

**多端**：服务端跨平台抽象层（Python `app/platform/*`）、设备配对接口、`/api/v1`、值班大屏都已完成；
但 Flutter 移动端**完全没有进展**（仍是骨架、无令牌、无平台目录），Tauri 桌面端也未开始。

---

## 2. 项目现状概览（与首轮相比的变化）

| 维度 | 首轮（2026-11-04） | 现在（2026-11-07） |
|---|---|---|
| 版本 | `0.1.0-alpha` / `2026.10` 混用 | `VERSION=0.2.0` 单源（`server/app/version.py` 读取） |
| 服务端结构 | `api.py` 约 2000 行 | `api.py` 990 行 + `routers/{cameras,config,configio,events,folders,models,ops,stats}.py` + `create_app()` 工厂 |
| 新模块 | — | `sources.py secretstore.py devices.py configcache.py webrtc_proxy.py analyzer.py analyzer_sidecar.py sidecar_events.py auth_setup.py csvsafe.py version.py platform/*` |
| 依赖 | 无锁 | `requirements.in` + win/linux/cuda 三份带哈希锁（**但 `requirements.txt` 仍保留旧口径，见 N-04**） |
| 推理（Windows） | onnxruntime-directml 1.24.4（GPU） | onnxruntime 1.31.1 **CPU**（T-328 方案 A） |
| Rust | 无 | `engine-rs/`：ls-core/track/proto/analyzer/ingest/infer/store/dispatch/jobs/server，约 9,400 行 |
| 契约 | 无 | `proto/analyzer.proto`、`testdata/vectors/*.json`（Python 与 Rust 共用）、黄金语料验证记录 |
| 部署 | 仅 Windows 便携 zip | + `deploy/docker/{Dockerfile,Dockerfile.cuda,Dockerfile.rust,compose.yaml}`、systemd unit、WinSW 服务 XML |
| CI | 重复的 ci/tests | `tests.yml`（win+linux 矩阵）、`rust.yml`、`release.yml`、`security-weekly.yml` |
| 文档 | 193 个 | 50 个（`499348b` 删除了大量历史报告与任务包） |
| 测试 | 511 个（17 失败） | Python：见 §14.1；Web：28 个全过；Rust：见 §14.1 |

---

## 3. 审查范围与方法

### 3.1 已检查
- 逐项核对 CHANGELOG 中声明完成的任务对应的代码（安全链路、依赖锁、部署、发布、Rust 控制面路由、分析器打包等），重点读了：
  `main.py api.py(鉴权/WS 段) go2rtc.py sources.py webrtc_proxy.py devices.py secretstore.py update_check.py validate.py analyzer_sidecar.py engine.py(后端注册表)`、
  `engine-rs/crates/{ls-server,ls-analyzer,ls-infer,ls-ingest,ls-dispatch}`、`deploy/docker/*`、`.github/workflows/*`、`scripts/{build_portable,build_rust,install_portable}.py`、
  `server/requirements*`、`rust-toolchain.toml`、`android/lib/api/client.dart`。
- 动态检查（临时副本）：按 `requirements-win.lock` 全新安装 + `pip check` + 关键模块导入；pytest 全量；vitest + 前端构建；npm audit；cargo test / clippy / fmt。

### 3.2 未覆盖 / 局限
- 未真实启动 Docker（本机无 Docker 引擎），Docker 结论来自配置文件静态分析与 Compose 规范，标"待验证"项会给出复现命令。
- 未连接真实摄像头/go2rtc；浏览器子协议握手失败的结论来自 RFC 6455 §4.1 与代码对照，**待实机验证**（R2-104 给出了验证步骤）。
- Python 依赖 CVE：`pip-audit` 结果见 §14.1。
- 未逐行审 `pipeline.py`（849 行）与 `store.py`（1105 行）的全部新增逻辑，只做了与新特性相关的抽查。

---

## 4. 首轮任务完成度核对

> 状态：✅ 已完成且核实 · 🟡 部分完成/有缺陷 · ⬜ 未开始 · ❌ 声明完成但核实不符。"本轮编号"指向 §5 的新问题。

### S0 基线
| 任务 | 状态 | 核实说明 | 本轮编号 |
|---|---|---|---|
| T-001 requirements ASCII + reportlab | 🟡 | `requirements.txt` 已是 ASCII 并含 reportlab；但新的事实源 `requirements.in` 第 9-11 行含中文（与其文件头"Keep this file pure ASCII"矛盾）；且 `.txt` 与 `.in` 内容已分叉 | N-04 |
| T-002 锁文件 + 干净安装 | 🟡 | 三份锁存在、带哈希、opencv 已去重；**漏掉 grpcio、cryptography** | N-01 |
| T-003 测试全绿 | 🟡 | 实测 587 通过 / 6 失败（首轮 17 失败）；其中 3 个因缺 Rust 产物而 assert 失败，导致 CI 必红 | N-35、N-36、N-38 |
| T-004 CI 合并 | 🟡 | `tests.yml` 双平台、锁安装；但 `rust.yml`/`release.yml` 里 `dtolnay/rust-toolchain@stable`、`Swatinem/rust-cache@v2`、`taiki-e/*`、`upload-artifact@v4`、`softprops/action-gh-release@v2` 未钉 SHA | N-33 |
| T-005 下载哈希 | ✅ | `scripts/assets.lock.json` 含 sha256；镜像代理默认关闭 | — |
| T-006 前端 dev 依赖 | ✅ | vite 8 / vitest 5；npm audit 生产 0，开发 5 个 moderate（glob/minimatch 链） | — |
| T-007/T-008 | 🟡 | `.editorconfig/.gitattributes/CHANGELOG` 已有；仓库 `.venv` 仍然损坏（指向不存在的 `C:\Users\User\...\Python312`），说明自愈只在跑 `deploy.py install` 时生效 | — |

### S1 安全
| 任务 | 状态 | 核实说明 | 本轮编号 |
|---|---|---|---|
| T-101 go2rtc 回环+鉴权 | ✅ | `go2rtc.py:29-75` 绑 `127.0.0.1:1984`、随机 Basic 凭据、去 `origin`、关闭 RTSP 再分发 | （Docker 下失效：N-08） |
| T-102 WebRTC 信令代理 | ❌ | 代理已实现，但 `main.py:339` `await ws.accept()` 未回显子协议，而前端 `WebrtcPlayer.vue:164` 有令牌时会发送子协议 → 浏览器按 RFC 6455 断开握手（待实机） | N-05 |
| T-103 source 校验 | ✅ | `sources.py` 白名单 + `yaml_stream_entry` JSON 转义 | — |
| T-104 首启令牌 | 🟡 | 已生成令牌；但"非回环无令牌 FAIL"只是校验项，**非 strict 模式下不阻止启动** | N-03 |
| T-105 Host/CSRF | 🟡 | 已实现；但 `testserver`（测试专用主机名）被无条件加入生产白名单 `main.py:447-448` | N-14 |
| T-106 WS 同源/子协议 | ✅ | `/ws/events` 正确回显子协议 | — |
| T-107 密钥加密 | 🟡 | Windows DPAPI 可用；POSIX 依赖 `cryptography`（未入锁）→ **静默明文**；Rust 侧无法读取加密配置 | N-01、N-12 |
| T-108 签名媒体 URL | ✅ | `api.py:136-191`；`?token=` 兼容开关保留 | — |
| T-109 脱敏 | ✅（按 CHANGELOG，未逐条复测） | — | — |
| T-110 更新签名 | 🟡 | 公钥 `update.pub` 不存在 → 验签跳过；验签代码与 minisign 格式不兼容（放入公钥后正确签名也会失败） | N-13 |
| T-111 文件夹允许根 | 🟡 | 黑名单只检查**父目录**，目录本身不检查：`C:\Windows`、`C:\Program Files` 本身可以通过 | N-11 |
| T-112 插件环境白名单 | ✅ | — | — |
| T-113 审计/500 安全头 | ✅ | `main.py:257-260` | — |

### S2 缺陷
| 任务 | 状态 | 说明 |
|---|---|---|
| T-201 WS 令牌/hub | ✅ | `/ws/events` OK |
| T-202 启动器托管 go2rtc | ✅（按 CHANGELOG；便携包未实机） | — |
| T-203~T-210 | ✅（按 CHANGELOG + 抽查 stats/backups） | — |
| B-28 /api/system 不加载模型 | ✅ | `api.py:42` 注释与 `engine.preload_async()` |

### S3 性能止血
| 任务 | 状态 | 说明 | 本轮编号 |
|---|---|---|---|
| T-301 PPE 单次推理 | ✅ | — | — |
| T-302/T-303/T-309 | ✅ | 线程本地连接、ConfigCache、v11 索引 | — |
| T-304 ORT 调优 | ✅ | — | — |
| T-305 告警限流 | ✅ | — | — |
| T-306 迟滞 | ✅ | — | — |
| T-307 压测基线 | ⬜ | 没有基线数据文档（`docs/perf-baseline.md` 不存在）；T-328 切 CPU 后性能影响无人测量 | N-20 |
| T-308 存证 JPEG | ✅ | — | — |
| T-310 AnalyzerBackend | ✅ | — | — |
| PF-08 审计写入阻塞事件循环 | ⬜ | `main.py:271` 仍在 async 中间件里同步写 SQLite | N-24 |

### S4 发布工程
| 任务 | 状态 | 说明 | 本轮编号 |
|---|---|---|---|
| T-401 版本单源 | ✅ | — | — |
| T-402 发布流水线 | ❌ | CI 上不可能成功：`build_portable.py:54-58` 要求 `.venv`；`m0-spike/yolo11n.onnx` 被 gitignore 且无下载步骤；`build_rust.py` 写死 `D:/tmp/...` | N-09 |
| T-403 签名 | ⬜ | 占位（等证书/密钥：人工事项） | — |
| T-404 升级回滚 | 🟡 | 回滚机制存在，但默认健康检查是 `python -c "sys.exit(0)"`（`install_portable.py:338-341`），不会因服务起不来而触发回滚 | N-18 |
| T-405 安全卸载 | ✅ | 安装标记 + 拒绝非受管目录 + 逐项锚点删除 | — |
| T-406 许可证/素材 | ✅ | Mixkit 素材移除、PPE 权重按需下载 | — |
| T-407/T-408 | 🟡 | 脚本存在；签名密钥未到位 | — |
| T-409 release_check | 🟡 | 存在；但 `release.yml:20` 的干净树检查 `|| true` 等于没检查 | N-10 |
| T-410 CI 冒烟 | 🟡 | 写在 release.yml 里，但前序步骤必然失败，从未真正执行 | N-09 |

### S5 可维护性
| 任务 | 状态 | 说明 |
|---|---|---|
| T-501/T-502/T-503/T-506/T-507/T-509/T-510 | ✅ | — |
| T-504 统一日志 | 🟡 | 用 stdout tee 兜底，`print` 仍大量存在（可接受，但不是结构化日志） |
| T-505 拆分 api.py + 请求模型 | 🟡 | 拆出 8 个 router；`api.py` 仍有 990 行（鉴权、WS、face、alerts 等仍在里面） |
| T-508 create_app | ✅ | 但 `main.py:46-63` 导入期仍执行日志挂载与配置校验（副作用未完全收敛） |

### S6 多端
| 任务 | 状态 | 说明 | 本轮编号 |
|---|---|---|---|
| T-601~T-606 平台抽象 | ✅（Python 侧） | `app/platform/*`；Rust 侧 ffmpeg 仍只从 PATH 找 | N-22 |
| T-610 Docker/systemd | ❌ | 见 N-07/N-08（镜像不可达、compose 无法启动、go2rtc 配置不共享） | N-07、N-08 |
| T-611 WinSW | ✅（配置文件；未实机） | DPAPI 用户范围与 LocalService 账户不同 → 从便携版迁移到服务模式后密钥无法解密 | N-12 |
| T-612/T-613 | ⬜ | 按 77 号 Q-16 已取消 Python macOS 打包 | — |
| T-620 配对/三级令牌 | 🟡 | 服务端接口存在；**设备令牌不被读接口和 WS 接受** | N-02 |
| T-621 /api/v1 | ✅ | — | — |
| T-701 PWA | 🟡 | 未见 Lighthouse 报告 | — |
| T-702~T-706 移动端 | ⬜ | `android/` 仍是 12 个文件的骨架，无平台目录、无配对、无令牌 | N-26 |
| T-707/T-708 Tauri | ⬜ | — | — |
| T-709 值班大屏 | ✅ | `web/src/views/Wall.vue`（受 N-05 影响：开令牌后画面握手失败） | — |
| T-710 MQTT zones | ✅ | — | — |

### S7 Rust 迁移
| 任务 | 状态 | 说明 | 本轮编号 |
|---|---|---|---|
| T-320 契约 | ✅ | `proto/analyzer.proto` | — |
| T-321 黄金语料 | ✅ | 事件一致率 94.4%（18 事件合计）；大语料 track-agnostic 85%；检测框 IoU 58/58 ≥ 0.99 | N-21 |
| T-322 视频栈 ADR | ✅ | ADR-0006（ffmpeg 子进程） | — |
| T-323/T-324 core/track | ✅ | 322/322 帧逐帧一致 | — |
| T-325 推理 | 🟡 | 默认后端 tract（纯 CPU）；ort 后端需外部 `onnxruntime.dll`，未随包 | N-20 |
| T-326 ingest/存证 | 🟡 | 存证 MP4 测试在本机 ffmpeg 9.0.2 下失败（见 §14.1）；ffmpeg 只从 PATH 查找 | N-22、N-23 |
| T-327 sidecar | 🟡 | 可运行；但 `ls-analyzer.exe` 从未被打包，默认路径写死开发机 `D:/tmp/ls-rs-target/debug/ls-analyzer.exe`（`analyzer_sidecar.py:22`），且 grpcio 未入锁 | N-06 |
| T-328 默认切换 | ⬜ | 前提（打包）未满足 | N-06 |
| T-329 告警派发 | 🟡 | 8 渠道等价；但渠道只从环境变量读取，忽略网页端保存在 `app_settings` 的配置（任务 140 核心功能），也无法解密 `enc:v1:` | N-12 |
| T-330 后台作业 | 🟡 | retention/backup/escalation 已迁；genai/plugins/folder 编排推迟（有评估结论） | — |
| T-331 控制面 | ❌（声明"L3 完成"） | `ls-server` 只有 17 条路由 vs OpenAPI 97 条；监听地址写死 `127.0.0.1`、读保护写死 `false`、无首启令牌 | N-27 |
| T-332 DB 双向兼容 | ✅ | 三路径测试 | — |
| T-333 L4 移除 Python | ❌（声明完成） | Python 运行时仍是唯一可用的产品形态；`Dockerfile.rust` 注释"Python 运行时已移除"不成立 | N-27 |
| T-334 工具链边界 | ✅ | `tools/README-ENGINE-BOUNDARY.md` | — |
| T-335 Rust 基建 | ❌ | `rust-toolchain.toml` 被 rustup 1.29 拒绝（实测 `error: target tuple in channel name`），CI 与 Docker 构建都会失败 | N-15 |
| T-336 观测/回退 | ✅ | — | — |

**完成度统计（按首轮 101 项）**：✅ 约 58 项 · 🟡 约 22 项 · ⬜ 约 14 项 · ❌ 7 项（T-102、T-402、T-610、T-331、T-333、T-335 为主）。

---

## 5. 新发现问题总览表

| ID | 类型 | 级别 | 位置 | 摘要 | 证据 |
|---|---|---|---|---|---|
| N-01 | REL/SEC | **P0** | `server/requirements*.lock`、`requirements.in` | 锁文件缺 `grpcio`、`cryptography`：sidecar 不可用、POSIX 密钥明文、更新验签不可用 | 实测导入失败 |
| N-02 | FUNC/SEC | **P1** | `api.py:86-95,157-176,963-969` | 设备令牌只被 `require_triage` 接受；`require_read`/WS 不接受 → 读保护开启时（局域网强制开启）手机拿不到任何数据 | 代码 |
| N-03 | SEC | **P1** | `validate.py:21-42`、`main.py:59-63`、`api.py:66-73` | 安全判定依据 `os.environ["LS_HOST"]`，不是真实监听地址；FAIL 只在 strict 模式下阻断启动；`.env` 里的 `LS_HOST` 还读不到 | 代码 |
| N-04 | BUG | P2 | `requirements.in:9-11`、`requirements.txt` | `.in` 非 ASCII；`.txt` 仍是 `onnxruntime-directml>=1.17`，与锁分叉（`deploy.py` 源码部署装的是 `.txt`） | 实测编码检查 + diff |
| N-05 | BUG | **P1** | `main.py:332-346`、`web/src/components/WebrtcPlayer.vue:164` | WebRTC 信令 WS 不回显子协议 → 有令牌时浏览器断开握手，实时画面/大屏失效（待实机） | 代码 + RFC 6455 |
| N-06 | REL | **P1** | `analyzer_sidecar.py:22`、`scripts/build_portable.py` | `ls-analyzer.exe` 未打包，默认路径是开发机 debug 目录；用户在界面切到 rust 后实际会失败/回退 | 代码 grep |
| N-07 | REL | **P1** | `deploy/docker/compose.yaml:23-27` | go2rtc 同时声明 `network_mode: service:` 与 `networks:`（Compose 规范禁止组合，`docker compose up` 会报错）；引用的 `docker-compose.cuda.yaml` 不存在 | 配置（待验证） |
| N-08 | REL | **P1** | `Dockerfile:29,38`、`Dockerfile.cuda:30,40`、`Dockerfile.rust`、`ls-server/src/main.rs:505` | 容器内服务绑 `127.0.0.1` → `-p 8000:8000` 无法访问；go2rtc 容器读 `/config`，服务端把 yaml 和凭据写在 `/data`，两者不共享 → go2rtc 用默认配置运行、服务端 Basic 鉴权失败 | 配置 |
| N-09 | REL | **P1** | `release.yml:48-68`、`build_portable.py:54-58,122-125`、`build_rust.py:20-21,25` | 发布流水线必然失败：需要 `.venv`、需要未入库的 `yolo11n.onnx`、Rust 构建写死本机路径 | 代码 |
| N-10 | REL | P2 | `release.yml:20,24` | 干净树检查 `|| true` 是空操作；CHANGELOG 检查只认 `[Unreleased]`，不认本版本段 | 代码 |
| N-11 | SEC | P2 | `sources.py:72-74` | 系统目录黑名单只看父目录，`C:\Windows`、`C:\Program Files`、`/etc` 本身可通过；同时 `bin/library/system` 作为任意层级的父目录名会误杀正常目录（如 `D:\data\Library\shots`） | 代码 |
| N-12 | BUG | P2 | `secretstore.py:104-117`、`ls-dispatch/src/lib.rs:497`、`deploy/windows/labsentinel-service.xml` | 加密降级明文只打一条日志；Rust 告警只读环境变量、读不到网页配置也解不了 `enc:v1:`；DPAPI 用户范围在服务账户下不可解 | 代码 |
| N-13 | BUG | P2 | `update_check.py:80-118` | 验签取 `.minisig` 最后一行（全局签名）当签名，且未按 minisign 的 `ED`（BLAKE2b 预哈希）格式处理 → 放入公钥后合法清单也会被拒；`cryptography` 未入锁 | 代码 |
| N-14 | SEC | P3 | `main.py:447-448` | 生产 Host 白名单无条件包含 `testserver` | 代码 |
| N-15 | REL | **P1** | `engine-rs/rust-toolchain.toml` | `channel = "stable-x86_64-pc-windows-gnu"` 被 rustup 1.29 拒绝；Linux CI 与 Docker 构建也会受影响 | 实测报错 |
| N-16 | SEC | P3 | `analyzer_sidecar.py:89-90` | sidecar 一次性令牌通过命令行参数传递，本机其他用户可从进程列表读到 | 代码 |
| N-17 | SEC | P3 | `devices.py:27-46`、`api.py` 配对接口 | 6 位配对码只靠全局写限流（120/分钟）防爆破；无失败次数上限 | 代码 |
| N-18 | REL | P2 | `install_portable.py:336-349` | 升级后的默认健康检查只运行 `sys.exit(0)`，不启动服务 → 自动回滚形同虚设 | 代码 |
| N-19 | REL | P2 | `Dockerfile:28` | `LS_BACKEND_ORDER=openvino,cpu`，但锁里没有 `onnxruntime-openvino` → 静默 CPU | 锁文件 |
| N-20 | PERF | **P1** | `requirements.in:9-12`、`engine.py:41-59`、`ls-infer/src/yolo.rs:31-48` | Windows 推理被改为 CPU（DirectML 移除），Rust 默认 tract（纯 CPU）；无性能基线（T-307 未做） | 代码 + CHANGELOG |
| N-21 | PERF/QA | P2 | `docs/验证记录-S7-L1黄金语料对齐.md` | 黄金语料共 3 段 + 大语料，规模小于 76 号 §T-321 要求的 ≥10 段场景（遮挡/夜间/断流/文件夹等未覆盖） | 文档 |
| N-22 | BUG | P2 | `ls-ingest/src/lib.rs:76,201,214`、`ls-analyzer/src/evidence.rs:204` | Rust 侧 ffmpeg 只从 PATH 找，便携包把 ffmpeg 放在 `tools/` 不在 PATH → 取流/存证失败 | 代码 |
| N-23 | BUG | P2 | `ls-analyzer/src/evidence.rs:183-234,329-346` | 存证编码测试在 ffmpeg 9.0.2 下失败；stderr 被 pipe 却不读（大量输出时可能死锁）；失败信息不带 stderr | 实测 |
| N-24 | PERF | P2 | `main.py:262-274,279-296` | 审计写入仍在异步中间件里同步打开 SQLite | 代码 |
| N-25 | QA | P2 | `web/tests/*` | 前端只有 3 个测试文件，WebrtcPlayer 子协议、Wall、Models 下载流程、mediaUrl 签名都无测试 | 实测 |
| N-26 | FUNC | P1 | `android/` | 移动端零进展；服务端配对接口无人消费 | git ls-files |
| N-27 | DOC/ARCH | **P1** | `CHANGELOG.md:199-207`、`Dockerfile.rust:1`、`ls-server/src/main.rs:44-65` | "L3 契约测试 100% / L4 移除 Python"与事实不符：控制面 17/97 路由 | 代码计数 |
| N-28 | DOC | P3 | `ls-analyzer/src/main.rs:169` | 注释称"PPE 分类尚未接入"，但同文件 `:195-368` 已有 `YoloPpeRunner`——注释过时，删除即可 | 代码 |
| N-34 | QA | P3 | `ls-dispatch/tests/dispatch_integration.rs:123-137` | 重试语义测试不稳定：断言桩请求数时桩线程可能尚未记录完最后一次请求；且测试真实等待退避（20s） | 实测 |
| N-35 | BUG | P2 | `server/app/diagnostics.py:21-24` | B-28 修复不完整：`/api/diagnostics/summary` 仍调用 `get_engine()`，模型缺失时 500（实测堆栈 `no backend available ... yolo11n.onnx ... File doesn't exist`） | 实测 |
| N-36 | QA | **P1** | `server/tests/test_store_bidirectional.py:27` | 三个跨语言用例在 `labsentinel-server.exe` 不存在时用 `assert` 失败而不是 skip；`tests.yml` 不构建 Rust → **CI pytest 必红** | 实测 |
| N-37 | BUG | P2 | `server/app/platform/process.py:26-32,62-68` | PowerShell 输出按系统 ANSI 代码页（中文系统为 GBK），而便携启动器设置 `PYTHONUTF8=1`，`text=True` 按 UTF-8 严格解码 → 读取线程 `UnicodeDecodeError`（测试中出现 10+ 次）→ `is_our_process` 返回 False → 停止/卸载拒绝杀进程或误判陈旧锁。首轮代码曾用 `encoding="mbcs"` 规避，平台层重构后回归 | 实测 + 代码 |
| N-38 | QA | P3 | `server/tests/test_web_static.py:114-125`、`test_health_errors.py` | gzip 用例在 httpx 自动解压后再 `gzip.decompress`（`BadGzipFile`），断言末尾 `or True` 无意义；就绪探针用例依赖仓库外模型文件 | 实测 |
| N-39 | BUG | P2 | `server/app/backups.py:74-79` | 测试中自动备份子进程 `backup_data.py exit 1` 且 stderr 为空（`text=True` 无 `errors=`，同 N-37 解码问题）——失败原因不可见 | 实测 |
| N-29 | REL | P3 | `build_rust.py`、`analyzer_sidecar.py`、`verify_sidecar_evidence.py` | 多处写死 `D:/tmp/ls-rs-target`、`D:/tmp/w64devkit` 等本机路径 | 代码 |
| N-30 | DOC | P2 | 10 个文件引用"76/77 号文档" | 计划与决策文档不在仓库，后续执行者无法追溯；`499348b` 一次删除 187 个文档也降低了可追溯性 | grep |
| N-31 | REL | P3 | `ls-server` | 无首启令牌、读保护写死、Host 白名单写死——作为"可发布的第二产物"（release.yml 第 66-68 行）不安全 | 代码 |
| N-32 | QA | P2 | `rust.yml` | CI 不带 `--all-features`：`ls-infer` 的 tract/ort 路径不在 CI 默认测试中（ls-analyzer 依赖时会间接编译 tract+ort） | 配置 |
| N-33 | SEC | P3 | `rust.yml`、`release.yml`、`security-weekly.yml` | 部分第三方 Action 未钉 SHA（T-004 声明已钉） | 配置 |

---

## 6. 安全审查详情

### N-03（P1）安全判定与真实监听地址脱钩
**证据**：
- `validate.py:21` `host = os.environ.get("LS_HOST", "127.0.0.1")`；`api.py:72` `_auth_off()` 同样读 `os.environ`。
- 真正决定监听地址的是 uvicorn 的 `--host` 参数（`portable_launcher.py`、`deploy/*` 里都写死 `127.0.0.1`），**与 `LS_HOST` 无关**。
- `main.py:59-63`：FAIL 级校验只有 `settings.strict_config=True` 时才 `raise`。
- pydantic-settings 会读取 `.env` 文件，但 `os.environ` 不会，因此写在 `server/.env` 的 `LS_HOST` 对这两处判断不可见。

**攻击/误用场景**：
1. 用户按网上教程 `uvicorn app.main:app --host 0.0.0.0` 想让手机访问，`LS_HOST` 没设 → 系统认为是回环 → 不强制读保护；若再设了 `LS_AUTH_DISABLED=1` → 局域网内任何人可写。
2. 用户在 `.env` 写 `LS_HOST=0.0.0.0` 且无令牌 → validate 读不到，不报 FAIL。

**修复方向**：
- 把监听地址变成**单一来源**：`settings.host`（默认 `127.0.0.1`），启动器/Dockerfile/systemd/WinSW 一律 `--host "$LS_HOST"` 从同一配置取；
- 运行时以**实际绑定地址为准**：在 lifespan 或首个请求里读 `request.scope["server"]`（uvicorn 填充的实际 `(host, port)`），非回环且安全前提不满足时立刻停止服务（不是警告）；
- 安全类 FAIL（无令牌却非回环、逃生口用于非回环）**无论 strict 与否都拒绝启动**。→ R2-102。

### N-02（P1）设备令牌不能读
`require_read`（`api.py:157-176`）与 `_read_token_ok`（`api.py:86-95`）只认管理/只读令牌；`_ws_auth_check`（`api.py:963-969`）同理。
77 号 Q-11 的口径是"设备令牌 = 查看 + 处置"。局域网绑定会强制 `auth_read_protected=True`（`validate.py:32-33`），所以**配对后的手机在唯一需要它的场景里拿不到任何数据**。→ R2-103。

### N-05（P1）WebRTC 信令子协议
RFC 6455 §4.1：客户端请求了 `Sec-WebSocket-Protocol`，而服务器响应不带该头时，客户端**必须**断开连接（Chrome/Firefox/Safari 均如此实现）。
`/ws/events` 已正确处理（`api.py:980-982`），`/ws/webrtc/{cid}`（`main.py:339`）没有。首启令牌（Q-02）已默认开启 → 默认配置下实时画面与 `/wall` 大屏失效。→ R2-104。

### N-11（P2）文件夹黑名单绕过与误杀
`sources.py:72`：`parts_lower = {q.name.lower() for q in real.parents}`，不含 `real` 自身。
- 绕过：`C:\Windows`（parents 只有 `C:\`，name 为空）、`C:\ProgramData`、`/etc` 直接通过。
- 误杀：任意父目录叫 `bin`/`library`/`system`/`etc`/`usr` 就拒绝，例如 `D:\实验室\Library\截图`、macOS 用户目录下 `~/Library/...`。
**修复**：改为"真实路径是否位于具体系统根之下"（`%WINDIR%`、`%ProgramFiles%`、`%ProgramFiles(x86)%`、`%ProgramData%`、`/etc`、`/usr`、`/bin`、`/sbin`、`/System`、`/Library`（仅根级）），包含目录本身。→ R2-105。

### N-12（P2）密钥存储的三个缺口
1. `cryptography` 未入锁 → POSIX `_fernet()` 返回 None → `protect()` 返回明文且只打 WARNING（`secretstore.py:114-116`）。应改为：加密不可用时**拒绝保存**并提示。
2. Rust `ls-dispatch` 只从环境变量取渠道（`lib.rs:497` 起），网页端配置的渠道在 Rust 路径完全不生效。
3. DPAPI 用户范围：WinSW 服务以 `LocalService` 运行（`deploy/windows/labsentinel-service.xml`），与安装用户不同 → 迁移后密文无法解密（`unprotect` 返回空串，渠道静默失效）。
→ R2-106、R2-305。

### N-13（P2）更新验签与 minisign 不兼容
minisign `.minisig` 格式：第 1 行注释；**第 2 行** base64 = `签名算法(2B: "Ed" 或 "ED") + key_id(8B) + 签名(64B)`；第 3 行可信注释；第 4 行全局签名。
`ED`（默认，minisign ≥0.10）对文件内容先做 BLAKE2b-512 再签名。现代码取最后一行并直接 `verify(sig, body)`，两点都不对。
**修复片段**：
```python
lines = r.text.strip().splitlines()
blob = base64.b64decode(lines[1])
alg, key_id, sig = blob[:2], blob[2:10], blob[10:74]
if key_id != pub_key_id:
    raise UpdateCheckError("签名公钥不匹配")
msg = hashlib.blake2b(body, digest_size=64).digest() if alg == b"ED" else body
key.verify(sig, msg)
# 可选：再校验第 4 行全局签名 = sign(sig + trusted_comment_bytes)
```
公钥同样要解析 `alg(2B)+key_id(8B)+pk(32B)`，并比对 key_id。→ R2-107。

### N-14、N-16、N-17、N-33（P3）
- N-14：`testserver` 仅在 `PYTEST_CURRENT_TEST` 或显式 `LS_TESTING=1` 时加入。
- N-16：令牌改为通过环境变量或 stdin 传给 sidecar（Windows 上环境变量对同用户可见但不对其他用户可见）。
- N-17：配对码连续 5 次失败作废当前码、`claim` 接口单独限流 10 次/分钟/IP。
- N-33：所有第三方 Action 钉 commit SHA，Dependabot 维护。

---

## 7. Bug 审查详情

### N-01（P0）锁文件缺依赖——最先修
**实测**（临时副本，Python 3.13 venv）：
```
pip install --require-hashes --no-deps -r server/requirements-win.lock   → exit 0
python -c "import grpc"           → ModuleNotFoundError: No module named 'grpc'
python -c "import cryptography"   → ModuleNotFoundError: No module named 'cryptography'
```
本地开发 `.venv` 里有 grpc（`.venv/Lib/site-packages/grpc`），说明是手工装的，未回写到 `requirements.in`。
**影响**：`analyzer_sidecar.py:114` `import grpc` 失败 → `analyzer=rust` 路径永远回退；`secretstore`（POSIX）、`update_check`（验签）失效。
**修复**：`requirements.in` 增加 `grpcio==<与 analyzer_pb2 生成版本匹配>`、`protobuf`（已在锁里，确认兼容）、`cryptography>=43`；重新生成三份锁；
`build_portable.smoke()` 与 CI 增加"`requirements.in` 中每个顶层包都能 import"的断言（自动从 `.in` 解析）。→ R2-001。

### N-04（P2）依赖事实源分叉
`requirements.txt` 仍写 `onnxruntime-directml>=1.17`，`.in`/锁写 `onnxruntime==1.31.1`；`deploy.py:262` 源码部署装 `.txt`。
**修复**：`requirements.txt` 改为"`-r requirements.in` 一行 + 注释"或直接删除并更新 README/deploy；`.in` 改英文注释。→ R2-002。

### N-15（P1）Rust 工具链文件
本机实测：`cargo test` → `error: target tuple in channel name 'stable-x86_64-pc-windows-gnu'`。rustup 新版本要求 `channel` 只写版本，目标写到 `targets`。
**修复**：
```toml
[toolchain]
channel = "1.96.0"          # 钉具体版本（可复现）
components = ["clippy", "rustfmt"]
targets = ["x86_64-pc-windows-msvc", "x86_64-pc-windows-gnu", "x86_64-unknown-linux-gnu"]
```
本机 GNU 配方改为在 `build_rust.py` 里显式 `cargo +stable-x86_64-pc-windows-gnu` 调用。→ R2-003。

### N-22 / N-23（P2）Rust 侧 ffmpeg
- 查找：Rust 统一用 `ls_core::bin::find("ffmpeg")`：`LS_FFMPEG` → `<exe 同级>/tools/<os>-<arch>/ffmpeg[.exe]` → `<exe>/../tools/ffmpeg[.exe]` → PATH（与 Python `platform/binaries.py` 同口径）。
- 存证编码：失败时读取 stderr 并附在错误里；`stderr` 用线程排水，或改为 `Stdio::null()` + 失败时用 `-v error` 重跑取日志；对 ffmpeg 9.x 复测 `libx264`/`mpeg4` 参数（失败原因见 §14.1 实测输出）。→ R2-301、R2-302。

### N-18（P2）升级健康检查
默认健康检查应真实启动新版本（`runtime\python.exe -m uvicorn ... --port <临时端口>`），轮询 `/api/health/ready` 60s，结束后停止；或直接调用 `portable_launcher.py --selftest`（新增：启动→就绪→停止，返回码）。→ R2-404。

### N-28（P3）
已复核：`main.rs:195-368` 存在 `YoloPpeRunner` 与 `make_ppe_runner()`，PPE 已接入；删除 `main.rs:169` 的过时注释。

### N-34（P3）不稳定的重试测试
修复：退避时长改为可注入（测试用 `[0ms,0ms,0ms]`），桩服务器在处理请求**之前**记录计数；断言前 `join` 或轮询等待计数稳定（最多 1s）。→ R2-505 一并处理。

---

## 8. 功能与多端审查详情

### 8.1 服务端多平台现状
- Python 平台层齐全；Linux 部署因 N-07/N-08 实际不可用；macOS 按 Q-16 等 Rust。
- `trusted_hosts` 默认只有回环；用户开放局域网时必须手动加 IP/主机名，否则 400。首启向导与 `validate.py` 应在 `LS_HOST` 非回环时检查 `trusted_hosts` 是否包含可达地址（R2-102 内）。

### 8.2 客户端
| 客户端 | 现状 | 本轮建议 |
|---|---|---|
| Web/PWA | 可用；`/wall` 已有 | 修 N-05；补前端测试（N-25） |
| Android/iOS（Flutter） | 零进展 | **1.1.0 首要任务**：先修服务端 N-02，再按 76 号 T-702~T-706 推进 |
| 桌面（Tauri） | 未开始 | 等 Rust 控制面达到可用度（见 §9.3 门槛）后再启动 |

### 8.3 功能缺口（新）
| ID | 级别 | 说明 | 任务 |
|---|---|---|---|
| F2-01 | P1 | 设备管理 UI：生成配对码（含二维码）、设备列表、吊销（服务端接口已有，前端未见对应页面——**待确认** `Cameras.vue`/设置页是否已有） | R2-203 |
| F2-02 | P2 | 分析器切换（python↔rust）UI 应在 sidecar 不可用时**禁用**并说明原因，而不是切过去再回退 | R2-204 |
| F2-03 | P2 | `/api/system` components 增加 `analyzer_sidecar: {available, exe, version, reason}` | R2-204 |
| F2-04 | P2 | GPU 后端可选安装（DirectML 包作为可选组件/第二种便携包） | R2-303 |

---

## 9. 性能与 Rust 迁移（S7）审查详情

### 9.1 N-20（P1）GPU 加速丢失
T-328 方案 A 的逻辑是"同一个 ORT 版本 → Python/Rust 检测逐位一致 → 黄金语料一致率达标"。这个目标本身合理，但代价是 Windows 默认推理从 DirectML 变成 CPU：
- 原 v0.1 的 M0 实测与架构都基于 DirectML（`docs/30-architecture.md` 推理锁机制、`engine.py` 注释）。
- 现在 Windows 锁里只有 `onnxruntime==1.31.1`（无 DmlExecutionProvider），`LS_BACKEND_ORDER=directml` 也只会自动降级到 CPU。
- Rust 默认 tract：纯 CPU 解释执行，通常明显慢于 ORT CPU（**待 R2-306 实测**）。

**建议的平衡方案**（替代"逐位一致"作为唯一标准）：
1. 对齐验收改为**分层容差**：检测框 IoU ≥ 0.99 且 |Δconf| ≤ 0.02（同后端）/ ≤ 0.05（跨后端）；事件一致率 ≥ 95%（track-agnostic）。这样 DML、CUDA、CoreML、OpenVINO 都能纳入。
2. Windows 便携包提供**两个推理变体**：默认 `cpu`（onnxruntime 1.31.1，与 Rust ort 同版本，用于对齐与兼容）+ 可选 `gpu-dml`（onnxruntime-directml 最新可用版本，作为第二个锁文件 `requirements-win-dml.lock`）；首启探测 DirectX 12 可用时推荐 GPU 变体。
3. Rust 侧：`ort` 后端设为默认（随包携带 `onnxruntime.dll`；DirectML 版本携带 `DirectML.dll` 并启用 DML EP），tract 只作为"无动态库环境"的兜底。
4. 先做 T-307 压测基线，用数据决定默认值。→ R2-303、R2-306。

### 9.2 Rust 迁移真实进度
| 层 | 进度 | 可发布性 |
|---|---|---|
| L0 契约/语料 | ✅ | — |
| L1 数据面（ingest→infer→track→rules→事件流→存证） | 功能完整度约 80%（PPE 待复核、ffmpeg 查找、存证编码问题） | ❌ 未打包、grpc 依赖缺失 |
| L2 后台作业 | retention/backup/escalation ✅；genai/plugins/folder 编排 ⬜ | 依附于 ls-server，同样不可发布 |
| L3 控制面 | 17/97 路由（约 18%）；鉴权/读保护/首启令牌/网页配置渠道未齐 | ❌ |
| L4 移除 Python | ⬜ | — |

### 9.3 下一阶段门槛（建议写入 ADR-0009）
- **L1 发布门槛**（1.2.0 "实验性 rust 分析器"）：ls-analyzer 随便携包分发 + grpcio 入锁 + ffmpeg 查找统一 + 存证测试通过 + 黄金语料扩展到 ≥10 段 + 压测显示 rust 路径在同硬件上吞吐 ≥ Python 的 1.5 倍（否则不推荐用户切换）。
- **L3 发布门槛**：`tests/contract` 对 Rust 实现覆盖 ≥ 95% 的 OpenAPI 路径；路由覆盖率由脚本自动统计并进 CI（R2-307）。
- 达到 L3 门槛前，`Dockerfile.rust` 与 `build_rust.py` 产物**不得**进入正式 Release（只作为 nightly/实验产物）。

---

## 10. 发布与安装工程审查详情

### 10.1 发布流水线断点（N-09、N-10）
| 步骤 | 问题 | 修复 |
|---|---|---|
| verify | `git diff --exit-code HEAD --stat \|\| true` 永远通过 | 去掉 `\|\| true`，改为 `test -z "$(git status --porcelain)"` |
| verify | 只要求 `[Unreleased]` 存在 | 要求存在 `## [$V]` 段（发布前把 Unreleased 改名） |
| build-win | `build_portable.venv_python()` 需要 `.venv` | 新增 `--python` 参数，CI 传 `sys.executable`；venv 只用于本机 |
| build-win | `yolo11n.onnx` 不在仓库 | 加入 `scripts/assets.lock.json`（来源 + sha256），release.yml 在构建前运行 `scripts/fetch_assets.py --only yolo11n.onnx` |
| build-win | `build_rust.py` 写死 `D:/tmp` | 改为 `CARGO_TARGET_DIR` 缺省 `engine-rs/target`；CI 暂时把 Rust 产物移出正式 Release（§9.3） |
| sign | 占位 | 证书到位前保持 draft（现状合理），但 `release_check` 应把"未签名"标为 WARN 并写进 Release 说明 |
| publish | 第三方 Action 未钉 SHA | 钉 SHA |

### 10.2 Docker（N-07、N-08、N-19）
- 容器内监听 `0.0.0.0`（容器网络隔离由宿主端口映射 `127.0.0.1:8000:8000` 控制），同时设 `LS_HOST=0.0.0.0` 让校验知情，并要求 `LS_ADMIN_TOKEN`（compose 里用 `${LS_ADMIN_TOKEN:?}` 强制）；`trusted_hosts` 加 `localhost,127.0.0.1` + 用户配置的外部名。
- go2rtc：去掉 `networks:`；两个容器共享一个卷，服务端设 `LS_GO2RTC_YAML=/shared/go2rtc.yaml`，go2rtc 用 `command: ["-config", "/shared/go2rtc.yaml"]`；凭据文件放同一卷（只读挂给 go2rtc）。
- CPU 镜像：要么锁里加 `onnxruntime-openvino`（单独锁文件），要么把 `LS_BACKEND_ORDER` 改为 `cpu`。
- `Dockerfile.rust`：`COPY --from=builder /build/engine-rs/target/release/labsentinel-server`；web 在多阶段里构建，不依赖宿主 `web/dist`；`LS_HOST=0.0.0.0`；ls-server 改为读 `LS_HOST`。
- 加一个 CI job：`docker compose -f deploy/docker/compose.yaml config`（静态校验）+ `docker build` + 容器内 `curl /api/health/live`。→ R2-401。

### 10.3 安装/升级
- N-18 见 §7。
- 首次在真实 VM 上完成 76 号 §13.6 的 8 项安装验收并留档（当前只有单元/演练测试）。→ R2-404。

### 10.4 文档与可追溯性（N-27、N-30）
- 把 76/77/78 三份文档放入 `docs/plan/`（或 `docs/9x-*`），并修正仓库内 10 处引用。
- CHANGELOG 更正：T-331/T-333 改为"控制面骨架（17 路由）/ L4 未开始"，`Dockerfile.rust` 注释同步。
- 建议恢复一个精简的 `docs/50-tasks/README.md`（或 `docs/plan/status.md`）记录任务状态表（就是本文 §4 的持续更新版）。→ R2-501。

---

## 11. 推进策略

### 11.1 阶段（本轮）
| 阶段 | 目标 | 任务 | 预计 |
|---|---|---|---|
| **R0 发布阻断修复** | 0.2.0 可以按锁安装、可以构建、默认配置安全可用 | R2-001~R2-006、R2-101~R2-104、R2-401（Docker 修到可启动或从 0.2.0 移除）、R2-402、R2-501 | 4–6 人日 |
| **R1 安全收口** | N-11/12/13/14/16/17 | R2-105~R2-110 | 2–3 人日 |
| **R2 GPU 与性能基线** | 恢复 GPU 选项、建立压测基线、决定默认后端 | R2-303、R2-306 | 4–6 人日 |
| **R3 Rust L1 可发布** | 打包 ls-analyzer、ffmpeg 查找、存证、语料扩展 | R2-301、R2-302、R2-304、R2-305、R2-308 | 6–8 人日 |
| **R4 多端 1.1** | 设备令牌读权限 → 配对 UI → Flutter 工程与功能 | R2-103、R2-203、R2-601~R2-605 | 15–20 人日 |
| **R5 Rust L3** | 控制面覆盖率 ≥95%，路由覆盖自动统计 | R2-307、R2-309 | 25–35 人日 |

### 11.2 版本节奏（修订 77 号 §三）
| 版本 | 内容 | 门槛 |
|---|---|---|
| **0.2.0** | 现有 S0–S5 成果 + R0 | §15 清单；Docker 若未修好则**不随 0.2.0 发布**（文档标注"预览"） |
| **0.3.0** | R1 + R2（GPU 可选包、压测基线） | 性能门槛（§14.3） |
| **1.0.0** | 真实 RTSP 24h + 签名 + 0.3.0 稳定 | 76 号 §14 |
| **1.1.0** | R4（移动端 + 局域网） | 76 号 §13.7 Android |
| **1.2.0** | R3（rust 分析器实验性随包）+ Linux Docker 正式 | §9.3 L1 门槛 |
| **2.0.0** | R5（Rust 控制面默认）+ Tauri + macOS | §9.3 L3 门槛 |

### 11.3 风险控制（本轮新增约束）
- **"声明完成"必须附证据**：PR/CHANGELOG 中的完成声明要附测试名或命令输出；涉及部署形态的必须附真实运行截图/日志（这次 Docker/Rust 构建问题都是"写了但没跑过"）。
- **对齐不压倒产品能力**：任何为了"逐位一致"而牺牲用户可见能力（GPU、格式支持）的改动，需要在 ADR 中写明代价与恢复计划。
- 锁文件变更必须带"顶层依赖全部可导入"的自动断言（R2-001）。

---

## 12. 精细化任务清单（R2-xxx）

> 字段同首轮：优先级/类型/证据/涉及文件/现状/方案/步骤/验证/回滚/依赖/影响。

### R0 发布阻断修复

#### R2-001 锁文件补 grpcio / cryptography + 可导入断言
- **优先级/类型**：P0 / REL（N-01）
- **涉及文件**：`server/requirements.in`、三份 `.lock`、`scripts/build_portable.py`（smoke）、`.github/workflows/tests.yml`、新增 `server/tests/test_locked_imports.py`
- **步骤**：
  1. 查生成物版本：`head -20 server/app/analyzer_pb2_grpc.py`（`GRPC_GENERATED_VERSION`），`requirements.in` 加 `grpcio>=<该版本>`；加 `cryptography>=43,<46`（以锁定时最新稳定为准）。
  2. 重锁：
     ```bash
     uv pip compile server/requirements.in -o server/requirements-win.lock --generate-hashes --python-version 3.12 --python-platform x86_64-pc-windows-msvc --override server/overrides.txt
     uv pip compile server/requirements.in -o server/requirements-linux.lock --generate-hashes --python-version 3.12 --python-platform x86_64-unknown-linux-gnu --override server/overrides.txt
     ```
     （CUDA 锁同理，使用 `requirements-cuda.in`。）
  3. 新测试（CI 中按锁安装后运行）：
     ```python
     import importlib, re
     from pathlib import Path
     IMPORT_NAME = {"opencv-python-headless": "cv2", "python-multipart": "multipart",
                    "pydantic-settings": "pydantic_settings", "paho-mqtt": "paho.mqtt.client",
                    "uvicorn[standard]": "uvicorn", "grpcio": "grpc", "defusedxml": "defusedxml"}
     def test_top_level_requirements_importable():
         req = Path(__file__).resolve().parents[1] / "requirements.in"
         for ln in req.read_text("ascii").splitlines():
             ln = ln.split("#")[0].strip()
             if not ln or "never" in ln: continue
             name = re.split(r"[<>=; ]", ln)[0]
             if "win32" in ln and __import__("sys").platform != "win32": continue
             if "!= \"win32\"" in ln and __import__("sys").platform == "win32": continue
             importlib.import_module(IMPORT_NAME.get(name, name.replace("-", "_")))
     ```
  4. `build_portable.smoke()` 追加 `import grpc, cryptography`。
- **验证**：在全新 venv 中 `pip install --require-hashes --no-deps -r server/requirements-win.lock && python -c "import grpc, cryptography"`；`pytest tests/test_locked_imports.py`。
- **回滚**：revert；**依赖**：R2-002（先把 `.in` ASCII 化）；**影响**：便携包体积 +约 15MB。

#### R2-002 依赖事实源统一
- P2 / REL（N-04）。`requirements.in` 注释改英文；`requirements.txt` 改为：
  ```
  # Deprecated: use requirements.in (source) / requirements-<os>.lock (install).
  -r requirements.in
  ```
  `deploy.py pip_install` 改为按平台安装锁文件（`--require-hashes --no-deps`）；README 方式 B 同步。
- 验证：`test_requirements_ascii.py` 扩展到 `.in`；`python scripts/deploy.py install` 在干净机器成功。

#### R2-003 Rust 工具链文件修复
- P1 / REL（N-15）。见 §7 片段；`build_rust.py` 的 GNU 配方改用 `cargo +stable-x86_64-pc-windows-gnu`；`rust.yml` 中 `dtolnay/rust-toolchain` 与 toolchain 文件版本保持一致（直接读文件，或改用 `rustup show` 自动安装）。
- 验证：本机 `cd engine-rs && cargo test --workspace` 不再报 toolchain 错误；CI rust.yml 绿。

#### R2-004 发布流水线可运行
- P1 / REL（N-09、N-10）。按 §10.1 表逐项修；`yolo11n.onnx` 入 `assets.lock.json`（来源：Ultralytics 官方 release 或项目自己的 Release 资产，sha256 实测写入）；
  Rust 产物步骤改为独立 job `nightly-rust`，不进 `publish`。
- 验证：在 fork 上打 `v0.2.0-rc.1` 标签，全流程到 draft release；下载 zip 后 `smoke_portable.py` 通过。

#### R2-005 测试基线回归（6 个失败 → 0）
- P1 / QA（N-35、N-36、N-38）。
  1. `diagnostics.py:21-24` 改读 `engine._engine.backend if engine._engine else engine.PRELOAD 状态`（与 `/api/system` 同口径），不触发加载。
  2. `test_store_bidirectional.py:27`：`pytest.skip("labsentinel-server.exe 未构建")`，并在 `rust.yml` 新增一个 job：构建 ls-server 后用 `LS_SERVER_EXE` 跑这 3 个用例（保证跨语言兼容测试真的在 CI 中执行）。
  3. `test_web_static::test_gzip_assets_and_json`：断言 `r.headers["content-encoding"] == "gzip"` 且 `len(r.content) >= 1024`（httpx 已解压），删除 `gzip.decompress` 与 `or True`。
  4. `test_health_errors::test_ready_ok_then_model_missing`：在 `tmp_path` 写一个假模型文件并 monkeypatch `settings.model_fp32`，不依赖仓库外文件。
- 验证：干净检出 + 锁文件环境下 `pytest tests -q` 0 失败；CI 双平台绿。

#### R2-006 子进程输出解码统一（N-37、N-39）
- P2 / BUG。新增 `platform/process.run_text(argv, **kw)`：Windows 上 `encoding="mbcs", errors="replace"`（PowerShell/系统工具输出 ANSI），POSIX `encoding="utf-8", errors="replace"`；全仓 `text=True` 调用（`backups.py:79`、`platform/devices.py:26,52`、`platform/process.py:30,40,67`）改用它。
  PowerShell 调用可加 `[Console]::OutputEncoding=[Text.Encoding]::UTF8;` 前缀后按 UTF-8 解码（二选一，推荐前缀方案，结果与系统语言无关）。
- 验证：`PYTHONUTF8=1` 下中文路径进程 `is_our_process` 返回 True；测试输出无 `UnicodeDecodeError`；自动备份失败时日志带 stderr。

#### R2-101 （空号，保留与首轮编号对齐）

#### R2-102 监听地址单一来源 + 运行时校验
- **优先级/类型**：P1 / SEC（N-03）
- **涉及文件**：`config.py`（新增 `host: str = "127.0.0.1"`）、`validate.py:21-42`、`api.py:66-73`、`main.py:59-63`、`scripts/portable_launcher.py`（`--host`）、`deploy/*`、`docs/部署指南*.md`
- **方案**：
  - 所有判断改读 `settings.host`（pydantic 会同时读环境变量与 `.env`）。
  - 启动器/服务配置一律 `--host {settings.host}`。
  - 运行时兜底：
    ```python
    # main.py lifespan 之后的首个请求（或 uvicorn 启动回调）
    def _enforce_bind(scope) -> None:
        host = (scope.get("server") or ("127.0.0.1", 0))[0]
        if host not in ("127.0.0.1", "::1", "localhost"):
            if not settings.admin_token or settings.auth_disabled:
                logging.getLogger("app.security").critical("非回环监听但无令牌/逃生口开启——拒绝服务")
                os._exit(3)
            settings.auth_read_protected = True
    ```
    （更干净的做法：自定义 uvicorn `Server` 启动钩子；或在请求中间件里第一次请求时检查一次并缓存。）
  - 安全类 FAIL 无条件拒绝启动（新增 `level="FATAL"`）。
- **验证**：`uvicorn app.main:app --host 0.0.0.0`（无令牌）→ 进程退出码 3；`.env` 里写 `LS_HOST=0.0.0.0` 无令牌 → 启动即退出；有令牌 → 启动且 `/api/events` 匿名 401。
- **回滚**：revert；**影响**：之前"手工 --host 0.0.0.0 且无令牌"的用户升级后无法启动（发布说明写明，这是期望行为）。

#### R2-103 设备令牌纳入读权限与 WS
- P1 / FUNC+SEC（N-02）
- **代码提示**：
  ```python
  def _read_token_ok(provided: str | None) -> bool:
      if not settings.admin_token or _auth_off():
          return True
      if not provided:
          return False
      if hmac.compare_digest(provided, settings.admin_token):
          return True
      if settings.read_token and hmac.compare_digest(provided, settings.read_token):
          return True
      from . import devices
      return store is not None and devices.verify(store, provided) is not None
  ```
  同时支持 `Authorization: Bearer <token>` 头（移动端惯例），与 `X-Admin-Token` 等价；`devices.verify` 的 `touch_device` 改为最多每 60 秒写一次库（避免每个读请求都写 SQLite）。
- **验证**：配对 → 用设备令牌 GET `/api/events`（读保护开）200；PUT `/api/cameras` 401；WS 子协议 bearer.<设备令牌> 连接成功；吊销后 401。

#### R2-104 WebRTC 信令 WS 回显子协议
- P1 / BUG（N-05）
- **代码提示**：
  ```python
  async def webrtc_ws(ws: WebSocket, cid: int):
      ...
      protos = [p.strip() for p in ws.headers.get("sec-websocket-protocol", "").split(",")]
      await ws.accept(subprotocol="labsentinel" if "labsentinel" in protos else None)
  ```
  抽一个 `_accept_with_subprotocol(ws)` 公共函数给两个 WS 用。
- **验证**：pytest：`client.websocket_connect("/ws/webrtc/1", subprotocols=["labsentinel", "bearer.x"])` 断言 `ws.accepted_subprotocol == "labsentinel"`；实机：开令牌后摄像头页与 `/wall` 出画面。

### R1 安全收口

#### R2-105 文件夹黑名单改为"系统根包含判断"
- P2 / SEC（N-11）
- **代码提示**：
  ```python
  def _system_roots() -> list[Path]:
      if os.name == "nt":
          env = [os.environ.get(k) for k in ("WINDIR", "ProgramFiles", "ProgramFiles(x86)", "ProgramData")]
          return [Path(p).resolve() for p in env if p]
      return [Path(p) for p in ("/etc", "/usr", "/bin", "/sbin", "/boot", "/proc", "/sys", "/System", "/Library")]
  def _under(p: Path, root: Path) -> bool:
      try:
          return os.path.commonpath([str(p), str(root)]) == str(root)
      except ValueError:
          return False
  # check_folder_root 中：if any(_under(real, r) for r in _system_roots()): raise SourceError(...)
  ```
- **验证**：`C:\Windows`、`C:\Windows\Web`、`/etc` 拒绝；`D:\data\Library\shots`、`~/Pictures` 通过。

#### R2-106 SecretStore 加固
- P2 / SEC（N-12）：加密不可用时 `protect` 抛 `AlertConfigError("加密组件不可用，拒绝明文保存密钥")`；DPAPI 增加 `CRYPTPROTECT_LOCAL_MACHINE` 选项（服务模式自动启用，配置 `LS_SECRET_SCOPE=user|machine`）；
  迁移工具 `scripts/secrets_rekey.py`（用户范围 → 机器范围，需在原用户会话执行）。
- 验证：卸载 cryptography 后在 Linux 保存渠道 → 400 且 DB 无明文；服务模式下保存/重启/发送测试成功。

#### R2-107 更新验签按 minisign 格式实现
- P2 / BUG（N-13），见 §6 片段；新增测试：用 `minisign`（或 Python 实现的测试密钥对）生成 `ED` 与 `Ed` 两种签名各一份，验证通过；篡改 1 字节失败；key_id 不匹配失败。

#### R2-108 testserver 只在测试中允许
- P3（N-14）：`if os.environ.get("PYTEST_CURRENT_TEST") or settings.testing: hosts.append("testserver")`。

#### R2-109 sidecar 令牌改环境变量传递
- P3（N-16）：`child_env["LS_ANALYZER_TOKEN"] = token`；Rust 侧优先读环境变量，命令行 `--token` 保留一个版本兼容。

#### R2-110 配对码防爆破
- P3（N-17）：`claim` 失败计数（进程内，按 IP），5 次失败作废所有待配对码并要求管理员重新生成；接口单独限流 10/分钟。

### R2 GPU 与性能基线

#### R2-303 Windows GPU 推理恢复为可选变体
- **优先级/类型**：P1 / PERF（N-20）
- **方案**：
  1. 新增 `server/requirements-win-dml.in`（`-r requirements.in` + 覆盖：`onnxruntime-directml==<最新可用>`，并 override 掉 CPU `onnxruntime`）→ `requirements-win-dml.lock`。
  2. `build_portable.py --variant cpu|dml` 产出两个 zip（`LabSentinel-portable-<ver>-cpu.zip` / `-dml.zip`）。
  3. 首启（或设置页）探测：`ort.get_available_providers()` 含 `DmlExecutionProvider` 才显示"GPU 加速：开启"；CPU 版界面提示可下载 GPU 版。
  4. 黄金语料验收改为分层容差（§9.1），DML 变体单独跑一遍并记录一致率。
  5. Rust：`ort` 后端 + `ORT_DYLIB_PATH` 指向随包 `onnxruntime.dll`（cpu）或 DML 版 DLL；ls-analyzer 默认 `LS_INFER_BACKEND=ort`，tract 兜底。
- **验证**：两种 zip 均通过冒烟；DML 变体在有 DX12 GPU 的机器上 `/api/system.backend == "directml"`；R2-306 压测对比。
- **依赖**：R2-306（先测 CPU 基线）。

#### R2-306 压测基线（补 T-307）
- P1 / PERF
- **步骤**：`tools/camera_soak.py` 跑 1/4/8 路 720p 文件源 @5fps，各 30 分钟，三种实现：Python-CPU、Python-DML（R2-303 后）、Rust-tract、Rust-ort；
  记录每路分析 fps、事件端到端 P95、CPU%、RSS、句柄数；结果写 `docs/perf-baseline.md`（附机器配置）。
- **门槛**：沿用 76 号 §13.5，并新增"Rust 路径吞吐 ≥ Python 同后端 1.5 倍"作为推荐切换的条件。

### R3 Rust L1 可发布

#### R2-301 Rust 统一二进制查找
- P2（N-22）：`ls-core` 新增 `bin.rs`（见 §7）；`ls-ingest`、`evidence.rs` 全部改用；SidecarAnalyzer 启动时把 `LS_FFMPEG` 传给子进程（取自 Python `platform.binaries.find("ffmpeg")`）。
- 验证：PATH 中无 ffmpeg、`tools/ffmpeg.exe` 存在时，rust 路径正常取流与存证。

#### R2-302 存证编码修复
- P2（N-23）：读取并返回 stderr；对 ffmpeg 6.x/7.x/9.x 三个版本测试；测试在 ffmpeg 不存在时 skip、存在时必须通过。
- 验证：`cargo test -p ls-analyzer evidence` 在本机 ffmpeg 9.0.2 下通过。

#### R2-304 打包 ls-analyzer
- P1（N-06）：`build_portable.py` 增加 `--with-rust`：构建 `ls-analyzer`（release）放入 `bin/`，同时携带 `onnxruntime.dll`；`portable_launcher.start_server` 设置 `LS_ANALYZER_EXE`；
  `analyzer_sidecar._DEFAULT_EXE` 改为相对安装目录的 `bin/ls-analyzer[.exe]`，找不到时 `available=False`（UI 置灰，F2-02）。
- 验证：便携包中切换一路到 rust → 事件正常入库；删除 `bin/ls-analyzer.exe` → UI 置灰且原因清楚。

#### R2-305 Rust 告警读取网页配置
- P2（N-12）：Python 在 `ApplyConfig` 快照中下发**已解密**的渠道配置（仅经 127.0.0.1 gRPC，令牌认证），Rust 不直接读 `app_settings`、不实现 DPAPI；ls-server 独立运行时则读取同一配置（L3 时再实现跨语言 SecretStore，ADR 记录）。
- 验证：网页端保存企微 webhook → rust 分析器路径的事件能推送到企微测试群。

#### R2-308 黄金语料扩展
- P2（N-21）：补齐 76 号 T-321 列出的场景（≥10 段：遮挡、夜间、重叠区、快速往返、断流重连、文件夹截图集、穿/未穿、多人、超员）；素材来源用 Q-07 自拍 CC0 素材；一致率门槛按 §9.1 分层容差。

### R4 多端 1.1

#### R2-203 设备管理前端
- P1（F2-01）：设置页"设备"分区：生成配对码（显示 6 位码 + 二维码 `labsentinel://pair?u=<base>&c=<code>&fp=<cert_sha256>`，二维码用 `qrcode` npm 包本地生成）、设备列表（名称/平台/最后在线）、吊销。
- 验证：vitest 覆盖；手机扫码（R2-602 后）端到端。

#### R2-204 分析器可用性透出
- P2（F2-02/F2-03）：`/api/system.components.analyzer_sidecar`；切换接口在不可用时 409 并给出原因；前端置灰。

#### R2-601 ~ R2-605 Flutter 移动端（沿用 76 号 T-702~T-706，补充）
- R2-601（=T-702）平台目录、包名、网络安全配置。
- R2-602（=T-703）配对扫码（`mobile_scanner`）、`flutter_secure_storage` 保存设备令牌、`Authorization: Bearer`、WS 子协议、证书指纹固定。
- R2-603（=T-704）事件列表/详情/处置/实时/本地通知；拉取 `/api/v1/meta` 文案。
- R2-604（=T-705）mobile.yml：APK 构建与签名。
- R2-605（=T-706）widget/integration 测试（服务端用 Docker——依赖 R2-401）。
- **前置**：R2-102、R2-103、R2-104（服务端口径必须先正确）。

### R5 Rust L3

#### R2-307 路由覆盖率统计进 CI
- P2（N-27）：脚本 `scripts/rust_route_coverage.py`：解析 `docs/openapi.json` 路径集合与 `ls-server` 注册路由（启动时 `--dump-routes` 输出 JSON），输出覆盖率与缺口清单；CI 打印并保存为 artifact；L3 门槛 ≥95%。

#### R2-309 ls-server 补齐（按路由组分批）
- 顺序建议：鉴权与安全中间件对齐（首启令牌/读保护/LS_HOST/设备令牌）→ cameras/zones/rules（配置主路径）→ events 全套 + 媒体签名 → alerts 配置与测试 → export/stats 全套 → folders → face/plugins/genai（最后）。
- 每批：契约测试同时跑 Python/Rust；覆盖率脚本数字上升；CHANGELOG 写真实覆盖率。

### 文档

#### R2-501 计划文档入库与口径更正
- P2（N-27、N-30）：`docs/plan/76-...md`、`77-...md`、`78-...md`（本文）入库；修正 10 处引用路径；CHANGELOG 中 T-331/T-333、`Dockerfile.rust` 注释更正；新增 `docs/plan/status.md` 作为 §4 的持续更新版（每批任务完成后更新）。

#### R2-401 Docker 修复（见 §10.2）
- P1（N-07/N-08/N-19）。验证（需 Docker 机器）：
  ```bash
  docker compose -f deploy/docker/compose.yaml config
  LS_ADMIN_TOKEN=<占位：运行时用 openssl rand -hex 24 生成> docker compose -f deploy/docker/compose.yaml up -d --build
  curl -fsS http://127.0.0.1:8000/api/health/live
  docker compose exec labsentinel curl -fsS -u "$(jq -r '.username+":"+.password' /data/go2rtc.secret)" http://127.0.0.1:1984/api/streams
  ```

#### R2-402 升级健康检查做实
- P2（N-18），见 §7。

#### R2-404 VM 安装验收留档
- P2：按 76 号 §13.6 八项在 Win10/Win11 VM 执行，截图/日志存 `docs/验证记录-安装升级回滚.md`。

### 其他
- R2-502 前端测试补齐（N-25）：WebrtcPlayer（子协议、断线重连）、Wall（网格上限 9）、Models（下载确认流程）、mediaUrl 批量签名、配对 UI。
- R2-503 审计写入移出事件循环（N-24）：`await run_in_threadpool(_audit_write, ...)` 或队列 + 单写线程。
- R2-504 第三方 Action 钉 SHA（N-33）+ Dependabot 配置 `.github/dependabot.yml`（pip/npm/cargo/github-actions 每周）。
- R2-505 CI 增加 `cargo test --workspace --all-features`（N-32）。
- R2-506 本机路径清理（N-29）：所有 `D:/tmp/...` 默认值改为仓库相对路径或环境变量必填。
- R2-507 N-28 复核 PPE 接线与注释。

---

## 13. 子代理执行拆分

| 波次 | 并行子代理 | 说明 |
|---|---|---|
| W-R0 | A：R2-001+R2-002（依赖）· B：R2-003+R2-505+R2-506（Rust 构建）· C：R2-102+R2-103+R2-104（鉴权链，同一人：改 `api.py/main.py/validate.py`）· D：R2-004+R2-402（发布）· E：R2-501（文档） | C 与 A 无文件交集；D 依赖 A（smoke 断言） |
| W-R1 | F：R2-105~R2-110 · G：R2-401（需要有 Docker 的执行环境） | F 改 `sources.py/secretstore.py/update_check.py`，与 C 的 `api.py` 改动在 R2-103 合并后再开始 |
| W-R2 | H：R2-306 → R2-303 | 需要有独显/核显的 Windows 机器 |
| W-R3 | I：R2-301+R2-302 · J：R2-304（依赖 A、B、I）· K：R2-305 · L：R2-308 | — |
| W-R4 | M：R2-203+R2-204+R2-502（前端）· N：R2-601~R2-605（Flutter，需 Flutter 环境） | N 依赖 C 合并 |
| W-R5 | O：R2-307 → R2-309（按路由组多个 PR） | — |

**子代理通用规则（追加到首轮 §12.3）**：
- 完成声明必须附"实际运行"证据（命令 + 输出摘要）；部署类任务必须附真实环境运行日志。
- 不得以"对齐"为由删除用户可见能力；如必须，先写 ADR。
- 锁文件改动必须跑 `test_locked_imports.py`。

---

## 14. 测试与验收标准（更新）

### 14.1 本轮实测结果
| 项 | 结果 |
|---|---|
| 锁文件安装（win，Python 3.13 venv，`--require-hashes --no-deps`） | exit 0；`pip check` 报 supervision 需要 opencv-python（预期，已 override）；**`import grpc` / `import cryptography` 失败** |
| pytest（临时副本，锁文件环境，Python 3.13，`LS_LOG_FILE=0 PYTHONUTF8=1`） | **587 通过 / 6 失败 / 4 跳过**，用时 609s。失败：`test_health_errors::test_ready_ok_then_model_missing`（503，依赖模型文件，N-38）、`test_logging_diag::test_summary_endpoint`（500，N-35）、`test_store_bidirectional` ×3（缺 Rust 产物时 assert，N-36）、`test_web_static::test_gzip_assets_and_json`（N-38）。跳过：sidecar/L3 契约用例（缺 exe，正确 skip）。另有 10+ 条 `UnicodeDecodeError` 线程告警（N-37） |
| vitest | **3 个文件 28 个测试全部通过**（vitest 5.1.0） |
| 前端构建 | 成功（js 186.5kB / gzip 65.2kB） |
| npm audit | 生产依赖 0；开发依赖 5 个 moderate（glob/minimatch 链） |
| pip-audit（win 锁、linux 锁） | 均为 **No known vulnerabilities found**（注意：锁里缺 grpcio/cryptography，补入后需重跑） |
| ruff（E9,F,B,S,PLE，宽规则统计） | 232 条，无 F821；主要是 S603/S607（子进程调用，多为受控参数）、F401×31（未用导入）、B904×22；CI 门禁只取 E9/F821/PLE，通过 |
| cargo（rust-toolchain.toml 原样） | **失败**：`error: target tuple in channel name 'stable-x86_64-pc-windows-gnu'` |
| cargo test（显式 `+stable-x86_64-pc-windows-msvc`，`--no-fail-fast`） | **69 通过 / 2 失败**：① `ls-analyzer evidence::tests::encode_mp4_integration`（`evidence.rs:343` `expect("encode")` panic，本机 ffmpeg 9.0.2）；② `ls-dispatch business_code_and_retry_semantics`（`dispatch_integration.rs:137` 桩收到的请求数 ≠ 4，耗时 20s——重试计数与桩线程记录存在竞态，判为不稳定用例，见 N-34） |
| clippy（`--workspace --all-targets -D warnings`） | 通过 |
| cargo fmt --check | 通过 |

<!-- 实测输出在审查执行结束时追加于附录 B -->

### 14.2 新增回归测试（必须随对应任务提交）
`test_locked_imports.py`、`test_bind_enforcement.py`（R2-102）、`test_device_token_read.py`（R2-103）、`test_webrtc_subprotocol.py`（R2-104）、
`test_folder_system_roots.py`（R2-105）、`test_secretstore_no_plaintext.py`（R2-106）、`test_update_minisign.py`（R2-107）、
`engine-rs/crates/ls-core/tests/bin_lookup.rs`（R2-301）、Docker compose 冒烟 job（R2-401）、`rust_route_coverage`（R2-307）。

### 14.3 性能验收（补充）
- Windows CPU 变体：4 路 720p@5fps 每路 ≥ 4.5fps（若达不到，0.3.0 默认推荐 DML 变体）。
- DML 变体：8 路 720p@5fps 每路 ≥ 4.5fps；事件 P95 ≤ 1.0s。
- Rust-ort 与 Python 同后端：吞吐 ≥ 1.5×，RSS ≤ Python 的 60%。

### 14.4 多端验收（补充）
- 局域网真机：手机配对 → 读事件 → 处置 → 吊销，全程在读保护开启、`LS_HOST=0.0.0.0` 下完成。
- `/wall` 在令牌开启时 9 路画面全部出图。

---

## 15. 发布检查清单（0.2.0 专用）

- [ ] R2-001 锁文件可导入断言通过（win + linux）
- [ ] R2-002 `requirements.txt` 不再与锁分叉
- [ ] R2-005 干净检出下 pytest 0 失败（CI 双平台绿）
- [ ] R2-006 中文系统 + `PYTHONUTF8=1` 下进程识别/停止/卸载正常（实测）
- [ ] R2-003 `cargo test` 在本机与 CI 都能直接运行
- [ ] R2-004 release 流水线在 fork 上跑通到 draft
- [ ] R2-102 非回环无令牌拒绝启动（实测）
- [ ] R2-103 设备令牌可读（若 0.2.0 不宣传移动端，可推迟到 1.1，但需在文档注明）
- [ ] R2-104 开令牌后实时画面可用（实机截图）
- [ ] Docker：R2-401 修好并实机验证，**或**从 0.2.0 发布说明中移除 Docker 并标注"预览"
- [ ] Rust 产物（labsentinel-server / ls-analyzer）不进入 0.2.0 正式 Release
- [ ] CHANGELOG 中 L3/L4 口径已更正（R2-501）
- [ ] 发布说明写明：Windows 默认 CPU 推理（GPU 变体在 0.3.0 提供）
- [ ] 76 号 §14 其余项（签名为 WARN：证书未到位时保持草稿并在说明中标注"未签名预览版"）

---

## 16. 待确认问题与风险

| ID | 问题 | 建议默认 |
|---|---|---|
| Q2-01 | 0.2.0 是否包含 Docker | 不包含（作为预览），等 R2-401 实机验证后随 0.3.0 正式 |
| Q2-02 | Windows 默认推理用 CPU 还是 DML | 先发 CPU（兼容性最好）+ 可选 DML 包；R2-306 数据出来后若 DML 在主流核显上稳定，1.0 默认改 DML |
| Q2-03 | 黄金语料"逐位一致"是否继续作为硬门槛 | 改为分层容差（§9.1）；逐位一致作为同后端回归的附加检查 |
| Q2-04 | 被删除的历史文档（`499348b` 共 187 个文件）是否需要归档 | 不恢复到 `docs/`，但打一个 tag（如 `archive/docs-2026-11-07`）指向删除前的提交，便于追溯 |
| Q2-05 | Rust 控制面工作量（约 80 条路由）是否维持 2.0 目标 | 维持；以 R2-307 覆盖率数字跟踪，每月复盘一次 |

**风险**
- R2-1：鉴权链修复（R2-102/103/104）会改变默认行为，必须配合发布说明与升级指引。
- R2-2：GPU 变体引入两套锁与两种包，构建矩阵变复杂——用同一个 `build_portable.py --variant` 控制，CI 两个 job。
- R2-3：Rust 迁移的"已完成"口径与实际偏差，若不纠正会误导后续排期与对外宣传。
- R2-4：唯一硬阻塞"真实 RTSP 摄像头 24h"仍未完成。

---

## 17. 附录

### 附录 A：本轮只读命令
```bash
git log --oneline -40; git ls-files | wc -l
git log --all --oneline -- 'docs/76-*' 'docs/77-*'          # → 无输出（从未入库）
python -c "open('server/requirements.in','rb').read().decode('ascii')"   # → UnicodeDecodeError（位置 390）
diff <(grep -v '^#' server/requirements.txt) <(grep -v '^#' server/requirements.in)
grep -nE "^(grpcio|cryptography)" server/requirements-*.lock    # → 无
# 临时副本：
git archive HEAD | tar -x -C <scratch>/repo
python -m venv <scratch>/venv && <scratch>/venv/Scripts/python -m pip install --require-hashes --no-deps -r server/requirements-win.lock
<scratch>/venv/Scripts/python -c "import grpc"                  # → ModuleNotFoundError
cd engine-rs && cargo test --workspace                          # → error: target tuple in channel name
cargo +stable-x86_64-pc-windows-msvc test --workspace --no-fail-fast
cargo +stable-x86_64-pc-windows-msvc clippy --workspace --all-targets -- -D warnings
cd web && npm ci && npx vitest run && npm run build && npm audit --registry=https://registry.npmjs.org
grep -nE '\.route\(' engine-rs/crates/ls-server/src/main.rs     # → 17 条（含 /ws/events）
python -c "import json;print(len(json.load(open('docs/openapi.json'))['paths']))"   # → 97
```

### 附录 B：实测输出（摘录）
```
# 锁文件安装后
lock-exit=0
supervision 0.25.1 requires opencv-python, which is not installed.   # 预期（override 去重）
ModuleNotFoundError: No module named 'grpc'
ModuleNotFoundError: No module named 'cryptography'

# pytest
6 failed, 587 passed, 4 skipped, 347 warnings in 609.26s (0:10:09)
FAILED tests/test_health_errors.py::test_ready_ok_then_model_missing - assert 503 == 200
FAILED tests/test_logging_diag.py::test_summary_endpoint - assert 500 == 200
FAILED tests/test_store_bidirectional.py::test_path1_python_build_rust_rw_python_verify
FAILED tests/test_store_bidirectional.py::test_path2_minimal_db_rust_rw_python_rw
FAILED tests/test_store_bidirectional.py::test_path3_concurrent_wal_visibility
FAILED tests/test_web_static.py::test_gzip_assets_and_json - gzip.BadGzipFile: Not a gzipped file (b'{"')
RuntimeError: no backend available: directml: ... yolo11n.onnx failed. File doesn't exist; openvino: ...; cpu: ...
AssertionError: labsentinel-server.exe 未构建（先 cargo build -p ls-server）
UnicodeDecodeError: 'utf-8' codec can't decode byte 0xb2 in position 0: invalid start byte   # 子进程读取线程，10+ 次

# vitest / build / npm audit
Test Files  3 passed (3)   Tests  28 passed (28)
dist/assets/index-DqFtvLOy.js   186.50 kB │ gzip: 65.24 kB
5 moderate severity vulnerabilities          # 全量
found 0 vulnerabilities                      # --omit=dev

# pip-audit
requirements-win.lock   → No known vulnerabilities found
requirements-linux.lock → No known vulnerabilities found

# cargo
cargo test --workspace           → error: target tuple in channel name 'stable-x86_64-pc-windows-gnu'
cargo +stable-x86_64-pc-windows-msvc test --workspace --no-fail-fast → 69 passed, 2 failed
  evidence::tests::encode_mp4_integration ... FAILED  (evidence.rs:343:41)
  business_code_and_retry_semantics ... FAILED        (dispatch_integration.rs:137:5，耗时 20s)
cargo clippy --workspace --all-targets -- -D warnings → Finished（无告警）
cargo fmt --all -- --check → 0
```
