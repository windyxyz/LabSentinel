# M0 / S3: 用 COCO person 在两段实验室视频上跑检测，抽样保存标注帧，人工评估
# 用法: python s3_detect_lab.py
import cv2, json
from pathlib import Path
from ultralytics import YOLO

model = YOLO("yolo11n.pt")
out_dir = Path("out_s3"); out_dir.mkdir(exist_ok=True)
summary = {}
for video in ["data/lab-walk-24076.mp4", "data/lab-equip-21453.mp4"]:
    cap = cv2.VideoCapture(video)
    total, with_person = 0, 0
    samples = []
    idx = 0
    while True:
        ok, frame = cap.read()
        if not ok:
            break
        if idx % 5 == 0:  # 每秒约 6 帧分析 1 帧
            r = model.predict(frame, imgsz=640, classes=[0], conf=0.3, verbose=False)[0]
            total += 1
            n_person = len(r.boxes)
            if n_person:
                with_person += 1
                if len(samples) < 3:
                    annotated = r.plot()
                    samples.append(out_dir / f"{Path(video).stem}_f{idx}.jpg")
                    cv2.imwrite(str(samples[-1]), annotated)
        idx += 1
    cap.release()
    summary[video] = {
        "analyzed_frames": total,
        "frames_with_person": with_person,
        "ratio": round(with_person / max(total, 1), 2),
        "samples": [s.name for s in samples],
    }
print(json.dumps(summary, indent=2, ensure_ascii=False))
