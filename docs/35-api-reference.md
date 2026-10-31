# 35 · API 契约（服务端 ⇄ 客户端）

> v0.1（2026-10-26）· 依据**当前真实实现**（server/app/api.py）整理。前后端/子代理开发以本文为准；改 API 必须同步更新本文。
> Base URL：`http://127.0.0.1:8000`（Vite 开发代理 `/api`、`/ws`）。无鉴权（局域网内网部署，M3 加管理密码）。

## 通用约定

> **鉴权（任务16）**：`LS_ADMIN_TOKEN` 非空时，全部 POST/DELETE 端点要求请求头 `X-Admin-Token`（401 否则）；
> `/api/system` 永不拦截；`LS_AUTH_READ_PROTECTED=true` 时 events/alert-deliveries/face persons 等读端点同样要求；
> WS `/ws/events` 用 `?token=`。下文各写端点默认带 `⚡写保护` 标记。

- 请求/响应 JSON，UTF-8；时间 `ts` 为 ISO 字符串（`2026-10-26T08:30:00`）
- 错误：HTTP 4xx/5xx + `{"detail": "..."}`（FastAPI 默认）
- 摄像头画面统一缩放为 **1280×720**，安全区坐标为该坐标系像素

## 1. 系统

### GET /api/system
```json
{
  "version": "0.1.0-m1",
  "backend": "directml",                     // directml | vitisai | cpu
  "cameras": [{"id": 1, "fps": 21.3, "status": "running"}],
  "today_counts": {"1": 3, "2": 7, "3": 2}   // 今日按级别事件数
}
```

## 2. 摄像头

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | /api/cameras | 列表 → `Camera[]` |
| POST | /api/cameras | 创建，body `CameraCreate` |
| POST | /api/cameras/{id}/start | 手动启动（配合 autostart=false） |
| DELETE | /api/cameras/{id} | 删除（连带安全区/规则） |
| GET | /api/cameras/{id}/snapshot | 最新帧 JPEG（无帧时 404） |
| GET | /api/cameras/{id}/webrtc | 实时画面信令地址 |

`Camera`: `{id, name, source, type: "rtsp"|"file"|"usb", status: "running"|"stopped"|"error"}`
`CameraCreate`: `{name, source, type="file", autostart=true}`

webrtc 响应：`{"camera_id": 1, "ws_url": "ws://<host>:1984/api/ws?src=1"}`（浏览器连 go2rtc 信令；RTSP 地址由 go2rtc 拉流，不额外占用摄像头连接）

## 3. 安全区

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | /api/zones?camera_id= | `Zone[]`（camera_id 可省略=全部） |
| POST | /api/zones | body `ZoneCreate`，顶点 <3 返回 400 |
| DELETE | /api/zones/{id} | 删除 |

`Zone`: `{id, camera_id, name, polygon: [[x,y],...]}`（1280×720 像素坐标）

## 4. 规则

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | /api/rules?camera_id= | `Rule[]` |
| POST | /api/rules | body `RuleCreate` |
| DELETE | /api/rules/{id} | 删除 |

`Rule`: `{id, camera_id, zone_id|null, behavior, level, cooldown_s, time_start, time_end, threshold, enabled}`
- `behavior`：`enter` | `enter_no_ppe` | `exit` | `occupancy_over`
- `level`：1 提示 / 2 警告 / 3 严重
- `time_start/end`：`"HH:MM"`，支持跨午夜（end<start 视为跨天）
- `threshold`：仅 occupancy_over 用（全局规则 zone_id=null，track 记 0）

## 5. 事件

### GET /api/events?limit=50&camera_id=&level=&status=&offset=0&start_ts=&end_ts=

任务19：`offset` 分页、`start_ts/end_ts` 时间范围（纯日期自动补 T00:00:00/T23:59:59，含边界）；
`limit` 钳制 1-200。

### GET /api/events/count?camera_id=&level=&status=&start_ts=&end_ts=

同过滤条件的总数 `{"count": n}`（分页计算用；不消耗 limit）。
返回 `Event[]`（id 倒序）。`Event`：
```json
{
  "id": 13, "ts": "2026-10-26T08:30:00", "camera_id": 1,
  "type": "enter_no_ppe", "level": 3, "track_id": 7,
  "detail": "进入安全区「reagent」未穿实验服，人员：张三",
  "snapshot": "server/data/snapshots/ev_...jpg",  // 内部路径
  "video": "server/data/videos/ev_13.mp4",
  "has_snapshot": true, "has_video": true,
  "status": "new",  "handled_by": null, "handled_ts": null   // 任务12 事件闭环
}
```
`type` 中文映射见 `web/src/api.js` 的 TYPE_TEXT 与 `server/app/constants.py`。

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | /api/events/{id}/snapshot | 事件截图 JPEG |
| GET | /api/events/{id}/video | 事件短视频（H264 优先；缺 DLL 时 mp4v） |
| POST | /api/events/{id}/status | 事件处置（任务12）：body `{"status":"acked / false_positive / ignored","handled_by":可选}`；误报时截图自动复制到 data/annotation_pool/；非法值 400 |

