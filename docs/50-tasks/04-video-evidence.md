# 任务 04 · 事件短视频存证（先读本目录 README.md）

## 背景

当前事件只有单帧截图（`server/app/pipeline.py` 的 `_fire_event`，整帧存 `server/data/snapshots/`）。需求 FR6 要求"前后数秒短视频证据"，便于人工复核违规行为。

## 目标

事件触发时保存 `前 2 秒 + 后 2 秒` 的 MP4 片段（h264 编码，无声），事件 API 返回 `has_video` 字段，Web 事件查询页可点开播放。

## 设计约束（重要）

- 简单可靠方案：**环形帧缓存**——CameraWorker 在主循环里维护 `collections.deque(maxlen=int(file_fps*2)+10)` 存最近 ~2.5 秒原始帧；触发事件时，从缓冲拷出、继续收集 2 秒（用 `threading.Event` + worker 循环里的"待完成录制"队列，每个事件一个 `mp4v`/`avc1` VideoWriter，分辨率 1280x720）
- 同一时间最多 3 个并发录制（超出的退化为仅截图），防止告警风暴拖垮 IO
- 视频存 `server/data/videos/ev_{event_id}.mp4`；事件行加 `video` 列（SQLite 迁移：`ALTER TABLE events ADD COLUMN video TEXT`——写个小迁移函数，建表语句也同步加列，旧库兼容）
- API：`GET /api/events/{id}/video` 返回 `video/mp4`（FileResponse）；events 列表行加 `has_video`
- Web 端 `Events.vue`：有视频时显示 ▶ 按钮，点击弹出 `<video controls>` 播放层（复用现有 zoom 弹层样式）
- 截图逻辑保留不变

## 验收标准

1. pytest 全绿 + 新增 ≥2 个单测（环形缓冲长度/迁移函数旧库兼容）
2. 用 `m0-spike/data/lab-walk-24076.mp4` 走一遍垂直切片（README 有 httpx 模板），事件产生视频文件且 ≥2 秒、可用 ffplay/浏览器播放
3. `npm run build` 通过

## 执行报告（2026-10-25，由主线验收补记）

- 功能已落地：环形帧缓冲 + 后台写盘线程 + events 表 video 列 + `/api/events/{id}/video` + Web 播放层；实测生成 `server/data/videos/ev_14.mp4` 等。
- 验收发现（已确认可接受）：本机无 OpenH264 DLL，avc1 尝试会打印 4 行错误日志后由 mp4v 兜底成功——**功能正常，日志噪音待降噪**。
- 遗留问题：mp4v（MPEG-4 Part 2）编码 Chrome 等浏览器可能无法直接播放（ffplay 可播）；浏览器兼容需换 H264——方案：下载 openh264 dll 放 PATH 或改用 ffmpeg 子进程编码，列为后续优化。

## 执行报告

### 交付物
- `server/app/pipeline.py`：`CameraWorker` 主循环维护环形帧缓存 `deque(maxlen=int(file_fps*2)+10)`；
  事件触发时拷出前 2s、主循环补齐后 2s（`_Recording` + `queue.Queue` + 后台写盘线程）；
  同一时间最多 3 个并发录制，超出退化为仅截图；停机时同步清空队列保证视频写完
- 编码：`_open_writer` 优先 `avc1`（CAP_MSMF → FFMPEG）失败退 `mp4v`，1280x720，fps 取源帧率
- `server/app/store.py`：`events` 表加 `video` 列（建表语句同步）+ `_migrate()` 旧库 `ALTER TABLE` 兼容（幂等）；
  `set_event_video`/`event_video`
- API：`GET /api/events/{id}/video`（FileResponse video/mp4）；事件行加 `has_video`（列表与 WS 推送都带）
- Web：`Events.vue` 有视频的事件显示 ▶ 按钮 → 复用 zoom 弹层样式的 `<video controls autoplay>` 播放层
- 新增 `LS_FILE_LOOP`（默认 false）：文件源循环播放，为压测/长演示准备（正常播放完即止）

### 关键工程点：H.264 编码（浏览器可直接播放）
- 本机 opencv-python 5.0.0.93 的 VideoWriter 写 avc1 失败，报缺 Cisco OpenH264 DLL
- 解决：从 Cisco 官方二进制 CDN（ciscobinary.openh264.org，bz2 压缩包）下载 `openh264-2.5.0-win64.dll`
  放入 m0-venv 的 cv2 包目录，并在 `server/app/__init__.py` 把该目录加入 PATH
  （OpenCV 按导入时快照的搜索路径加载 DLL，必须先于 `import cv2` 生效，故放包初始化处）
- 实测 fourcc='h264'、浏览器 <video> 可直接播放。协议：OpenH264 二进制为 Cisco BSD-2+专利授权，
  与模型同理不随库分发，部署者按需下载（已写入 README 协议节）

### 验收结果
- pytest：4 个新单测（环形缓冲长度/录像生命周期/旧库迁移兼容+幂等/编码器冒烟），全套 31 passed
- 垂直切片实测（隔离 8001 实例 + `m0-spike/data/lab-walk-24076.mp4`）：
  事件 enter/exit 均产出视频；`GET /api/events/1/video` → 200 video/mp4 8.2MB；
  **144 帧 @25fps = 5.76s、fourcc='h264'**（≥2s 达标，超出部分因文件源在事件后播完、收尾帧并入）
- `npm run build` 通过；截图逻辑未动

### 遗留问题
- mp4v 回退路径下浏览器不可播（仅 ffplay 可播）——已通过 OpenH264 让本机默认走 H.264，其他部署机若缺 DLL 会静默回退 mp4v，报告里需提示装 DLL
- 录像为无声视频（设计如此）；音频存证不在本任务范围
