# 任务 56 · 本机 USB / OBS 虚拟摄像头接入验证

> 背景：真实多路 RTSP 摄像头暂时无法接入。用本机 HD Webcam + OBS Virtual Camera 验证「设备链路」（OpenCV 打开 → 分析 → 事件 → 截图），并修复 USB 源解析问题。完整证据见 [docs/64](../64-usb-virtual-camera-report.md)。

## 目标

1. 枚举本机摄像头设备（DirectShow），确认物理/虚拟设备索引；
2. 用 OBS Virtual Camera 播放演示视频，服务端以 `type=usb, source=1` 接入，验证 fps/事件/截图；
3. 用物理 HD Webcam `source=0` 验证连接/状态/快照；
4. 修复 USB 源字符串 `"0"` 被 OpenCV 当文件名的问题；修 FPS=-1 导致录像 writer 异常；
5. 提供可复用工具：`tools/list_cameras.py`、`tools/virtual_cam_feed.py`；更新部署指南。

## 约束

- 只占 8001 + 临时数据目录；主库零写入；不训练；不与其它推理并发。
- OBS 虚拟摄像头仅联调，不替代真实 RTSP 24h；测试结束清理进程/临时数据。

## 验收标准

- 枚举工具能列出设备并保存首帧；
- OBS 虚拟摄像头接入后 fps>0、产生事件、截图 200；
- 物理摄像头接入后 status=running、fps>0、快照 200（画面黑属遮挡/隐私开关，不算失败）；
- USB 源解析/FPS 兜底有单元测试，pytest 全绿；
- 临时数据清理、8001 释放、主库未动。

## 执行报告（主线，2026-10-28）

### 设备与工具
- 本机识别到 `HD Webcam`（USB，index 0）与 `IR Camera`；`ffmpeg -list_devices` 同时看到 **OBS Virtual Camera**（驱动已注册）。
- 新工具：`tools/list_cameras.py`（枚举 index/分辨率/fps，可存首帧）、`tools/virtual_cam_feed.py`（pyvirtualcam 播放视频到 OBS 虚拟摄像头，循环/尺寸/fps 可配）。

### 修复
- `server/app/pipeline.py`：新增 `capture_source()`——`type=usb` 的 `"0"/"1"` 转 int；RTSP/文件原样。此前 `VideoCapture("0")` 返回 False（被当文件名），USB 摄像头无法接入。
- USB 摄像头 `CAP_PROP_FPS` 常为 `-1`：录像 writer/环形缓冲会异常；统一 `<=0 或 >240 → 30` 兜底。
- 新增单测 `test_capture_source_usb_numeric`（9 个 core tests 通过）。

### 实测（8001 + 临时目录）
| 设备 | source | 结果 |
|---|---|---|
| OBS Virtual Camera（pyvirtualcam 播 lab-walk） | `"1"` | status=running、**fps=29.6**、3 条 enter 事件、截图 200/196KB |
| HD Webcam（物理，当前黑屏/遮挡） | `"0"` | status=running、**fps=12.5**、快照 200/77KB（黑帧符合遮挡预期） |

- backend=directml；测试摄像头/区域/规则已删除；8001 停止；主库零写入。
- OBS 虚拟摄像头通过 `pyvirtualcam 0.15.0`（装入 m0-venv，工具可选依赖，未加入 server requirements）。

### 局限
- 虚拟摄像头/黑屏物理摄像头只能验证设备链路，不能替代真实画面质量、红外、RTSP 长连接；
- 未做多路 USB/虚拟摄像头并发；未做 24h；
- OBS 驱动/应用版本差异可能导致索引变化，用 `list_cameras.py` 为准。
