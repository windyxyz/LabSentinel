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

## 9. 未实现（规划中，勿在代码中假设）

- 鉴权/多用户——M3（当前 handled_by 固定 admin/请求体可传）
- Top 规则触发统计（月报 PDF 的规则维度，需 events 关联 rule_id）
