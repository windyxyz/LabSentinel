# 部署与运维 FAQ（任务 24）

> 日常运维（巡检/备份/升级/告警排障）请看 **[运维手册](运维手册.md)**；本文按主题收录踩坑问答。
> 全部条目来自本项目开发过程中**实际遇到并解决过**的问题（2026-10-25/27）。
> 格式：**现象 → 原因 → 解决**。查不到 here 的问题先看 [80-risk-register.md](80-risk-register.md)。

## 1. 安装与网络

**Q：pip 安装极慢/断流**
→ 国内直连 PyPI 不稳。解决：一律加清华源
```bash
pip install <pkg> -i https://pypi.tuna.tsinghua.edu.cn/simple
```

**Q：GitHub 下载（模型/二进制）失败或 SSL 错误**
→ 直连不稳。解决（按优先级尝试）：
```bash
# ① 镜像前缀 + 断点续传循环
for i in 1 2 3 4 5; do curl -sL --max-time 120 -C - -o <文件> "https://ghfast.top/https://github.com/<路径>" && break; done
# ② 部分包走官方 CDN（如 OpenH264 用 ciscobinary.openh264.org）
# ③ GitHub API（api.github.com）通常可达，可先查资产真实 URL
```

**Q：npm 安装慢**
```bash
npm install --registry=https://registry.npmmirror.com
```

**Q：Roboflow 下载/上传连不上**
→ 需要代理且注意参数名。解决：`HTTPS_PROXY=http://127.0.0.1:7890`（本地代理不一定在线，先探测）；
Roboflow API 的 key 参数名是 **`api_key`**（不是 key/token）。key 一律走 `RF_API_KEY` 环境变量，禁止写死。

**Q：HF（HuggingFace）不可达**
→ 用 hf-mirror 镜像：`export HF_ENDPOINT=https://hf-mirror.com`

## 2. 依赖冲突（最高频事故区）

**Q：推理报错/onnxruntime 提供者消失**
→ **onnxruntime（CPU 版）与 onnxruntime-directml 同装互杀**（ultralytics 的 AutoUpdate 也会偷偷装 CPU 版！）。
解决（两个一起卸再装回）：
```bash
pip uninstall -y onnxruntime onnxruntime-directml
pip install onnxruntime-directml==1.24.4 -i https://pypi.tuna.tsinghua.edu.cn/simple
```
预防：`tools/export_onnx.py` 已内置 `YOLO_AUTOINSTALL=false`；npu-venv 里 CPU 版与 vitisai 同理互杀。

**Q：numpy 报二进制不兼容 / import 崩溃**
→ numpy 1.x/2.x wheel 按版本编译，混装即崩。解决：m0-venv **numpy 不许动**；npu-venv 钉 **numpy 1.26.4**（AMD wheel 按 1.x 编译）、pandas 钉 **2.2.3**。

**Q：ByteTrack 报错/找不到跟踪器**
→ **supervision 必须 0.25.1**（0.26+ 把跟踪器移出主包）：
```bash
pip install supervision==0.25.1 -i https://pypi.tuna.tsinghua.edu.cn/simple
```

**Q：pytorch.org 下载 torch 超时**
→ pytorch 官方源本网络不可达，直接用 PyPI 的 CPU 版 torch（清华源）。

**Q：pip 安装报 WinError 5 拒绝访问（onnxruntime DLL）**
→ 运行中的 uvicorn/训练进程锁着 DLL。解决：先停服务（scripts\stop-all.bat）再装；
纯 Python 包（如 python-multipart）无 DLL 锁风险可例外。

## 3. 运行与端口

**Q：端口被占用启动失败**
→ `scripts\start-all.bat` 会自动检测 1984/8000/5173 并**跳过**被占用组件；手工排查：
```bash
netstat -ano | findstr ":8000" | findstr "LISTENING"   # 最后一列是 PID
taskkill /PID <pid> /F
```
或直接 `scripts\stop-all.bat`（带确认）。

