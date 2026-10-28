# 64 · 本机 USB / OBS 虚拟摄像头接入报告（任务 56）

> 日期：2026-10-28。目的：真实多路 RTSP 暂不可用时，验证服务端的 **USB 设备链路**（open→分析→事件→截图），
> 并用 OBS Virtual Camera 播放演示视频模拟"摄像头画面"。

## 1. 设备盘点

- `Get-PnpDevice -Class Camera,Image`：`HD Webcam`（USB\VID_2B7E...）、`IR Camera`。
- `ffmpeg -list_devices true -f dshow -i dummy` 可见：
  - `"HD Webcam" (video)`
  - `"OBS Virtual Camera" (video)`（虚拟摄像头驱动已注册）
- OpenCV(DirectShow) 索引：`0` = 物理摄像头（当前黑屏/遮挡），`1` = OBS 虚拟摄像头（pyvirtualcam 供帧后可见）。

## 2. OBS 虚拟摄像头供帧

```bat
python tools\virtual_cam_feed.py --video m0-spike\data\lab-walk-24076.mp4 --width 1280 --height 720 --fps 30
```

- `pyvirtualcam 0.15.0`（装入 m0-venv，仅工具可选依赖）→ `pyvirtualcam.Camera(...).send(RGB)`；
- OpenCV 打开 index 1 抽帧确认为实验室演示画面（非黑屏）。

## 3. 服务端接入

隔离实例（主库零写入）：

```bat
set LS_DB_PATH=data/usb_test.db
set LS_SNAPSHOT_DIR=data/usb_snapshots
set LS_VIDEO_DIR=data/usb_videos
set LS_ANNOTATION_POOL_DIR=data/usb_pool
uvicorn app.main:app --port 8001
```

### 3.1 OBS 虚拟摄像头

- `POST /api/cameras {name:"obs-virtual", source:"1", type:"usb", autostart:false}`；
- 安全区（全画面）+ `enter` 规则（L2，cooldown 2s）；启动后：
  - status=running；**fps=29.6**（统计窗口后）；
  - 3 条 `enter` 事件；事件详情含安全区名；
  - `GET /api/events/{id}/snapshot` → **200 image/jpeg 196424B**。

### 3.2 物理 HD Webcam

- `source:"0"`，启动后 status=running、**fps=12.5**、快照 200（77KB，黑帧——摄像头当前遮挡/隐私开关）。

## 4. 修复的 bug

1. **USB 源字符串被当文件名**：`cv2.VideoCapture(camera["source"])` 对 `"0"` 返回 False。
   新增 `capture_source()`：`type=usb` 且纯数字 → `int`；RTSP/文件原样；重连路径同样使用。
2. **USB FPS=-1**：`file_fps = cap.get(...) or 30` 会保留 -1，录像 writer/环形缓冲异常；
   已加 `<=0 或 >240 → 30` 兜底。
3. 单测：`test_capture_source_usb_numeric`（core tests 9 passed）。

## 5. 清理与边界

- 删除测试摄像头/区域/规则；8001 停止；`usb_test.db`/三个临时目录删除；主库未写入。
- 虚拟摄像头 feed 进程已停；1984/8554/8000/8001/5173 无残留监听。
- 局限：仅设备链路验证；不能替代真实 RTSP 24h、红外、多路并发；OBS 索引因机器而异，以
  `python tools/list_cameras.py` 枚举为准。

## 6. 复现命令

```bat
python tools\list_cameras.py --snapshot-dir logs\cam_probe
python tools\virtual_cam_feed.py --video m0-spike\data\lab-walk-24076.mp4
:: 服务端摄像头：type=usb, source=1（OBS）或 0（物理）
```
