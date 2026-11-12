# Release Notes · LabSentinel v0.2.0

> Draft for GitHub Release（78 号审阅 §15 发布检查清单产物）。**未发布**——签名证书
> 未到位，按 §15 口径保持 draft 并标注「未签名预览版」。
> 数字纪律：所有数字来自仓库文档/实测命令（出处随行标注）；未经实测的能力如实标
> 「待验证/预览」。

**Status:** beta 前夜 —— S0–S5 加固 + R0–R5 二轮收口完成（见
`docs/plan/status.md` 与 `CHANGELOG.md ## [0.2.0]`）。

## Highlights（相对 0.1.0-alpha）

- **安全链路收口**：监听地址单一事实源（`settings.host`）+ 运行时绑定校验；
  非回环无令牌**拒绝启动**（FATAL，不再只是 strict 警告）；设备令牌纳入读权限
  与 WebSocket（Bearer 头等价）；WebRTC 信令回显子协议（开令牌后实时画面/大屏
  可用）；POSIX 密钥加密组件不可用 → **拒绝明文保存**；更新清单按 minisign
  格式验签（Ed/ED 双算法）；文件夹黑名单改「系统根包含判断」；配对码防爆破
  （5 次失败作废 + 按 IP 限流）；sidecar 一次性令牌改环境变量传递。