**Q：vite 起了但 curl http://127.0.0.1:5173 不通**
→ vite dev **仅绑 IPv6 localhost**。解决：`curl -g "http://[::1]:5173"`；浏览器用 http://localhost:5173 无碍。

**Q：Git Bash 的 curl 发中文 JSON 后服务端收到乱码**
→ Git Bash 转码损坏。解决：测试 API 一律用 python httpx 写脚本。

**Q：浏览器打开事件视频无法播放**
→ 缺 OpenH264 编码库（服务器端生成时回退了 mp4v）。解决：下载 `openh264-2.5.0-win64.dll`
（Cisco 官方 CDN：`http://ciscobinary.openh264.org/openh264-2.5.0-win64.dll.bz2`，解压得 dll）放入
**服务端 venv** 的 `Lib\site-packages\cv2\`（项目内 `.venv`；`deploy.py install` 会自动下载放置；
便携包自带无需处理）——`server/app/__init__.py` 已自动把该目录加进 DLL 搜索路径。

**Q：USB 摄像头索引变了 / 添加页面"未发现设备"**
→ 索引是 DirectShow 枚举顺序，**拔插顺序、驱动/系统更新后可能变化（设备名更稳定）**。处理顺序：
1. 「添加摄像头」选 USB 类型 → 点「刷新设备」重新枚举（接口有 10 秒缓存，`?refresh=1` 强制刷新）；
2. 仍无设备：检查 USB 线/隐私挡板，**关掉正在占用摄像头的会议软件**（Zoom/腾讯会议独占时枚举不到）；
3. OBS Virtual Camera 未启动 OBS 时不会出现——先开 OBS 并启动虚拟摄像头；
4. 以上都不行：回退手动输入索引（输入框仍在），用 `python tools\list_cameras.py` 确认可用索引。

**Q：摄像头显示 error / 画面停滞，怎么看问题在哪？**
→ 「摄像头」页状态列现在有可观测信息（任务65）：红字 `last_error` 是最近一次失败的原因
（RTSP 账号密码已脱敏）；`画面停滞 Ns` 表示 running 但超过 10 秒没读到新帧；`重连 N 次` 是
断流后自动重连的累计。排查顺序：
1. `error` + "打开失败"：RTSP 地址/账号错（401/404）、USB 索引变了、文件路径不存在——按提示修；
   rtsp/usb 会自动重试 5 次（间隔 3s），file 源直接报错退出；
2. `画面停滞`：网络抖动（子码流带宽）、摄像头端掉线、被会议软件抢占——看 `重连 N 次` 是否在涨；
   重连成功会自动恢复 running 并清掉 last_error；
3. `analyze:` 开头的错误是推理侧瞬时异常（单帧丢弃自愈，不影响状态）；频繁出现才需要查日志；
4. 仍不明确：看 `GET /api/cameras/{id}/status` 的完整字段 + `server.log` 按 request_id/时间检索。

**Q：AI 复核（VLM 误报预分类）怎么开？会自动改处置状态吗？**
→ **不会**。任务124 的 AI 复核只给事件打"疑似误报/疑似真实/不确定"标记（事件详情
`AI：…（仅供参考）`徽标），**绝不自动改 status、不抑制告警**；误报率仍以人工处置为准。
开启前提：本机跑 Ollama 等 OpenAI 兼容视觉模型服务，`LS_GENAI_MODEL` 填模型名后重启
（`LS_GENAI_PROVIDER=ollama|openai_compat`）。安全边界：截图默认只发回环地址（非回环需
`LS_GENAI_ALLOW_REMOTE=1`）；失败静默不影响事件/告警；单并发（同机 GPU 与检测推理竞争，
**24h 验收/压测时建议整体关闭**）。状态看 `GET /api/genai/status`。

**Q：怎么配"有新版本"提示？为什么不在线升级？**
→ 自建一个静态 JSON 清单（任意 https 静态托管；自建局域网 http 也可，需
`LS_UPDATE_ALLOW_HTTP=1`），格式：
`{"version":"2026.11","url":"下载页地址","sha256":"…","notes":"…","channel":"stable"}`，
然后 `LS_UPDATE_MANIFEST_URL` 指向它。仪表盘「版本」卡会显示"发现新版本"+查看链接；
比较口径是**字符串不等即提示**（不做 semver）。结果缓存 1h，「检查更新」强制刷新。
**不做在线升级的原因**：更新包含模型与运行时（数百 MB）、签名未定、失败会变砖——
升级走"手动下载新 zip → 备份 server/data（`scriptsackup_data.py`）→ 按部署指南覆盖
（数据目录外置不受影响）"，面板提示只负责"该去看了"。

**Q：怎么把面板装成应用 / 收桌面通知？**
→ **装成应用（PWA，任务115）**：用 Chrome/Edge 打开面板 → 地址栏右侧"安装"图标（或菜单
"安装应用"）→ 独立窗口、无地址栏、任务栏/开始菜单归属。**注意：Service Worker 只在
localhost 或 HTTPS 下生效**——局域网 IP 的 HTTP 访问装不了，属浏览器安全策略。
离线行为：仅缓存界面壳（index/assets），事件/截图/实时流**绝不缓存**，断网刷新出壳+错误提示。
**桌面通知**：仪表盘「桌面通知」卡 → 点"开启桌面通知"授权 → 之后页面在后台时 L2/L3 事件
弹系统通知（同摄像头同类型 10s 去重；L1 不打扰；勾选可暂停，localStorage 记忆）。

**Q：怎么把一套规则复用到多个实验室/新装环境？**
→ 「安全区配置」页**规则模板**（任务116）：把当前规则表单存为命名模板（含星期/假期/时间窗），
换实验室时选模板→选套用目标（全局或某区域）→"套用到当前摄像头"——已存在同类型同时间窗的
规则自动跳过，不会重复。与「复制到其他摄像头」（任务80，即时复制）并存：换环境用模板、
同环境加路用复制。

**Q：发现助手搜不到摄像头怎么办？**
→ 逐项查：①摄像头与主机**同网段**（跨 VLAN/路由不转发组播）；②摄像头 Web 后台开启
**ONVIF**（海康："高级设置→集成协议"，大华/宇视类似）并允许管理员账号 ONVIF 接入；
③主机防火墙放行 **UDP 3702** 入站/出站；④Wi-Fi 开了 **AP 隔离**会拦设备间互访；
⑤服务端只保留私网段结果（10/172.16/192.168/169.254），跨网段设备请手输 RTSP
（部署指南 §3.1 品牌模板）。「探测」按钮可单独验证某 IP 的 554 口与编码。

**Q：多路摄像头怎么按楼层/区域分组管理？**
→ 「摄像头」页顶部「分组」工具条（任务111）：添加摄像头时填"分组"（或事后用 PUT
`/api/cameras/{id}/group` 设置，空=未分组）；筛选下拉按组查看，"启动本组/停止本组"一键
整组操作；勾选多路后"批量启动/批量停止"（一次 ≤100 路）。分组只是标签，不改变单路启停语义。

**Q：怎么设置规则只在夜间/周末/假期告警？**
→ 三层条件（任务112）都在「安全区」页规则表单：①时间段 `time_start—time_end`（支持跨午夜如
22:00—06:00）；②**生效星期**多选（周一只勾周一…"仅工作日"就勾一~五）；③**仅假期生效**勾选
（配合页面底部「日期例外」表：把法定假期标成 holiday、周末补班日标成 workday；
无例外记录时周末默认按假期，`LS_WEEKEND_IS_HOLIDAY=0` 可关闭）。
判定顺序：星期 → 假期 → 时间窗 → 冷却；`occupancy_over` 全局规则暂不受时间窗/日历限制。

**Q：摄像头 worker 退出了会自动恢复吗？重试多少次？**
→ 会（任务114 守护，默认开启）：worker 线程退出（重连耗尽/未捕获异常）后，守护线程每
`LS_SUPERVISOR_INTERVAL_S=10` 秒巡检一次，对"应该是运行中"的摄像头自动重启，退避
1/5/30/60s，连续失败默认最多 `LS_SUPERVISOR_MAX_RESTARTS=10` 次——存活超过 5 秒即算成功、
计数清零。状态列会显示 `重试中（第 N 次）`；达到上限显示 `自动重启失败，请检查`，
摄像头转 error 并停止该路守护（手动点「启动」可重新纳入守护）。`LS_SUPERVISOR_ENABLED=0`
或 `LS_SUPERVISOR_MAX_RESTARTS=0` 关闭守护（worker 退出需手动启动）。

**Q：数据库目录出现 `-wal`/`-shm` 文件正常吗？`database is locked` 怎么调？**
→ **正常，不要手删**（任务64 起默认 WAL 模式）：`labsentinel.db-wal` 是预写日志、`-shm` 是共享
索引，正常关闭服务会自动 checkpoint 合并回主库。备份**必须用 backup API**
（`scripts/backup_data.py`，任务62）——只拷 `.db` 会丢 WAL 里未合并的数据。

`database is locked`：WAL + `busy_timeout`（默认等 5000ms）已消除绝大多数场景；若仍出现
（如外部工具长期占写锁），调大 `LS_SQLITE_BUSY_TIMEOUT_MS`，或排查是否有第三方 SQLite 客户端
（DB Browser 等）开着同一库。

**Q：怎么判断服务是否就绪？报错里的 request_id 怎么查日志？**
→ 就绪/存活分开看：`GET /api/health/live` 200 = 进程活着；`GET /api/health/ready` 200 = DB/数据目录/
模型文件/磁盘（≥1GB，`LS_READY_MIN_FREE_GB` 可调）全部就绪，503 时 `checks` 里逐项给出原因。
前端报错弹出的 `request_id`（响应头 `X-Request-ID`，错误体同名字段）：

```bash
grep "[req=<那个id>]" server/data/logs/server.log     # 该请求的完整堆栈就在这行下面
```

**Q：如何备份 / 迁移数据？恢复前注意什么？**
→ 用任务 62 的 CLI（**运行中也可直接备份**——SQLite backup API 保证一致快照，不需要停服）：

```bash
python scripts/backup_data.py backup --data-dir server/data --out backup-20261030.zip
python scripts/backup_data.py verify --zip backup-20261030.zip      # 校验 manifest/sha256/integrity
python scripts/backup_data.py restore --zip backup-20261030.zip --data-dir <新机数据目录> --force
```

- 便携版 `--data-dir` 填 `%LOCALAPPDATA%\LabSentinel\data`；`--no-media` 只备 db（截图/视频不含）；
- 恢复是**保守**的：目标已有 db 时必须 `--force`（现有 db 自动转存 `.bak-<时间戳>`，保留最近 3 个），
  且不删除目标中备份没有的文件；
- 换机迁移：新机装好同版本 → restore → 启动即可（数据库 + 截图/视频一起迁移）。

**Q：日志在哪 / 怎么生成诊断包 / 轮转策略**
→ 源码模式：`server/data/logs/server.log`（服务端，5MB×4 份轮转）+ `logs/server-<日期>.log`
（uvicorn 控制台重定向）；便携模式：`%LOCALAPPDATA%\LabSentinel\logs\`。级别可用 `LS_LOG_LEVEL`
调（默认 INFO），`LS_LOG_FILE=0` 关闭文件日志。报障一键打包（脱敏，不含截图/视频/密钥/整库）：

```bash
python scripts/collect_diagnostics.py --out diag.zip --data-dir server/data
```

运行中实例的聚合状态另见 `GET /api/diagnostics/summary`（版本/后端/运行时长/各表计数/
磁盘剩余/最近 20 条 ERROR）。

## 4. 模型与推理

**Q：多路摄像头时 worker 全部静默死亡，甚至整机死机**
→ **DirectML 会话跨线程并发 Run 不线程安全**（本项目最大事故，详见 51-loadtest-report.md §3）。
已修复：`engine.py` 模块级 `_ACCEL_LOCK` 全局串行 + worker 单帧异常自愈。
规则：**任何推理必须走 `Engine._run()`，代码出现 `sess.run` 即不合格**；训练/压测/批量推理禁止并行。

**Q：NPU 推理怎么弄**
→ XINT8 量化（检测头保留 FP32，`/model.23` 子图排除——全图量化类别输出会归零）；
本机无 MSVC 需走 `tools/quantize_npu.py` 内置的"注入绕道"；配方细节见 tools/README.md 第二节。

**Q：PPE 模型换了之后检测类别不对**
→ 词表顺序必须与模型输出一致。PpeEngine 优先读模型同名 `<model>.classes.txt`（一行一类），
没有才回退 `LS_PPE_CLASS_NAMES`。换模型：导出 ONNX → 生成 classes.txt → 改 `.env` 的 `LS_PPE_MODEL` → 重启。

**Q：白大褂判定不准/误报**
→ 采用**证据分级**：No Lab Coat 显式检测（conf≥0.30）才算违规；Lab Coat 低阈值（0.20）作合规证据；
两者皆无 = "不确定"（不告警）。阈值可调 `LS_PPE_CONF`/`LS_PPE_WEARING_CONF`。持续不准走数据飞轮（docs/90）。

## 5. 训练

**Q：训练命令与安全**
→ 限线程+过夜，完整前置条件与看门狗见 [50-tasks/训练安全规程.md](50-tasks/训练安全规程.md)。速记：
```bash
D:/LabSentinel/m0-venv/Scripts/python.exe tools/train_ppe.py --data Resources/datasets/merged/data.yaml \
    --epochs 24 --batch 8 --workers 2 --name ppe_v2
