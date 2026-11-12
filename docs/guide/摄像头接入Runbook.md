# 摄像头接入 Runbook（现场实施一条龙）

> 面向现场实施人员：从摄像头拆箱到 24h 验收通过的完整操作手册。
> 每一步都给出**预期结果**与**失败先查什么**。配套阅读：
> [部署指南](部署指南.md)（软件安装）、[运维手册](运维手册.md)（日常巡检/排障）、
> [摄像头24h验收清单](摄像头24h验收清单.md)（正式验收表）、[部署FAQ](部署FAQ.md)（常见坑）。
> 本手册只描述操作，不含任何未实测的"已验证"结论。

## 0. 前置条件（缺一项先补齐）

| 项 | 要求 | 不满足的后果 |
|---|---|---|
| 网络 | 摄像头与主机同网段/可路由；主机**有线优先** | 无线抖动 → 画面停滞/误报上涨 |
| 供电 | 摄像头电源/POE 正常（POE 交换机口够） | 掉电=掉线 |
| 账号 | 已知摄像头 IP/账号密码；建议建**只读子账号**（看不到的通道给不了权限） | 管理账号泄露风险 |
| 磁盘 | 主机剩余 ≥ 20GB（服务端另有 5GB 下限预警） | 截图/视频把盘写满触发紧急清理 |
| 时间 | 主机 NTP 同步（`w32tm /query /status` 核对） | 事件时间线错乱、报表按天分组漂移 |
| 软件 | LabSentinel 已部署且 `doctor.py` 10 项 0 FAIL | 先跑 `python scripts\doctor.py` |

## 1. 接入前预检（不进系统也能做）

**1a. RTSP 连通性**（用包内 ffmpeg，完整 Windows 路径可复制）：

```bat
tools\ffmpeg\ffprobe.exe -v error -rtsp_transport tcp -show_entries stream=codec_name,width,height,avg_frame_rate -of default=noprint_wrappers=1 "rtsp://用户名:密码@192.168.1.64:554/Streaming/Channels/102"
```

- 预期：列出 `codec_name=h264 / width=1280(或720) / avg_frame_rate=xx`；
- 失败先查：IP 是否 ping 通 → 账号密码 → 摄像头 Web 后台是否开了 RTSP → 防火墙拦 554。

**1b. 编码与规格判断**：

- `codec_name=h264` ✅ 直接可用；`h265/hevc` ⚠️ 实时画面需 go2rtc 转码（CPU 代价高），
  分析不受影响——优先在摄像头后台把子码流改 H.264；
- **分析用子码流**（720p/15fps 足够，模型输入 640px）：主码流留给本地录像；
- 常见 RTSP 地址模板见 [部署指南](部署指南.md) §（海康 `Channels/102`、大华 `subtype=1`、宇视 `video1`）。

**1c. 一键预检**（服务已起时，替代 ffprobe）：

```bat
python tools\camera_soak.py --preflight --base-url http://127.0.0.1:8000 --camera-id <id>
```

（先在页面把摄像头加好、**不启动**再预检也行；预检只读探测，不产生事件。）

## 2. 添加摄像头（页面操作）

1. 「摄像头」页 → 添加 → 类型 `RTSP 拉流`，source 填**子码流**地址；
2. **取消勾选"自动启动"**——先配区域/规则再启动（否则裸跑全画幅，误报先来一轮）；
3. USB 摄像头：类型选 `USB`，设备下拉直接选（后台 `GET /api/cameras/usb-devices` 枚举，
   被会议软件占用的设备不出现——先关 Zoom/腾讯会议）；
4. 预期结果：列表出现新行；点「启动」后状态 `running`、fps 与子码流标称一致、
   「实时画面」出图；
- 失败先查：状态列红字 `last_error`（401=账号、404/超时=路径、`cannot open`=编码不支持）
  → 完整字段看 `GET /api/cameras/{id}/status`（[口径](api-reference.md) §8.3）。

## 3. 画安全区与规则

1. 编辑摄像头 → 区域：**优先画"人只能从这里进"的门/通道多边形**，别用全画幅
   （全画幅把画面角落路过的人也算进来，误报直接翻倍）；
2. 规则类型语义：
   - `enter`：有人**进入**区域（L2 提示起步）；
   - `enter_no_ppe`：进入且未穿实验服（违规主类型，`LS_PPE_VIOLATION_CLASS` 判定，默认 L3）；
   - `occupancy_over`：区域内人数超限（阈值=规则人数，L3）；
3. 每条规则配：`level`（1 提示/2 警告/3 严重）、`cooldown`（同区同 track 冷却秒数，建议 ≥30）、
   `time_start/time_end`（只在实验室使用时段启用，夜间红外会放大误报）；
4. 多路同布局：用摄像头复制接口一键带过区域+规则（`POST /api/cameras/{id}/copy`，
   见 [35-api-reference](api-reference.md)）；
