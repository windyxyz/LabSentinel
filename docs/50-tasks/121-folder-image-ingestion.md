# 任务 121 · 图片文件夹自动识别（P1，用户直接指示）

> 来源（2026-11-03 用户）：实际生产场景中会有一个软件把所有内容自动截图为图片，
> 系统需**自动读取目标文件夹、识别关键图片并汇报**；同时**保留原有视频接入**。
> 平台优先级不变：先把 Windows 端（网页+服务）做完整，手机端暂缓。

## 需求拆解

1. 新增一种“摄像头”源：`type=folder`，`source=目录路径`；与 rtsp/file/usb 并列，可启停/删除。
2. 目录由外部截图软件持续写入；系统自动发现新文件并识别，**不改动原始截图**。
3. “关键图片”可筛选、可汇报：有人出现的图、显式违规图分开标记；按摄像头/时间汇总。
4. 违规图按现有安全区规则产生事件与告警（冷却限频），事件证据可回溯。
5. 原有 RTSP/视频文件/USB 接入、安全区、规则、事件、告警全部保持可用。
6. 长跑友好：大目录不每轮全量 stat；重启不重复识别；损坏文件不阻塞队列。

## 设计

### 数据流

```
外部截图软件 → 目录（jpg/png/bmp/webp/tif，可递归）
   → FolderWorker（轮询/增量目录扫描/静置过滤/去重）
   → 人体模型（detect_persons）+ 实验服模型（全图一次取证据框）
   → 安全区锚点判定 + 规则引擎（track_id=0，按规则冷却）
   → folder_images 表（每图一行：人数/在区/证据/违规/关键）
   → 图片识别页（缩略图墙 + 汇总卡 + 详情原图）
   → 违规时：events 表 + 告警派发（截图证据副本）
```

### 关键语义（与视频源的区别）

- 图片帧之间**没有连续性**：不做 ByteTrack、不维护进出场状态、不产生 `exit`。
- `key=1`：图片中检测到人员（需要人工关注的候选帧；即使未配置实验服模型也成立）。
- `violation=1`：出现**显式违规证据**——人员锚点在安全区内且实验服判定 `no`
  （`No Lab Coat` 类检测，阈值 `LS_PPE_CONF`），或图片内在区人数超过 `occupancy_over` 阈值。
  “实验服状态不确定”**不算违规**（沿用证据分级，避免误报）。
- **默认只为显式违规创建事件/告警**，不为每张“有人”的截图造 `enter` 事件
  （否则每秒截图都像“新进入”，语义错误且刷屏）。需要时 `LS_FOLDER_ENTER_EVENTS=true`
  按 `enter` 规则+冷却产生事件。`LS_FOLDER_ENTER_EVENTS` 默认 false。
- 事件 `track_id=0`，规则冷却按 `(rule_id, 0)` 生效：同一条规则在冷却期内只报一次，
  但 `folder_images` 仍逐图记录（汇报不丢图）。

### 扫描与去重

- 轮询间隔 `LS_FOLDER_POLL_S`（默认 3s）；文件修改后静置 `LS_FOLDER_STABLE_S`（默认 1.5s）
  才读取，避免读到写了一半的截图。
- 去重键 = `(camera_id, path, size, mtime_ns)`，数据库唯一索引 + 本次会话内存缓存；
  服务重启后不会重复识别，也不会重复产生事件。
- 只处理白名单扩展名、`0 < size <= LS_FOLDER_MAX_FILE_MB`、mtime 不在未来（防时钟异常）。
- 目录级增量：目录 mtime 未变则跳过其文件枚举（新增/删除/改名会更新目录 mtime）；
  每 `LS_FOLDER_FULL_RESCAN_S`（默认 300s）强制全量一次，补抓“原地覆盖但目录时间不变”的文件。
- 单轮最多 `LS_FOLDER_MAX_PER_TICK` 张（默认 20），图片之间按 `LS_ANALYSIS_FPS` 限速
  （默认 5 张/秒/路），避免打满 DML 串行锁。