```
本机在高负载组合下（CPU 满核+GPU 推理+虚拟显示器/剪映）**发生过三次整机硬挂起**
（Kernel-Power 41，DML 并发已修复但硬件侧建议照做：更新 BIOS/驱动、停用 Parsec/GameViewer 虚拟显示器）。

**Q：训练中断了怎么办**
→ 看项目根 `.training.lock`（记录 PID）与 `runs/detect/<name>/weights/last.pt` 是否更新；
规程约定**不自动重启**，人工确认后可 `--exist-ok` 续跑或重跑。

**Q：GPU 训练可行吗**
→ 可选路线：`D:\LabSentinel\gpu-venv`（Python 3.12 + torch-directml 0.2.5）+ `--device dml`。
注意 torch-directml 只发到 cp310/cp311 wheel；实测加速比与稳定性见 tools/README.md 第八节。

## 6. 数据与标注

**Q：Roboflow 导出的 YOLO 标签一行几十个数字**
→ 那是**分割多边形格式**，不是检测框（本项目实测 4/5 的框因此被静默丢弃）。解析代码必须兼容：
>5 字段且偶数个 → 按 x/y 序列取外接矩形。`tools/eval_report.py` 的 `load_gt` 可参考。

**Q：标注规范在哪**
→ [90-data-flywheel.md](90-data-flywheel.md) 第二节（Lab Coat/No Lab Coat 语义、遮挡阈值、歧义帧处理）。
数据集许可 CC BY 4.0；对外发布前必须过 `tools/export_dataset.py` 生成的脱敏确认清单。

## 7. 告警与推送

**Q：各渠道 key 从哪拿**
→ 网页端「告警配置」页每个渠道都有「配置指引」（企微/钉钉/飞书/Server酱/PushPlus/Qmsg/SMTP 逐个说明）；
也可看 [部署指南](部署指南.md)「4. 告警渠道配置」的 `.env` 口径。

**Q：网页保存的配置存在哪/怎么回退**
→ 存在本机 `data/labsentinel.db` 的 `app_settings`，随数据库备份；保存即生效、无需重启。
密钥不回显；输入框留空保存=保持原值；「清除」=回退到启动时 `.env`。

**Q：为什么发不出/被拒绝**
→ 安全机制：域名白名单 + 强制 https + 不跟随重定向（防 SSRF）。钉钉/企微 HTTP 200 但
body `errcode≠0` 也判失败（`_is_ok`）。失败会重试（1s/4s/15s 三次）→ 死信 →「摄像头」页告警面板手动重发。

**Q：怎么验证渠道通了**
→ 「告警配置」页每个渠道的「发送测试」（单渠道），或「摄像头」页「发送测试告警」/`POST /api/alert-test`（不落 events 表）。

## 8. 前端

**Q：bundle 体积预算**
→ 项目体积敏感，禁止引 ECharts 等重型库；图表全部原生 SVG（Reports.vue 可参考）。
当前 gzip 约 42KB。

**Q：主题令牌在哪/我能改全局样式吗**
→ WinUI 3 令牌在 `web/src/theme.css`（浅/深双主题）；**该文件与 style.css/App.vue/main.js 为主线专属**，
其他协作者新增样式一律写组件 `<style scoped>`，令牌（--card-bg/--accent/--l1~l3/--stroke-*）直接复用。

**Q：写接口 401**
→ 服务端启用了管理鉴权（LS_ADMIN_TOKEN）。「摄像头」页顶部填写令牌（存浏览器 localStorage），
所有 POST/DELETE 自动携带。

**Q：能嵌到别的系统里吗（iframe）？CSP 怎么开？**
→ 默认 `X-Frame-Options: DENY`（任务89）——嵌入大屏改 `LS_FRAME_OPTIONS=SAMEORIGIN`。
CSP 默认不发送（避免页面白屏难排查）；配置 `LS_CSP=default-src 'self'; connect-src 'self' ws: wss:`
后发送，首次建议 `LS_CSP_REPORT_ONLY=1`（只报告不阻断，观察控制台无违例后再去掉）。

**Q：怎么查谁删了摄像头/改了规则？**
→ 报表页 →「操作审计」折叠区（或 `GET /api/audit`，仅管理员）：所有 `/api/*` 写操作自动记录
操作者（admin/anonymous）、动作、路径目标、状态码与 request_id（可据此查 server.log 完整上下文）。
审计不记录请求体（防密码/令牌入日志）；审计行按 `LS_RETENTION_EVENTS_DAYS` 同口径自动清理。

**Q：怎么让某渠道只收 L3？群机器人只想收"未穿实验服"？**
→ `LS_ALERT_FILTERS`（任务86，JSON）：`{"email":{"min_level":3},"wecom":{"types":["enter_no_ppe"]}}`。
`min_level` 1–3；`types` 留空/缺省=不限制。被过滤的事件不建投递行；`/api/alerts/status` 每渠道
回显 filters 便于核对；`alert-test` 不受过滤（确认连通性用）。

**Q：怎么改告警文案？支持哪些占位符？**
→ `.env`/`config.env` 的 `LS_ALERT_TEMPLATE_TITLE` / `LS_ALERT_TEMPLATE_TEXT`（任务85），
改后重启生效。占位符：`{icon} {level} {type} {type_text} {ts} {camera_id} {track_id} {detail}
{track_part}`（track_part 为" · track N"或空）；未知占位符原样保留；`detail` 里的花括号不会被
二次解析。改前可在「摄像头」页 → 告警文案模板 → 预览确认效果。

**Q：严重事件没人处理会怎样？怎么调提醒间隔？**
→ 未处置升级提醒（任务82）：L3（默认）事件超过 `LS_ESCALATE_AFTER_MIN`（默认 15 分钟）仍为
new 状态 → 通过现有渠道**再提醒一次**（文案带 ⏰ 前缀），每事件只提醒一次（不刷屏）；
`LS_ESCALATE_AFTER_MIN=0` 关闭；`LS_ESCALATE_MIN_LEVEL` 可设 2 连 L2 一起提醒。
事件记录/截图照常，提醒不等于处置。

**Q：维护/清洁时怎么临时不告警？事件还会记录吗？**
→ 「摄像头」页告警面板 → 告警静默（任务81）：可全局/按摄像头/按级别/按类型静默 1h–7d，
到期自动恢复，重启不丢。**事件照常入库、截图照常、WS 实时流照常**——只是不推送外部渠道
（不建投递行、不计失败）。维护完在列表点「解除」立即恢复。别用停摄像头代替静默：那会丢检测。

**Q：多路摄像头怎么快速套用同一套区域/规则？**
→ 「安全区配置」页 →「复制到其他摄像头」（任务80）：勾选目标摄像头 → 复制区域/规则 →
规则按区域名映射到各目标（不会指向源摄像头的区域），同名区域跳过。批量部署的另一种方式是
配置导出/导入（上一条）。

**Q：如何批量部署多路摄像头？换机怎么迁移配置？导入会覆盖事件吗？**
→ 「摄像头」页 → 配置导入导出（任务79）：先在一路上调好区域/规则 → 导出 JSON（默认脱敏
RTSP 密码）→ 改文件里的摄像头名/地址 → 导入（merge 跳过同名，replace 需勾选确认）。
换机迁移：新机装好 → 导入配置 JSON → 再用任务 62 的 backup zip 恢复事件数据。
**导入不会动事件表**；replace 模式事件保留但会失去对应摄像头（重新导入同名摄像头即恢复）。

**Q：能装到 D 盘吗？公司批量部署怎么做？覆盖升级会丢数据吗？**
→ 可以：`install --source . --install-dir "D:\Apps\LabSentinel"`（任务71，默认仍是
%LOCALAPPDATA%\Programs\LabSentinel）。批量部署：`--silent --no-shortcuts --dry-run` 预检后
`--silent` 静默安装（可用 `--install-dir` 统一路径、`--autostart` 开自启）；卸载按
`--install-dir` 定位。**覆盖升级不会丢数据**：程序文件覆盖、`%LOCALAPPDATA%\LabSentinel`
（数据库/截图/配置）原样保留——升级前按习惯可先用任务 62 的 backup 命令做一份快照。
卸载/重装默认都保留数据，只有显式 `--purge-data` 才删。

**Q：升级后页面还是旧的怎么办？**
→ 单端口模式已按资源类型区分缓存（任务72）：`/assets/*`（Vite 内容 hash 文件名）缓存一年
（immutable）——内容一变文件名就变，不存在旧缓存问题；`index.html` 与前端路由回退响应是
**no-cache**（每次回源校验 ETag），升级后刷新即加载新资源。若浏览器仍显示旧页：强刷
（Ctrl+F5）或清缓存即可；极少数情况是浏览器拿不到新 index.html（代理层缓存），检查中间层
是否吞掉了 no-cache 头。API 响应不带缓存头（行为不变）。

**Q：doctor 自检输出怎么读？**
→ `python scripts/doctor.py`（便携包用 `runtime\python.exe scripts\doctor.py`）输出 10 项
PASS/WARN/FAIL：**FAIL 必须处理**（缺模型/程序文件/数据库损坏/写权限），WARN 可选（缺 ffmpeg/
go2rtc 影响实时画面、端口占用可能是旧实例在跑、磁盘低）。每项下面的 `↳` 是修复建议；
`--json` 供支持脚本采集；退出码：有 FAIL=2，否则 0。全程只读，不会改动系统。

**Q：升级后数据库会自动迁移吗？迁移失败怎么办？**
→ 会（任务69）：程序首次启动检测 `schema_version` 表，自动执行未应用的迁移（只做加列/加表/索引，
不做破坏性变更；无版本记录的旧库若结构已最新会自动标记）。`/api/system` 的 `schema_version` 字段
可查当前库版本。迁移失败时启动会报「数据库迁移 vN 失败」——**先备份**（用任务 62 的 backup 命令
或直接拷贝 data 目录），然后带着该错误反馈；不要手工改库结构。

**Q：清理前怎么预演？磁盘预警阈值怎么调？**
→ 「报表/维护」之外有只读预演端点：`GET /api/maintenance/preview` 返回**将会**删除的事件数、
文件字节与预计释放空间，不删任何东西；确认后再 POST `/api/maintenance/cleanup` 执行。
磁盘预警：仪表盘「磁盘」卡显示剩余/总量；`low_disk`（剩余 < `LS_DISK_MIN_FREE_GB`，默认 5GB）时
卡片变红提示将触发紧急清理；「按当前增速约 N 天」为近 7 天日均增速的**估算值**（数据不足不显示）。

**Q：自建服务接通用 Webhook 怎么验签？为什么 http 被拒绝？**
→ 配置 `LS_WEBHOOK_URL`（+可选 `LS_WEBHOOK_SECRET`）后，告警会 POST JSON 到该地址；
签名对**原始请求体字节**计算 `hex(hmac_sha256(secret, body))`，放 `X-LS-Signature: sha256=<hex>`——
验签必须读原始字节（先 `request.body` 再 json 解析，不要框架帮你重序列化），用 `hmac.compare_digest`
防时序攻击。http 被拒是**默认安全策略**（明文会泄露告警内容与签名）；内网自建服务确认风险后设
`LS_WEBHOOK_ALLOW_HTTP=true` 放行。带 `user:pass@` 的 URL 一律拒绝（凭证不入 URL/日志）。

## 9. 新机部署脚本（2026-10-27 修复）

**Q：新电脑双击 `一键启动.bat` 提示"python 不是内部或外部命令"**
→ 旧版直接调 `python`。现改为先试 `py -3` 再试 `python`，都没有时给出安装指引；同时不再自动 UAC 提权
（提权会切换到管理员环境，新机上反而可能丢掉用户 PATH 里的 Python）。

**Q：首次部署卡在 OpenH264 下载失败，后面 Node/ffmpeg/配置全没执行**
→ 旧版把可选组件当致命错误，且三个源都不可用（npmmirror 无 openh264 目录、Cisco 裸 `.dll` 返回 403、
GitHub 直连不稳）。现改为：先扫本机已有 DLL 直接复用；下载只认 Cisco 官方 `.bz2`（自动解压）+ GitHub
加速镜像兜底；仍失败只警告——事件视频会回退系统 H.264（MSMF）/ mp4v，不影响分析与截图。

**Q：实时画面打不开，go2rtc 日志报 `did not find expected hexdecimal number`**
→ deploy 写 yaml 时 Windows 路径带反斜杠，YAML 双引号串把它当转义。现统一转 `/`；重跑部署会重写
`tools/go2rtc.yaml`。

**Q：本机装了 Node，但 Web 面板起不来**
→ 便携版 Node 解压后只有 deploy 进程的 PATH 知道，`start-all.bat` 新开的 cmd 找不到 `npm`。现在会先扫
本机 Node（Program Files / 用户目录 / nvm），用目录联接挂到 `tools/node`；下载的便携版也会把
`tools/node` 加入 Web 启动的 PATH。

**Q：venv 路径乱码 / start-all 找不到解释器**
→ `scripts/venv_path.txt` 现在写相对路径 `.venv\Scripts\python.exe`（纯 ASCII）；start-all 优先项目内
`.venv`，兼容旧绝对路径（GBK）与 `D:\LabSentinel\m0-venv`。

**Q：启动器等待超时，或浏览器打开是空白/404**
→ 旧版只等服务端 8000 就打开面板，Vite 还没就绪。现在要等 API + Web 都就绪（最多 60 秒），Web 检测
同时探 IPv6 与 IPv4。

**Q：下载策略到底是什么**
→ 先扫本机（venv / Node / ffmpeg / OpenH264），找到就不下载；确实缺才联网：pip 清华 → 阿里云 → 官方，
npm npmmirror → 官方 registry，Node npmmirror → nodejs.org，ffmpeg/ffprobe 首选 npmmirror CDN。
