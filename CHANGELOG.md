# Changelog

本文件记录 LabSentinel 的用户可见变更。格式参照
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)；版本号遵循 SemVer
（决策 77-Q-05：下一版 0.2.0；构建元数据 `+<YYYYMMDD>.<shortsha>`）。

## [Unreleased]

### 修复
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
