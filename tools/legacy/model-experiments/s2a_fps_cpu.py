# M0 / S2a: YOLO11n & YOLO11s person-detection FPS benchmark on CPU (PyTorch)
# 用法: python s2a_fps_cpu.py <video.mp4>
import sys, time, json
import cv2
import torch
from ultralytics import YOLO

video = sys.argv[1] if len(sys.argv) > 1 else "data/lab-walk-24076.mp4"
results = {}

for name in ["yolo11n.pt", "yolo11s.pt"]:
    model = YOLO(name)
    cap = cv2.VideoCapture(video)
    if not cap.isOpened():
        raise SystemExit(f"cannot open {video}")
    n, t_all = 0, 0.0
    persons = 0
    # 预热 10 帧（首次推理含 lazy init，不计入）
    warm = 0
    while True:
        ok, frame = cap.read()
        if not ok:
            break
        t0 = time.perf_counter()
        r = model.predict(frame, imgsz=640, classes=[0], verbose=False, device="cpu")
        t_all += time.perf_counter() - t0
        if warm >= 10:
            n += 1
            persons += len(r[0].boxes)
        warm += 1
    cap.release()
    results[name] = {
        "frames_measured": n,
        "fps": round(n / t_all, 1) if t_all else 0,
        "avg_persons_per_frame": round(persons / max(n, 1), 2),
    }
    print(f"{name}: {results[name]}")

print(json.dumps(results, indent=2))