- 损坏/不可解码图片落一行 `error` 记录（不反复重试），计入“读取失败”。

### 存储与保留

- 新表 `folder_images`（schema v5）：路径/大小/mtime/图片时间/识别时间/分辨率/人数/在区/
  未穿/已穿/命中区域/违规明细/关键/违规/关联事件/证据副本/错误。
- 违规图片才生成证据副本（`snapshots/img_<cid>_<ts>_<rand>.jpg`，JPEG 原样复制、其它格式转 JPEG）；
  列表缩略图懒生成并缓存到 `data/thumbs/img_<id>.jpg`。
- `LS_RETENTION_IMAGES_DAYS`（默认 30 天）按**识别时间**清理记录 + 证据副本 + 缩略图；
  **原始截图永不移动/删除**（外部截图软件自行管理生命周期）。
  若证据副本仍被事件引用（事件保留 180 天），则先删图片记录、保留文件，由事件保留策略到期清理，
  避免事件详情 404；原始截图已被外部轮转删除时，详情/缩略图自动回退到证据副本。
- 删除图片文件夹摄像头时同步清理该路的识别记录/证据/缩略图，原始截图保留。

### 安全

- 新增/启动时校验目录存在；服务只读取 DB 中登记过、且**真实路径位于该路接入目录内**的图片
  （`resolve` + `commonpath`，防软链接/DB 篡改读取目录外文件）。
- 扩展名白名单；媒体端点沿用读保护（`require_read`，支持 header 或 `?token=`）。
- 内部数据目录（`data/`）不会被当作输入目录递归，防止“证据图→再识别”的自激循环。

## API（增量）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/folder-images/summary?days=7&camera_id=` | 按摄像头汇总：总数/关键/违规/人数/在区/失败/最近时间 |
| GET | `/api/folder-images?camera_id=&key=1&violation=1&start_ts=&end_ts=&error_only=&limit=&offset=` | 图片列表（时间倒序） |
| GET | `/api/folder-images/{id}` | 单张识别详情 |
| GET | `/api/folder-images/{id}/file` | 原始图片（目录内校验；读保护） |
| GET | `/api/folder-images/{id}/thumb` | 480px 缩略图（磁盘缓存） |
| GET | `/api/folder-images/export.csv?camera_id=&key=&violation=&start_ts=&end_ts=` | 按当前筛选导出 CSV（UTF-8 BOM，≤5 万行） |
| POST | `/api/folders/{cid}/scan` | 立即扫描（运行中唤醒一轮；未运行则启动） |
| POST | `/api/cameras` | `type` 增加 `folder`，创建时校验目录 |

`GET /api/cameras`、`/api/system` 的 folder 摄像头会带
`kind=folder / processed_total / key_total / violation_total / error_total / backlog / last_scan_ts / current_file`。
`GET /api/cameras/{id}/webrtc` 对 folder 返回 400（无实时流）。
`POST /api/cameras/{id}` 不存在，启停复用 `/start` 与删除。

## 配置（`LS_` 前缀，均有默认值）

| 变量 | 默认 | 说明 |
|---|---|---|
| `LS_FOLDER_POLL_S` | 3 | 目录轮询秒 |
| `LS_FOLDER_STABLE_S` | 1.5 | 文件静置秒（防半张截图） |
| `LS_FOLDER_MAX_PER_TICK` | 20 | 每轮最多识别张数 |
| `LS_FOLDER_RECURSIVE` | true | 递归子目录 |
| `LS_FOLDER_EXTS` | `.jpg,.jpeg,.png,.bmp,.webp,.tif,.tiff` | 允许扩展名 |
| `LS_FOLDER_MAX_FILE_MB` | 32 | 单文件大小上限 |
| `LS_FOLDER_FULL_RESCAN_S` | 300 | 强制全量重扫间隔（0=每轮全量） |
| `LS_FOLDER_ENTER_EVENTS` | false | 是否为“有人进区”也产生 enter 事件 |
| `LS_RETENTION_IMAGES_DAYS` | 30 | 识别记录保留天数 |

