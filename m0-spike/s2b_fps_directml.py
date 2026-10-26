# M0 / S2b: YOLO11n via ONNX Runtime DirectML (Radeon 880M)
# 用法: 先 python export_onnx.py 导出 yolo11n.onnx；再 python s2b_fps_directml.py <video.mp4>
# 依赖: pip install onnxruntime-directml numpy opencv-python
import sys, time, json
import numpy as np
import cv2
import onnxruntime as ort

video = sys.argv[1] if len(sys.argv) > 1 else "data/lab-walk-24076.mp4"
onnx_path = sys.argv[2] if len(sys.argv) > 2 else "yolo11n.onnx"
CONF, IOU, SIZE = 0.35, 0.5, 640

avail = ort.get_available_providers()
print("available providers:", avail)
providers = ["DmlExecutionProvider", "CPUExecutionProvider"]
sess = ort.InferenceSession(onnx_path, providers=providers)
print("active providers:", sess.get_providers())
inp_name = sess.get_inputs()[0].name

def letterbox(img, s=SIZE):
    h, w = img.shape[:2]
    r = min(s / h, s / w)
    nh, nw = int(h * r), int(w * r)
    img_r = cv2.resize(img, (nw, nh))
    canvas = np.full((s, s, 3), 114, np.uint8)
    top, left = (s - nh) // 2, (s - nw) // 2
    canvas[top:top+nh, left:left+nw] = img_r
    return canvas, r, left, top

def nms(boxes, scores, iou):
    x1, y1, x2, y2 = boxes[:,0], boxes[:,1], boxes[:,2], boxes[:,3]
    areas = (x2-x1) * (y2-y1)
    order = scores.argsort()[::-1]
    keep = []
    while order.size:
        i = order[0]; keep.append(i)
        xx1 = np.maximum(x1[i], x1[order[1:]]); yy1 = np.maximum(y1[i], y1[order[1:]])
        xx2 = np.minimum(x2[i], x2[order[1:]]); yy2 = np.minimum(y2[i], y2[order[1:]])
        inter = np.maximum(0, xx2-xx1) * np.maximum(0, yy2-yy1)
        ious = inter / (areas[i] + areas[order[1:]] - inter + 1e-9)
        order = order[1:][ious <= iou]
    return np.array(keep, dtype=int)

def detect(frame):
    canvas, r, dx, dy = letterbox(frame)
    rgb = cv2.cvtColor(canvas, cv2.COLOR_BGR2RGB).astype(np.float32) / 255.0
    blob = np.transpose(rgb, (2, 0, 1))[None]
    out = sess.run(None, {inp_name: blob})[0][0]  # (84, 8400) COCO
    pred = out.T  # (8400, 84): cx,cy,w,h + 80 cls
    person = pred[:, 4]  # class 0 = person（COCO 首类）
    m = person > CONF
    boxes = np.stack([
        (pred[m,0] - pred[m,2]/2 - dx) / r, (pred[m,1] - pred[m,3]/2 - dy) / r,
        (pred[m,0] + pred[m,2]/2 - dx) / r, (pred[m,1] + pred[m,3]/2 - dy) / r,
    ], axis=1)
    scores = person[m]
    keep = nms(boxes, scores, IOU)
    return boxes[keep], scores[keep]

cap = cv2.VideoCapture(video)
n, t_all, persons, warm = 0, 0.0, 0, 0
times = []
while True:
    ok, frame = cap.read()
    if not ok:
        break
    t0 = time.perf_counter()
    boxes, scores = detect(frame)
    dt = time.perf_counter() - t0
    if warm >= 10:
        t_all += dt; n += 1; persons += len(boxes); times.append(dt)
    warm += 1
cap.release()
res = {
    "provider": sess.get_providers()[0],
    "frames_measured": n,
    "fps": round(n / t_all, 1) if t_all else 0,
    "p50_ms": round(float(np.median(times)) * 1000, 1),
    "avg_persons_per_frame": round(persons / max(n, 1), 2),
}
print(json.dumps(res, indent=2))
