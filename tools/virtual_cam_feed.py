"""把演示视频喂给 OBS 虚拟摄像头（pyvirtualcam），用于无真实摄像头时的 USB 设备链路测试。

前置：Windows 已注册 OBS Virtual Camera（ffmpeg -list_devices 能看到）；pip install pyvirtualcam。
用法（m0-venv）:
    python tools/virtual_cam_feed.py --video m0-spike/data/lab-walk-24076.mp4
    python tools/virtual_cam_feed.py --width 1280 --height 720 --fps 30
然后服务端添加 type=usb、source=1（OBS Virtual Camera 的 OpenCV 设备号）的摄像头。

注意：本脚本只负责产生虚拟摄像头画面；不要与真实摄像头 24h 验收同时跑。
"""
import argparse
import sys

import cv2
import numpy as np

try:
    import pyvirtualcam
except ImportError:
    print("缺少 pyvirtualcam：pip install pyvirtualcam -i https://pypi.tuna.tsinghua.edu.cn/simple")
    raise SystemExit(2)

DEFAULT_VIDEO = r"D:\User\Desktop\TOOLS\监控识别\m0-spike\data\lab-walk-24076.mp4"


def main() -> int:
    ap = argparse.ArgumentParser(description="播放视频到 OBS 虚拟摄像头")
    ap.add_argument("--video", default=DEFAULT_VIDEO)
    ap.add_argument("--width", type=int, default=1280)
    ap.add_argument("--height", type=int, default=720)
    ap.add_argument("--fps", type=int, default=30)
    ap.add_argument("--no-loop", action="store_true", help="播完即退出（默认循环）")
    a = ap.parse_args()

    cap = cv2.VideoCapture(a.video)
    if not cap.isOpened():
        print(f"打不开视频: {a.video}")
        return 1

    try:
        with pyvirtualcam.Camera(width=a.width, height=a.height, fps=a.fps,
                                 fmt=pyvirtualcam.PixelFormat.RGB) as cam:
            print(f"虚拟摄像头就绪: {cam.device} ({a.width}x{a.height}@{a.fps})，Ctrl+C 退出")
            while True:
                ok, frame = cap.read()
                if not ok:
                    if a.no_loop:
                        break
                    cap.set(cv2.CAP_PROP_POS_FRAMES, 0)
                    continue
                frame = cv2.resize(frame, (a.width, a.height))
                cam.send(cv2.cvtColor(frame, cv2.COLOR_BGR2RGB))
                cam.sleep_until_next_frame()
    except KeyboardInterrupt:
        pass
    except Exception as e:  # noqa: BLE001
        print(f"虚拟摄像头启动失败: {e}")
        print("检查：OBS Virtual Camera 是否注册（ffmpeg -list_devices true -f dshow -i dummy）")
        return 1
    finally:
        cap.release()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
