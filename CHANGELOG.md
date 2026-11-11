# Changelog

本文件记录 LabSentinel 的用户可见变更。格式参照
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)；版本号遵循 SemVer
（决策 77-Q-05：下一版 0.2.0；构建元数据 `+<YYYYMMDD>.<shortsha>`）。

## [Unreleased]

### Rust 控制面深审简化（2026-11-11）
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

### 杀毒软件误报治理（2026-11-11）
- **`LabSentinel.exe` 身份化**：注入程序集属性（产品/版本/公司/描述，版本随包
  单源 0.2.0）与 `/win32manifest` 原生清单（asInvoker——不请求 UAC 提权、
  supportedOS、PerMonitorV2 DPI）。无签名、无版本资源、无清单的裸 winexe
  拉起解释器是杀软启发式的典型画像。
- **启动链审计**：全部随包脚本无 `-c`/`exec`/`runpy` 动态执行；`urlopen` 仅
  回环健康检查；`runtime\python.exe` 来自 python.org 官方签名 embed（SHA256
  锁定）。
- 新增 `docs/杀毒软件误报说明.md`：命中原因对照表、排除路径指引、误报申诉
  通道（opentip.kaspersky.com / Microsoft WDSI）；维护者本机复现注意事项
  （卡巴斯基会静默删除便携 runtime，构建期冒烟不受影响，发布产物 zip 经
  全量 CRC 校验完整）。

### 终止 Docker 支持（2026-11-11，维护者决策）
- 删除 `deploy/docker/`（compose.yaml / Dockerfile / Dockerfile.cuda / Dockerfile.rust），
  不再推进容器化开发——原已知问题（容器内监听回环、go2rtc 配置卷不共享、
  compose 双网络声明，78 号 N-07/N-08/N-19）随产物移除而闭合。
- `docs/部署指南-Linux.md` 改写为 **systemd 原生部署唯一路线**（Docker 路线移除，
  附自行封装指引）；`docs/release-notes-v0.2.0.md` 同步改为「Docker 支持终止」。
- 78 号文档 R2-401 状态由「暂缓」改为「终止」。

## [0.2.0] - 2026-11-11

### 二轮余量收口（2026-11-11，78 号文档 §12 收尾批次）
- **R2-303 GPU 收口**：`/api/system` 透出 onnxruntime `providers`（只读探测，
  B-28 同口径，绝不触发模型加载）；Dashboard 后端卡显示 GPU 加速状态——
  providers 含 `DmlExecutionProvider` → 「GPU 加速：开启」，CPU 变体提示
  「可选 GPU(DML) 变体包随 0.3.0 提供」。DML 实机首测（AMD Radeon 880M 核显 /
  onnxruntime-directml 1.24.4 / `--duration 30 --cameras 1,4`）：单路 **69.43fps**
  （CPU 变体的 1.94×）、4 路 **17.5fps/路**（1.80×），远超 §14.3 DML 门槛
  4.5fps/路；4 路总吞吐与单路几乎持平证明 GPU 未饱和。`tools/perf_baseline.py`
  的实现标注改为按 Engine 实际后端输出（`python-<backend>`），python-dml 行
  仅在缺 DML EP 的环境才出现 n/a；`docs/perf-baseline.md` 已并入实测数据。
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
  `docs/部署指南-Linux.md`（含 §13.7 验收清单）。
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