- **依赖事实源统一**：`requirements.in` + win/linux/cuda/**dml** 四份带哈希锁
  （grpcio/cryptography 已入锁）；`requirements.txt` 降级为兼容 shim；
  顶层依赖可导入断言进 CI（`test_locked_imports.py`）。
- **发布工程可运行**：`release.yml` 干净树/CHANGELOG/模型资产全链校验；
  `build_portable.py --python/--models-dir/--variant cpu|dml/--with-rust`；
  `yolo11n.onnx` 入 `assets.lock.json`（SHA256）；升级健康检查真实启动新版本
  并轮询 `/api/health/ready`（失败自动回滚）；全部第三方 Action 钉 commit SHA。
- **Rust 迁移如实推进**：数据面 L1 可运行（ls-analyzer 双后端 tract/ort、
  存证 MP4、统一 ffmpeg 查找）；控制面 L3 路由覆盖 **105/106 = 99.1%**
  （≥95% L3 门槛已过，`scripts/rust_route_coverage.py`）；网页端保存的告警
  渠道在 Rust 路径真实生效（Fernet 兼容解密注入 ls-jobs）。
- **GPU 变体实测**（R2-303）：可选 DML 变体锁 `requirements-win-dml.lock`
  （onnxruntime-directml 1.24.4）。Radeon 880M 核显实测：单路 **69.43fps**
  （CPU 1.94×）、4 路 **17.5fps/路**（1.80×），远超 4.5fps/路 门槛
  （`docs/perf-baseline.md`）。
- **便携包启动链 8/8 冒烟全绿**（发布前实证）：`smoke_portable.py` 隔离
  解包 → 启动 → 就绪 → API/前端/播种断言 → 停止 → 隔离零污染，首次完整
  跑通前修复 4 个真实缺陷（`health_check.py` 漏随包、go2rtc 锁覆盖单实例
  锁、PID 防护拒停探针启动的服务、5 处 `python -c` 动态代码串文件化），
  并对齐 2 处冒烟断言（T-406 无演示素材播种口径 / 回环不走系统代理）。
- **安装包形态（ADR-0010）**：同一 staging 产物同时出便携 zip 与
  `LabSentinel-setup-<ver>.exe`（Inno Setup 6，mpv.net/Tautulli 双形态先例）；
  按用户安装、覆盖升级、开始菜单/控制面板卸载项、可选 Windows 服务勾选页，
  装完自动真实健康检查；升级/回滚能力复用现有 Python 安装器。CI 由
  `choco install innosetup` 编译（本地未装 ISCC 如实标注待 CI 验证）。
- **Rust 控制面归档化管理**：ls-server 巨石 main.rs（5179 行）拆为共享层 +
  11 个功能域模块（1349 行主文件），便于按域归档与后续演进；门禁全绿。

## 行为变化（升级须知）

1. **监听与鉴权**（R2-102/R2-103）：`--host 0.0.0.0` 或 `.env` 写
   `LS_HOST=0.0.0.0` 而未配置 `LS_ADMIN_TOKEN` → 服务**拒绝启动**（退出码 3）。
   这是期望行为：局域网暴露必须配令牌；开启读保护后，设备令牌（配对）与管理/
   只读令牌均可读，`Authorization: Bearer` 与 `X-Admin-Token` 等价。
2. **Web 实时画面**（R2-104）：开令牌后浏览器与 `/ws/webrtc/{cid}` 的握手要求
   子协议回显——旧版本前端/第三方播放器若不带 `labsentinel` 子协议，需要更新
   前端（内置前端已同步）。
3. **Windows 默认推理为 CPU**（onnxruntime 1.31.1，与 Rust ort 同版本，用于
   对齐与兼容）；DirectML 移至可选 DML 变体包，**0.3.0 随正式 GPU 变体提供**
   （本版发布说明与 Dashboard 已显示探测状态）。若追求吞吐可先按
   `server/requirements-win-dml.lock` 自行构建。
4. **POSIX 密钥保存**：`cryptography` 不可用时 `protect()` 抛错拒绝明文入库
   （原为静默降级 + 仅日志告警）。

## 随版形态与边界

- **Docker 支持终止**（维护者决策，2026-11-11）：`deploy/docker/*`（compose /
  Dockerfile / Dockerfile.cuda / Dockerfile.rust）已从仓库移除，不再修复其已知
  问题（绑定回环、go2rtc 配置卷不共享等）；Linux 部署请使用 systemd 原生路线
  （`docs/部署指南-Linux.md`）。
- **Rust 产物不进正式 Release**：`labsentinel-server` / `ls-analyzer` 仅作为
  nightly artifact（`release.yml nightly-rust` job，保留 7 天）；ls-server L3
  仍有 1 条路由未实现（`/api/folders/{cid}/scan` 需推理管线接线，维持 503）。
- **未签名预览版**：`release_check` 将「未签名」标为 WARN；证书到位前保持
  draft。
- Python macOS 打包按 77 号 Q-16 取消（等 Rust L4）。

## 验证基线（2026-11-11，本机 Windows / Py3.12 / rustup 1.29 / node 24.22）

| 项 | 结果 |
|---|---|
| Python 全量 | 629 passed / 7 skipped / 0 failed |
| 前端（vitest + build） | **44/44** 通过；构建成功 |
| Rust 全量 | **93 passed / 0 failed**（`--workspace --all-features`，GNU 配方；clippy/fmt 通过） |
| ls-server L3 覆盖率 | **105/106 = 99.1%** |
| Python-DML 实测 | 单路 69.43fps / 4 路 17.5fps/路（§14.3 门槛 4.5） |

## 已知问题与余量（如实保留）

1. **杀毒软件启发式误报**：便携包的「启动器 EXE → 解释器托管本地服务」形态与
   恶意软件画像重叠，可能被启发式引擎（HEUR 类）命中——启动链已审计清白
   （无动态代码执行），`LabSentinel.exe` 已注入版本资源与应用清单；遇到告警请按
   **`docs/杀毒软件误报说明.md`** 处理（来源校验 / 排除路径 / 误报申诉通道）。
   代码签名证书（T-403）是根治手段。**更正（2026-11-12）**：开发本机未安装
   卡巴斯基，此前会话「杀软删除产物」系探测路径错误（已更正）；便携包冒烟
   8/8 实证启动链工作正常。
2. **Q-08 PPE 模型按需下载不可用**：`assets.lock.json` 中
   `yolo11n_ppe.onnx`/`.classes.txt` 指向的 `Windy-LabSentinel/assets` release
   实测 404（仓库未建立）——需维护者上传资产或改 URL；两侧下载失败报错已
   指明来源与处置动作。
3. `assets.lock.json` 的 `yolo11n.onnx` 使用 Ultralytics 官方导出（sha256
   `634279b4…`），与本地黄金语料基准 `m0-spike/yolo11n.onnx`（`4758511e…`）
   不是同一文件——差异与恢复计划见 ADR-0009 §4。
4. 完整 1/4/8 路 × 30 分钟压测矩阵（`--duration 1800`）、真实 RTSP 24h 挂机、
   黄金语料 ≥10 段扩展（CC0 素材）、VM 安装验收留档、Flutter/Tauri 客户端：
   均按用户决策暂缓/待人工事项（`docs/plan/status.md`）。
