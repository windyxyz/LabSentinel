# Changelog

本文件记录 LabSentinel 的用户可见变更。格式参照
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)；版本号遵循 SemVer
（决策 77-Q-05：下一版 0.2.0；构建元数据 `+<YYYYMMDD>.<shortsha>`）。

## [Unreleased]

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
