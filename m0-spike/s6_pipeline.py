# M0 / S6: 最小事件闭环：视频 → person 检测 → 多边形安全区判定 → 进入事件(冷却) → 截图存证
# 含 S4 的区域判定逻辑（cv2.pointPolygonTest，目标底边中点为锚点）
# 用法: python s6_pipeline.py <video.mp4>
import sys, time, json
from datetime import datetime
from pathlib import Path
import cv2
import numpy as np
from ultralytics import YOLO

video = sys.argv[1] if len(sys.argv) > 1 else "data/lab-walk-24076.mp4"
W, H = 1280, 720
# 演示安全区：画面中部一个多边形（真实产品里由用户在 Web 上绘制）
ZONE = [(520, 180), (1180, 180), (1180, 620), (520, 620)]
CONF = 0.35
COOLDOWN_S = 5.0
out_dir = Path("out_s6"); out_dir.mkdir(exist_ok=True)

model = YOLO("yolo11n.pt")
cap = cv2.VideoCapture(video)
fps_in = cap.get(cv2.CAP_PROP_FPS) or 30
events = []
in_zone_ids = set()
last_alert = {}

# 极简跟踪：IOU 匹配上一帧（演示用；正式产品用 ByteTrack）
def iou(a, b):
    ax1, ay1, ax2, ay2 = a; bx1, by1, bx2, by2 = b
    ix = max(0, min(ax2, bx2) - max(ax1, bx1))
    iy = max(0, min(ay2, by2) - max(ay1, by1))
    inter = ix * iy
    ua = (ax2-ax1)*(ay2-ay1) + (bx2-bx1)*(by2-by1) - inter
    return inter / ua if ua else 0

next_id, prev_boxes = 1, {}  # id -> box
def track(boxes):
    global next_id, prev_boxes
    cur = {}
    for b in boxes:
        best_id, best_v = None, 0.3
        for tid, pb in prev_boxes.items():
            v = iou(b, pb)
            if v > best_v:
                best_id, best_v = tid, v
        tid = best_id if best_id is not None else next_id
        if best_id is None:
            next_id += 1
        cur[tid] = b
    prev_boxes = cur
    return cur

poly = np.array(ZONE, np.int32)
frame_idx = 0
while True:
    ok, frame = cap.read()
    if not ok:
        break
    frame = cv2.resize(frame, (W, H))
    r = model.predict(frame, imgsz=640, classes=[0], conf=CONF, verbose=False)[0]
    boxes = [tuple(map(float, b.xyxy[0].tolist())) for b in r.boxes]
    tracked = track(boxes)
    now = frame_idx / fps_in
    for tid, (x1, y1, x2, y2) in tracked.items():
        anchor = (int((x1 + x2) / 2), int(y2))  # 底边中点
        inside = cv2.pointPolygonTest(poly, anchor, False) >= 0
        if inside and tid not in in_zone_ids:
            in_zone_ids.add(tid)
            if now - last_alert.get(tid, -1e9) >= COOLDOWN_S:
                last_alert[tid] = now
                ev = {
                    "t": round(now, 2), "track_id": tid,
                    "type": "ENTER_ZONE", "ppe": "unknown(MVP: labcoat model pending)",
                    "box": [round(v) for v in (x1, y1, x2, y2)],
                }
                events.append(ev)
                crop = frame[max(0,int(y1)-20):int(y2)+20, max(0,int(x1)-20):int(x2)+20]
                cv2.imwrite(str(out_dir / f"ev_{len(events):03d}_t{ev['t']}.jpg"), crop)
        elif not inside and tid in in_zone_ids:
            in_zone_ids.discard(tid)
    frame_idx += 1

cap.release()
print(json.dumps({"events": events[:20], "total": len(events)}, indent=2, ensure_ascii=False))