5. 预期：启动后走一圈，Events 页出现 enter/enter_no_ppe 事件、截图与时间正确。

## 4. 告警验证（先自测后真实）

1. 「摄像头」页 → 告警面板：七渠道配置状态一眼表（企业微信/Webhook/SMTP…）；
2. 点**一键自测**（`POST /api/alert-test`）——每渠道发一条测试消息；
   失败列表点「重发」；反复失败查 §6 故障表"告警不发"；
3. 真实触发：区域内站 30 秒不穿实验服 → 手机/群里收到 L3；
4. 免打扰时段用**静默**（`POST /api/alert-silences`，按摄像头/类型/级别范围），
   未处置 L3 自动升级提醒默认开（`LS_ESCALATE_AFTER_MIN=15`）；
5. 预期：`GET /api/alerts/status` 各渠道 ok 计数增长、failed=0。

## 5. 稳定性观察（1 小时 → 24h）

```bat
:: 先 1 小时观察（对照查看 Dashboard 摄像头卡：frame_age 是否持续 >10s、重连是否上涨）
python tools\camera_soak.py --all --duration 1h --interval 30
:: 通过后正式 24h（夜间无人值守跑）
python tools\camera_soak.py --all --duration 24h --interval 30
```

- 产物：`logs\soak-<时间戳>.csv` + 同名 `.json`（每采样行含 fps/帧龄/重连/状态）；
- 判定阈值（连续 24h）：无 `error` 转 0 次以上 worker 重启、frame_age 无持续 >60s、
  重连次数不持续上涨；**中断恢复**：采样器只读可随时重跑，历史 CSV 不覆盖（新时间戳）；
- worker 意外退出由守护自动重启（任务114，默认开）：状态列 `重试中（第 N 次）` 说明守护在拉；
  若出现 `自动重启失败，请检查`（given_up）= 10 次退避重启用尽，必须查根因；
- 24h 挂机机器：配置开 `LS_KEEP_AWAKE=true`（防自动睡眠，[运维手册](运维手册.md) §12）。

## 6. 故障速查表

| 症状 | 最可能原因 → 处理 |
|---|---|
| 连不上（超时） | 网段不通/防火墙拦 554 → 先 ping，再 ffprobe（§1a） |
| 连不上（401） | 账号错/密码带特殊字符未转义 → URL-encode `@:#?=` 等字符 |
| 连上但无画面 | H.265 子码流 → 摄像头后台改 H.264；或分辨率过高 → 换子码流 |
| 能连但不出事件 | 区域没画/画错位置 → 实时画面叠加确认；规则时间窗没覆盖当前时段；模型类别不匹配（`LS_PPE_VIOLATION_CLASS`，见 [.env.example](../server/.env.example)） |
| 画面卡顿/帧龄涨 | 码率太高 → 摄像头端降子码流码率；多路 DML 负载 → 降低 analysis_fps（`LS_ANALYSIS_FPS`） |
| 误报多 | 阈值/区域/夜红外 → 收紧区域、加 cooldown、夜间缩小规则生效窗、`LS_PPE_CONF` 上调；长期治理走误报池标注（[标注与模型迭代手册](标注与模型迭代手册.md)） |
| 告警不发 | 渠道未配/静默生效/过滤规则（`LS_ALERT_FILTERS`）→ 告警面板自测 + `GET /api/alerts/status` 失败列表 |
| `重试中（第 N 次）` 不消失 | 源一直起不来 → 看状态列红字；达上限转 `自动重启失败，请检查`（[运维手册](运维手册.md) §4.1） |

## 7. 验收清单（正式）

逐项勾选，全过才算 PASS（详细表格与判定口径见
[摄像头24h验收清单](摄像头24h验收清单.md)，误报率取证用「报表」页处置质量卡）：

- [ ] 24h 连续运行，worker 无 given_up（守护自动重启次数记录在案）
- [ ] 事件/截图/视频可查、时间正确
- [ ] 磁盘增长在保留策略预期内（[容量](运维手册.md) §9）
- [ ] 日志无 ERROR 洪峰（`GET /api/diagnostics/summary`）
- [ ] CPU/内存无持续增长（任务管理器观察起止对照）
- [ ] 误报 ≤20 条/天、Lab Coat 召回 ≥60%（模型止损线，用 daily 口径）

## 8. 回滚 / 停用

- 停单路：「摄像头」页点「停止」（守护不拉已手动停止的摄像头）；
- 恢复旧配置：配置导出/导入（`POST /api/config/import`，merge/replace 双模式，
  [35-api-reference](api-reference.md) §10）；导入前先 `GET /api/config/export` 留底；
- 数据备份：`python scripts\backup_data.py`（备份 API 保证 WAL 一致，别直接拷 .db）；
- 卸载服务：便携版「卸载.bat」/ 控制面板卸载（数据目录自动改名备份，不直接删）；
  全流程见 [59-portable-install](安装与卸载.md)。
