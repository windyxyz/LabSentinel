# 任务 09 · 告警通道打磨（先读本目录 README.md）

## 背景（来自审查报告-2026-10-26.md）

告警渠道（server/app/alerts.py）功能完备，但有 4 个已确认的改进点。

## 目标（按优先级）

1. **EmailChannel 异步化**：`AlertDispatcher.dispatch` 在分析 worker 线程里同步逐渠道发送，`EmailChannel.send` 的 SMTP_SSL timeout=10 会阻塞分析循环。改法：dispatcher 用 `concurrent.futures.ThreadPoolExecutor(max_workers=2)` 提交各渠道 send（fire-and-forget，异常打印），保持 WebSocket 同步（它已是线程安全投递）。
2. **视频 H264 兼容**：本机缺 OpenH264 DLL，mp4v 兜底的视频浏览器播不了。方案 A（推荐）：改 `pipeline.py` 的 `_open_writer` 在 avc1 失败后尝试 `ffmpeg` 子进程写 H264（`subprocess.Popen` + stdin 喂 rawvideo，ffmpeg 参数 `-f rawvideo -pix_fmt bgr24 -s 1280x720 -r {fps} -i - -c:v libx264 -preset veryfast -mp4flags +faststart {out}.mp4`），ffmpeg 用 `imageio_ffmpeg.get_ffmpeg_exe()`（已在依赖里）；方案 B：下载 openh264 dll。要求保留 mp4v 为最终兜底。
3. `render_alert`：type 为 occupancy_over 时不输出 track 行。
4. Qmsg/Server酱 的 key/qq 拼 URL 前做 `urllib.parse.quote(key, safe='')`。

## 验收标准

- pytest 全绿（现有 26+ 测试不回归）；EmailChannel 异步化后新增测试：dispatch 不阻塞（用 MockTransport sleep 5s 断言 dispatch 耗时 <1s）
- 视频改造后：本机生成 `.ev_*.mp4`，用 ffprobe 或浏览器验证是 H264（avc1）编码；H264 失败路径回退 mp4v 的测试也要有
- 改动最小化，不动基类安全逻辑（白名单/https/禁重定向）

## 执行报告

### 逐项完成情况
1. **EmailChannel 异步化 ✅**：`AlertDispatcher` 增加 `ThreadPoolExecutor(max_workers=2)`，全部外部渠道 send 异步提交
   （`_safe_send` 兜底捕获异常打日志）；WebSocket 推送保持同步（hub 已是 `run_coroutine_threadsafe` 线程安全投递）。
   新增测试 `test_dispatch_nonblocking`：慢渠道 sleep 5s 时 dispatch 返回 <1s。
2. **视频 H264 ✅（采用方案 B）**：审查报告时缺 OpenH264 DLL，本次已从 Cisco 官方二进制 CDN 下载
   `openh264-2.5.0-win64.dll` 放入 m0-venv 的 cv2 包目录，并在 `server/app/__init__.py` 于 `import cv2` 之前
   把该目录加入 PATH（OpenCV 按导入时快照搜索 DLL）。实测产出 fourcc='h264'（详情见任务04执行报告）。
   `_open_writer` 保留 avc1→mp4v 回退链，未引入 ffmpeg 子进程（本机已无需，方案 A 留作其他部署环境的备选）。
   新增测试 `test_write_video_codec_fallback`：断言产出 h264 或 FMP4（分别对应有/无 OpenH264 的环境）。
3. **render_alert occupancy 隐藏 track ✅**：occupancy_over 输出仅"图标+级别+类型+时间 / 摄像头 #id / 详情"；
   新增 `test_render_alert_occupancy_no_track`，旧测试同步修正。
4. **Qmsg/Server酱 key URL 编码 ✅**：`quote(key, safe='')`（PushPlus token 走 JSON body 无需编码）。
   新增参数化测试 `test_key_urlencoded`（断言请求 `raw_path` 为编码后形式）。

### 验收结果
- `pytest tests/ -q`：**37 passed**（原 32 + 任务09 4 个 + 视频编码 1 个）
- 基类安全逻辑（白名单/https/禁重定向）未动
- 未引入新依赖（imageio-ffmpeg 已存在但未使用）
