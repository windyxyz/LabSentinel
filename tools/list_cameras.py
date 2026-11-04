"""列出本机 OpenCV(DirectShow) 可打开的设备索引，并可保存首帧用于确认画面。

用法（项目 venv）:
    python tools/list_cameras.py                 # 枚举 0..5
    python tools/list_cameras.py --max 8 --snapshot-dir Resources/logs/cam_probe
说明：Windows 下 OpenCV 按索引打开；OBS Virtual Camera 通常与物理摄像头并列出现。
服务端添加摄像头时 type=usb、source=<这里的索引>（如 "1"）。
"""
import argparse
from pathlib import Path

import cv2


def main() -> int:
    ap = argparse.ArgumentParser(description="枚举本机摄像头索引（DirectShow）")
    ap.add_argument("--max", type=int, default=6)
    ap.add_argument("--snapshot-dir", default="")
    a = ap.parse_args()
    out = Path(a.snapshot_dir) if a.snapshot_dir else None
    if out:
        out.mkdir(parents=True, exist_ok=True)

    found = 0
    for idx in range(a.max):
        cap = cv2.VideoCapture(idx, cv2.CAP_DSHOW)
        if not cap.isOpened():
            cap.release()
            continue
        ok, frame = cap.read()
        fps = cap.get(cv2.CAP_PROP_FPS)
        if ok and frame is not None:
            found += 1
            print(f"[{idx}] OK {frame.shape[1]}x{frame.shape[0]} fps={fps:.1f}"
                  + (f" snapshot={out / f'cam_{idx}.jpg'}" if out else ""))
            if out:
                # 中文路径下 cv2.imwrite 会静默失败，统一 imencode + 写字节
                eok, buf = cv2.imencode(".jpg", frame)
                if eok:
                    (out / f"cam_{idx}.jpg").write_bytes(buf.tobytes())
        else:
            print(f"[{idx}] opened but no frame (可能被遮挡/无虚拟摄像头输出)")
        cap.release()
    if not found:
        print("未找到可读摄像头。检查：系统隐私设置允许应用访问摄像头；OBS Virtual Camera 是否已启动输出。")
        return 1
    print(f"共 {found} 个可读设备。服务端添加：type=usb, source=<索引>")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