### 5.1 告警投递（任务13）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | /api/alert-deliveries?status=&limit=50 | 投递记录（pending/ok/failed；failed=死信） |
| POST | /api/alert-deliveries/{id}/retry | 死信手动重发（重新走 1s/4s/15s 三次退避） |
| POST | /api/alert-test | 渠道健康自检：向全部已配置渠道发测试消息，返回逐渠道 ok/error |

`GET /api/system` 新增字段：`unhandled_today`（今日未处置数）、`alerts: {pending, failed, ok}`（投递计数）。

### GET /api/export/evidence.zip?camera_id=&level=&status=&start_ts=&end_ts=（任务33）

事件证据包 ZIP：`events.csv`（UTF-8 BOM，列同任务15）+ `snapshots/` + `videos/`（存在才打包）+
`README.txt`（生成信息与合规提示）。上限：事件 500 条、文件总量 200MB（超限 400 并提示缩小范围）；
只打包 data_dir 内文件（防穿越）。文件名 `labsentinel_evidence_<时间戳>.zip`。

### GET /api/alerts/status（任务23）

`{"channels":[{"name":"wecom|dingtalk|feishu|serverchan|pushplus|qmsg|email","configured":bool,
"deliveries":{"ok":n,"failed":n,"pending":n}}],"test_supported":true}`（只读；configured 不含敏感值）。
Web 端「摄像头」页内告警面板：状态表 + POST /api/alert-test 一键自测 + 失败投递列表与重发。

### 5.2 导出（任务15）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | /api/export/events.csv?days=30&camera_id=&status= | 事件 CSV（UTF-8 BOM，流式；列：时间,摄像头,类型,级别,track,详情,状态,处理人,处理时间） |
| GET | /api/export/stats.csv?days=30 | 每日统计 CSV（日期,合计,L1,L2,L3） |
| GET | /api/export/monthly.pdf?month=2026-10 | 月报 PDF（reportlab，中文字体 LS_PDF_FONT；不含截图/人脸） |

### WS /ws/events
服务端**主动推送**事件行（结构同 Event）。客户端可发任意文本作心跳（服务端忽略）。断线自动重连由客户端负责（web/src/api.js `connectEvents` 已实现 3s 重试）。

## 5.3 维护（任务14）

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | /api/maintenance/cleanup | 手动触发一轮清理（幂等），返回 {deleted_events, deleted_files, freed_mb} |
| GET | /api/maintenance/status | 上次清理结果/当前磁盘剩余 GB/三项保留配置 |

后台每 6h 自动清理（LS_RETENTION_EVENTS_DAYS / LS_RETENTION_POOL_DAYS / LS_DISK_MIN_FREE_GB）。

## 6. 统计（报表页）

### GET /api/stats/daily?days=7&camera_id=&status=
```json
{"days": [
  {"date": "2026-10-26", "total": 12, "l1": 3, "l2": 7, "l3": 2,
   "enter": 5, "enter_no_ppe": 4, "exit": 3}
]}
```
days 1-90 钳制；缺数据天补零。

### GET /api/stats/weekly?weeks=4&camera_id=&status=

按 ISO 周聚合（周一为起点），最近 N 周（1-26，含本周，缺数据周补零）。行结构同 daily，日期字段为 `week_start`。

### GET /api/stats/summary?days=30&camera_id=&status=

汇总卡片：`{"days":30, "total":n, "by_level":{"1":..,"2":..,"3":..}, "by_type":{..}, "top_days":[{"date","total"}×≤5]}`。

### GET /api/stats/daily?days=7&start_ts=&end_ts=（任务21 增强）

`start_ts/end_ts` 给定时按自定义窗口逐日展开（纯日期自动补边界；窗口>400 天只返回有数据的天）。

### GET /api/stats/hourly?date=2026-10-26&camera_id=
```json
{"date": "2026-10-26", "hours": [{"hour": 8, "total": 3, "l1": 0, "l2": 2, "l3": 1, "...": 0}, ...]}
```
date 格式错误 → 400。

## 7. 人脸（默认关闭；合规红线见 20-research §7 与任务 05）