## 前端

- 「摄像头」页类型新增“图片文件夹”；表单提示支持的格式/递归/原始只读；列表显示
  已识别/关键/违规/待处理/失败，提供“立即扫描”，隐藏对 folder 无意义的“实时画面”。
- 新增「图片识别」页（导航第三项）：
  - 汇总卡：接入文件夹数、时间范围内识别数、关键/违规/读取失败、当前文件夹待处理；
  - 筛选：文件夹（全部/单个）、时间范围（24h/7d/30d/不限）、关键（有人）/仅违规/全部；
  - 缩略图墙（懒加载、失败占位、违规角标），点击查看原图与识别详情；
  - 详情：图片时间/识别时间/分辨率/人数/在区/命中区域/实验服证据/违规列表/关联事件/源路径/下载原图；
  - 15s 自动刷新，三态（加载/错误保留旧值/空）复用 AsyncState。

## 测试与验收

- 新增 `server/tests/test_folder_source.py`（17 项，假 engine/ppe，不加载 ONNX）：
  迁移 v5 + 唯一键幂等、图片入库/关键/违规/证据/事件、默认不造 enter 事件、
  `LS_FOLDER_ENTER_EVENTS` 可选、有人无违规不告警、静置窗口与非图片过滤、损坏图片只记一次、
  递归+缩略图缓存、每图 PPE 只推理一次/类、人数超限、状态落库回归、API 列表/汇总/媒体越界 403/404、
  删除摄像头清理、保留清理只删证据不删原图。
- `server/tests/test_alert_silence.py` 的 schema 版本断言更新到 v5。
- 前端 `npm run build` 通过。
- 真实服务端到端（隔离数据目录、DirectML）：登记 folder 摄像头 → 3 张真实帧识别入库 →
  `/api/system` running/processed_total=3/key_total=3 → `/api/folder-images` 返回人员/在区/区域 →
  缩略图 200 image/jpeg、原图 200、`/` 返回面板 → 立即扫描 200 → 新图自动入库 →
  重启后不重复识别、DB 状态 running。

## 约束

- 原始截图只读；不改外部截图软件目录结构；不做 OCR/内容理解（仅使用现有检测模型）。
- 一次只跑一个重负载：图片识别走 `Engine._run()` DML 串行锁，图片间限速，不并发压测。
- 不引入新依赖（OpenCV 已在依赖中）。
- 手机端/多端不同步开发；Windows 优先不变。

## 执行报告

**主线 2026-11-03 完成。**

- 后端：`server/app/folders.py`（FolderWorker/缩略图/增量扫描）、`server/app/vision.py`
  （中文安全读图/画布归一/PPE 全图取框，供视频与图片共用）、`store.py` v5 迁移+查询、
  `pipeline.py` Manager 分派+状态、`api.py` 7 个端点+删除清理、`config.py` 9 项配置、
  `retention.py` 图片记录/证据/缩略图保留清理、`folders` 源加入配置导入 type 白名单。
- 前端：`FolderImages.vue` 新页、`App.vue` 导航、`api.js` 4 个方法、`Cameras.vue` 新类型/统计/立即扫描。
- 文档：本文 + `docs/68-image-folder-ingestion.md` 使用手册。
- 测试：`test_folder_source.py` 17 项全绿；**全量 pytest 329 passed（365s）**；
  `npm run build` gzip **54.29KB**；真实 DirectML 端到端通过（隔离库/真实帧，非模拟）：
  5 张图入库、5 张关键、1 张 `No Lab Coat` 违规（事件+证据副本）、缩略图/原图 200、
  重启不重复识别、删除摄像头后记录/证据清零且原始截图保留。
- 说明：样例 mixkit 帧上实验服模型置信度低于阈值（域外，文档 52 已记录），
  端到端验证的是“人员/区域/关键图/媒体/状态/去重”链路；
  真实实验室截图上的 `No Lab Coat` 命中仍以现场数据验收为准。
