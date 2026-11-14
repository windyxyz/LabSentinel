"""数据飞轮 ②：用当前部署模型给抽帧做预标注（YOLO 检测框 txt + 目检 preview）。

用法：
  python tools/preannotate.py --images Resources/datasets/frames --out Resources/datasets/labels \
      --model m0-spike/yolo11n_ppe.onnx --classes-file Resources/datasets/lab-ppe/classes.txt

独立 onnxruntime 实现（不依赖已退役的 Python 服务端）：单进程单线程顺序推理，
不与在线服务并发；有 DirectML 时优先用它，否则 CPU。
"""
import argparse
import sys
from collections import defaultdict
from pathlib import Path

import cv2
import numpy as np

IMG_EXTS = {".jpg", ".jpeg", ".png", ".bmp", ".webp"}
PAD = 114


def load_classes(path: Path) -> list[str]:
    """一行一个类名；兼容 data.yaml 的 names 列表。"""
    lines = [ln.strip() for ln in path.read_text(encoding="utf-8").splitlines()]
    if any(ln.startswith("names:") for ln in lines):
        return [ln[2:].strip().strip("'\"") for ln in lines if ln.startswith("- ")]
    return [ln for ln in lines if ln and not ln.startswith("#")]


def make_session(model: str):
    import onnxruntime as ort

    avail = ort.get_available_providers()
    providers = []
    if "DmlExecutionProvider" in avail:
        providers.append("DmlExecutionProvider")
    providers.append("CPUExecutionProvider")
    return ort.InferenceSession(model, providers=providers)


def letterbox(img: np.ndarray, size: int):
    h, w = img.shape[:2]
    r = min(size / w, size / h)
    nw, nh = int(round(w * r)), int(round(h * r))
    resized = cv2.resize(img, (nw, nh), interpolation=cv2.INTER_LINEAR)
    canvas = np.full((size, size, 3), PAD, dtype=np.uint8)
    dx, dy = (size - nw) // 2, (size - nh) // 2
    canvas[dy:dy + nh, dx:dx + nw] = resized
    return canvas, r, dx, dy


def detect_all(sess, frame: np.ndarray, n_classes: int, conf_thres: float):
    inp = sess.get_inputs()[0]
    shape = inp.shape
    size = int(shape[2]) if len(shape) == 4 and isinstance(shape[2], int) else 640
    canvas, r, dx, dy = letterbox(frame, size)
    rgb = cv2.cvtColor(canvas, cv2.COLOR_BGR2RGB).astype(np.float32) / 255.0
    blob = np.transpose(rgb, (2, 0, 1))[None]
    out = sess.run(None, {inp.name: blob})[0]
    pred = out[0]
    if pred.shape[0] < pred.shape[1]:
        pred = pred.T  # (N, 4+C)
    results = []
    for ci in range(n_classes):
        if 4 + ci >= pred.shape[1]:
            break
        scores = pred[:, 4 + ci]
        m = scores > conf_thres
        if not m.any():
            continue
        boxes = np.stack([
            (pred[m, 0] - pred[m, 2] / 2 - dx) / r,
            (pred[m, 1] - pred[m, 3] / 2 - dy) / r,
            pred[m, 2] / r,
            pred[m, 3] / r,
        ], axis=1)
        keep = cv2.dnn.NMSBoxes(boxes.tolist(), scores[m].tolist(), conf_thres, 0.5)
        for i in np.array(keep).flatten():
            x, y, bw, bh = boxes[i]
            results.append((ci, [x, y, x + bw, y + bh], float(scores[m][i])))
    return results


def main():
    ap = argparse.ArgumentParser(description="用部署模型对图片做 YOLO 预标注")
    ap.add_argument("--images", required=True, help="图片目录")
    ap.add_argument("--out", required=True, help="YOLO 标签输出目录")
    ap.add_argument("--model", required=True, help="ONNX 模型路径")
    ap.add_argument("--classes-file", required=True, help="类名文件（一行一个，或 data.yaml）")
    ap.add_argument("--conf", type=float, default=0.25, help="预标注置信度阈值（默认 0.25）")
    args = ap.parse_args()

    img_dir, out_dir = Path(args.images), Path(args.out)
    if not img_dir.is_dir():
        print(f"图片目录不存在: {img_dir}")
        return 1
    out_dir.mkdir(parents=True, exist_ok=True)
    preview_dir = out_dir / "preview"
    preview_dir.mkdir(exist_ok=True)

    classes = load_classes(Path(args.classes_file))
    if not classes:
        print(f"类名文件为空或格式不对: {args.classes_file}")
        return 1
    images = sorted(p for p in img_dir.iterdir() if p.suffix.lower() in IMG_EXTS)
    if not images:
        print("图片目录为空（manifest.csv 会被跳过）")
        return 0

    sess = make_session(args.model)
    counts: dict[str, int] = defaultdict(int)
    conf_sum: dict[str, float] = defaultdict(float)
    for img_path in images:
        img = cv2.imdecode(np.fromfile(str(img_path), dtype=np.uint8), cv2.IMREAD_COLOR)
        if img is None:
            print(f"[skip] 读图失败: {img_path.name}")
            continue
        h, w = img.shape[:2]
        lines = []
        for ci, (x1, y1, x2, y2), conf in detect_all(sess, img, len(classes), args.conf):
            cx, cy = (x1 + x2) / 2 / w, (y1 + y2) / 2 / h
            bw, bh = (x2 - x1) / w, (y2 - y1) / h
            lines.append(f"{ci} {cx:.6f} {cy:.6f} {bw:.6f} {bh:.6f}")
            name = classes[ci]
            counts[name] += 1
            conf_sum[name] += conf
            cv2.rectangle(img, (int(x1), int(y1)), (int(x2), int(y2)), (0, 200, 255), 2)
            cv2.putText(img, f"{name} {conf:.2f}", (int(x1), max(12, int(y1) - 4)),
                        cv2.FONT_HERSHEY_SIMPLEX, 0.5, (0, 200, 255), 1)
        out_dir.joinpath(img_path.stem + ".txt").write_text("\n".join(lines), encoding="utf-8")
        ext = img_path.suffix.lower() if img_path.suffix.lower() in (".jpg", ".jpeg", ".png") else ".jpg"
        ok, buf = cv2.imencode(ext, img)
        if ok:
            (preview_dir / (img_path.stem + ext)).write_bytes(buf.tobytes())

    print(f"完成：{len(images)} 张图 → {out_dir}（preview 见 {preview_dir}）")
    for name in sorted(counts):
        print(f"  {name}: {counts[name]} 框, 平均置信度 {conf_sum[name] / counts[name]:.3f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