> 开关只能改 `.env`（`LS_FACE_ENABLED`）重启生效，不提供界面开关。关闭时所有写操作 403，管线不调用。

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | /api/face/status | `{"enabled": false, "configured": bool, "threshold": 0.75}` |
| POST | /api/face/persons | **multipart/form-data**：`subject`, `consent=true`, `photo=<file>`；无 consent → 400 |
| GET | /api/face/persons | `[{subject, consent_ts}]`（合并 CompreFace 列表与本地同意记录） |
| DELETE | /api/face/persons/{subject} | 删除单人 |
| DELETE | /api/face/persons/all | 一键清除（人员+同意记录） |

## 8. 集成注意（客户端开发者）

1. **前端不调推理**：所有 AI 在服务端（ADR-0002）
2. 安全区绘制：显示坐标按显示宽度等比换算回 1280×720（Zones.vue 已有实现可参考）
3. 事件类型/级别文案：以 `constants.py` 为准，Web 端 TYPE_TEXT 保持同步
4. WebSocket 消息可能乱序（多 worker 线程）——按 id 排序展示
5. 大文件（视频/截图）用 URL 拉取，不进 JSON

## 8.1 健康探针与错误约定（任务63）- `GET /api/health/live` → `200 {"status":"ok"}`（进程存活即返回，不查 DB/模型；负载均衡存活探测用）
- `GET /api/health/ready` → 全过 `200 {"status":"ready","checks":{db,data_dir,model,disk}}`，
  任一不过 **503** + `"status":"degraded"`（磁盘阈值 `LS_READY_MIN_FREE_GB` 默认 1GB；不加载 ONNX）
- **请求 ID**：每个响应头都带 `X-Request-ID`（客户端传入 1–128 字符则回显，否则生成 uuid4）；
  报障时提供该 ID，可在 `server.log` 按 `[req=<id>]` 检索对应错误堆栈。
- **错误体统一格式**（成功响应结构不变）：`{"detail": <可读信息>, "request_id": "...", "code": ...}`：
  - HTTPException → 原状态码，`code="http_<status>"`；
  - 参数校验失败 → 422，`code="validation_error"`，`detail` 为首个错误摘要（非整段 pydantic）；
  - 未捕获异常 → 500，`code="internal_error"`，`detail` 固定"服务器内部错误"（堆栈只进日志）。

`/api/system` 响应含 `schema_version`（任务69）：数据库结构版本，随迁移单调递增，客户端可用于
兼容性判断（只增不改）。

### 配置导入导出（任务79，仅管理员）

- `GET /api/config/export?with_secrets=0` → `{"format":"labsentinel-config","version":1,
  "schema_version":N,"exported_at":...,"cameras":[{name,source,type}],
  "zones":[{camera_name,name,polygon}],"rules":[{camera_name,zone_name,behavior,level,...}}`。
  默认把 RTSP 凭证脱敏为 `***:***@`；`with_secrets=1` 返回真实 source（响应 no-store）。
  不含事件/截图/人脸数据。
- `GET /api/events` / `GET /api/events/count` 增加 `q`（任务84）：对 detail/type 做
  LIKE 子串匹配（ASCII 不分大小写，中文原样）；`%`/`_` 按字面（服务端自动 ESCAPE）；
  1–100 字符（>100 → 400），空白=不过滤；可与 camera/level/status/日期/分页组合。
- `POST /api/events/bulk-status`（任务83，仅管理员）body
  `{"ids":[..≤200..],"status":"acked|false_positive|ignored","handled_by":"admin"}` →
  `{"updated","skipped","missing":[..],"failed":[..]}`。与单条端点同一套状态规则
  （false_positive 同样截图入标注池）；missing 容错；单条失败不中断整批。

### 操作审计（任务87，仅管理员）

- `GET /api/audit?limit=50&offset=0&actor=&action=&start_ts=&end_ts=` →
  `{"items":[{ts,request_id,actor,action,target,status_code}],"total"}`。
  自动记录所有 `/api/*` 写操作（POST/PUT/PATCH/DELETE；不含 body/headers/query）；
  `actor` 为 admin/anonymous；`target` 从路径提取（如 `camera:3`）；action 前缀匹配筛选。
  `request_id` 与响应头一致，可据此检索 server.log 完整上下文。

### 告警静默（任务81，写仅管理员；检测/事件/WS 照常，只抑制外部渠道）

- `GET /api/alert-silences?active=1` → 生效静默列表（含 `remaining_s`）；
- `POST /api/alert-silences` body `{"scope":"global|camera|level|type","target":"…","duration_s":3600,"reason":"…"}`
  （duration 60–604800；camera 目标须存在、level∈1..3、type∈事件类型白名单）→ 返回记录；
- `DELETE /api/alert-silences/{id}` 提前解除。命中静默的事件：**不建投递行、不重试、零渠道发送**；
  过期自动恢复（记录保留，列表默认只显示 active）。

- `POST /api/cameras/{cid}/copy-config`（任务80，仅管理员）body
  `{"target_ids":[2,3],"copy_zones":true,"copy_rules":true,"name_suffix":""}`：
  把源摄像头的区域/规则批量复制到多路目标；规则 zone_id 按**区域名**映射到目标（绝不指向源区域，
  目标无同名区域时跳过并 warning）；同名区域跳过；返回
  `{"zones_created","zones_skipped","rules_created","rules_skipped","targets":[{camera_id,zone_map}],"warnings"}`。
  先全量校验后写库（源/目标存在、排除自身）；不启动摄像头、不复制事件。
- `POST /api/config/import` body `{"mode":"merge|replace","confirm_replace":false,"data":{导出JSON}}`：
  merge 按名称去重（同名摄像头/区域/规则跳过）；replace 需 `confirm_replace=true`
  （清空现有摄像头/区域/规则，**事件保留但失去对应摄像头**，重新导入同名可恢复关联）。
  校验失败 400 且不产生部分写入；导入不自动启动摄像头。
  返回 `{"created":{...},"skipped":{...},"warnings":[...]}`。

## 8.2 维护与清理预演（任务14/68）

- `GET /api/maintenance/preview`（任务68，**只读预演**）：返回 `cutoff`（保留期截点）、
  `events{count,bytes}`（将删除的过期事件及关联文件字节）、`pool{files,bytes}`（过期误报池文件）、
  `disk_free_gb`、`estimated_free_gb`（执行后预计剩余）、`would_emergency_clean`。绝不删除任何东西。
- `GET /api/maintenance/status` 新增（任务68）：`disk_total_gb`/`disk_free_gb`/`disk_free_pct`、
  `data_bytes`（超 2 万文件时退化为四核心目录统计并置 `data_bytes_capped=true`）、
  `low_disk`（free < LS_DISK_MIN_FREE_GB）、`forecast_days`（**估算值**：按近 7 天日均增速推算
  可用天数，数据不足为 null）。
- `POST /api/maintenance/cleanup`：真实清理（任务14，写操作需管理令牌）——建议先 preview 预演。

## 8.3 单路摄像头运行指标（任务65）

- `GET /api/cameras/{id}/status` → 该路 DB 信息（name/type/status）+ 运行指标：
  `fps`、`last_frame_ts`、`frame_age_s`（无帧为 null；>10s 可视为画面停滞）、
  `last_error`（最近一次打开/读帧/分析异常的简短原因，**已脱敏**：RTSP 凭证以 `***:***@` 显示，
  分析异常带 `analyze:` 前缀）、`reconnect_count`（打开重试+断流重连累计）、`opened_at`、
  `frames_total`（累计帧数）。未启动过 worker 的摄像头返回各指标零值。不存在 → 404。
- `/api/system` 的 `cameras[]` 同步追加上述指标字段（原有键不变，向后兼容）。
- 行为：rtsp/usb 打开失败或断流做**有限重连**（5 次 × 3s，`reconnect_count` 计数），
  耗尽转 `error` 并退出线程；file 源坏路径直接 `error` 退出。

## 9. 未实现（规划中，勿在代码中假设）

- 鉴权/多用户——M3（当前 handled_by 固定 admin/请求体可传）
- Top 规则触发统计（月报 PDF 的规则维度，需 events 关联 rule_id）

## 10. 5 分钟上手（curl / PowerShell 实操，任务31）

> **版本提示**：本章含分页/`count`/事件处置等新端点，服务端需重启到最新代码（旧实例会 404）。
> 已知小问题：`GET /api/events` 行内含 `snapshot/video` 的服务器本地绝对路径（信息泄露面小，
> 但建议后续版本只回 `has_snapshot/has_video` 布尔——已记入主线待办，改前不动契约）。

> 以下示例与当前实现逐一对照过（端点/参数名见上文各节）。`$BASE` 指 8000 服务端。

### 10.1 Git Bash（curl）版

```bash
BASE=http://127.0.0.1:8000

# 0) 健康检查（永不鉴权）
curl -s "$BASE/api/system"

# 1) 添加文件摄像头（autostart=false 先配置再启动）
curl -s -X POST "$BASE/api/cameras" -H "Content-Type: application/json" \
  -d '{"name":"demo","source":"D:/录像/lab.mp4","type":"file","autostart":false}'
# → 返回 {"id": 5, ...}，记下 id

# 2) 画安全区（多边形为 1280x720 像素坐标）
curl -s -X POST "$BASE/api/zones" -H "Content-Type: application/json" \
  -d '{"camera_id":5,"name":"核心区","polygon":[[520,180],[1180,180],[1180,620],[520,620]]}'

# 3) 加规则：进入(L2) 与 未穿实验服(L3)
curl -s -X POST "$BASE/api/rules" -H "Content-Type: application/json" \
  -d '{"camera_id":5,"zone_id":1,"behavior":"enter","level":2,"cooldown_s":10}'
curl -s -X POST "$BASE/api/rules" -H "Content-Type: application/json" \
  -d '{"camera_id":5,"zone_id":1,"behavior":"enter_no_ppe","level":3,"cooldown_s":10}'

# 4) 启动摄像头
curl -s -X POST "$BASE/api/cameras/5/start"

# 5) 查事件（分页/过滤）
curl -s "$BASE/api/events?camera_id=5&limit=20&status=new"
curl -s "$BASE/api/events/count?camera_id=5"

# 6) 事件截图 / 视频存证
curl -s -o snap.jpg "$BASE/api/events/1/snapshot"
curl -s -o clip.mp4 "$BASE/api/events/1/video"

# 7) 处置事件：确认 / 误报 / 忽略
curl -s -X POST "$BASE/api/events/1/status" -H "Content-Type: application/json" \
  -d '{"status":"acked"}'

# 8) 清理（连带删除该摄像头的安全区与规则）
curl -s -X DELETE "$BASE/api/cameras/5"
```

### 10.2 PowerShell 版

```powershell
$BASE = "http://127.0.0.1:8000"

# 0) 健康检查
Invoke-RestMethod "$BASE/api/system"

# 1) 添加摄像头（中文 body 用 UTF-8 文件 + -Body 文件内容，避免乱码）
@{"name"="演示摄像头";"source"="D:/录像/lab.mp4";"type"="file";"autostart"=$false} |
  ConvertTo-Json | Out-File -Encoding utf8 camera.json
Invoke-RestMethod -Method Post -Uri "$BASE/api/cameras" `
  -ContentType "application/json; charset=utf-8" -InFile camera.json

# 2-4) 安全区 / 规则 / 启动：同上结构，换 Uri 与 body 文件
# 5) 查事件
Invoke-RestMethod "$BASE/api/events?limit=20"
```

### 10.3 真实踩坑注意

| 坑 | 说明与解法 |
|---|---|
| Git Bash curl 中文 JSON 损坏 | Windows 下 curl 对中文 body 会转码错乱。解法：**中文 body 写进 UTF-8 文件后 `-d @body.json`**（或加 `--data-binary`），或直接用 python httpx（服务端测试脚本均为 httpx） |
| PowerShell 中文乱码 | 先 `chcp 65001` 切 UTF-8 代码页；`Invoke-RestMethod -ContentType "application/json; charset=utf-8"`；body 走 `-InFile` |
| 鉴权 | `LS_ADMIN_TOKEN` 启用后，所有 POST/DELETE 需 `-H "X-Admin-Token: <token>"`；WS 连 `/ws/events?token=<token>`；`GET /api/system` 永不拦截 |
| WebRTC | `GET /api/cameras/{id}/webrtc` 返回的 `ws_url` 是给**浏览器播放器**用的（go2rtc :1984 信令），curl 拿到即可，不能用 curl 建连 |
| 事件视频 | `GET /api/events/{id}/video` 仅在服务端装有 OpenH264 时为浏览器可播的 H.264（见部署FAQ） |

### 10.4 端到端示例脚本（免手敲）

- `tools/e2e_smoke.py`：黑盒全链路（摄像头→安全区→规则→启动→事件→截图→清理），`--base-url` 可指向任意实例
- `tools/loadtest.py`：多路并发压测（训练/压测勿并行，见 50-tasks/README 铁律）

### 10.5 错误码速查（对照实现）

| 码 | 常见原因 |
|---|---|
| 400 | 参数不合法：status 非法值、date 非 YYYY-MM-DD、month 非 YYYY-MM |
| 401 | 管理令牌缺失/错误（LS_ADMIN_TOKEN 启用后；写操作与受保护读） |
| 403 | 人脸功能未开启（LS_FACE_ENABLED=false）时的 /api/face 写操作 |
| 404 | 资源不存在：摄像头/事件/投递记录 id 无效 |
| 502 | 上游（CompreFace）不可用或返回错误（人脸注册/列表/删除） |
| 503 | 功能未配置：人脸缺 API Key、投递查询早于 store 绑定 |
