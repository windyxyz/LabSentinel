# -*- coding: utf-8 -*-
"""数据飞轮 ②：用当前部署模型给抽帧做预标注（YOLO 检测框 txt + 目检 preview）。

用法：
  python tools/preannotate.py --images datasets/frames --out datasets/labels \
      --model m0-spike/yolo11n_ppe.onnx --classes-file datasets/lab-ppe/classes.txt

铁律：推理必须走 server/app/engine.py 的 Engine（统一入口 Engine._run，DirectML 全局串行锁），
不得绕过引擎直接调用底层会话 —— 多线程并发 DML 推理会挂死 GPU 驱动（2026-10-26 事故根因）。
"""
import argparse
import sys
from collections import defaultdict
from pathlib import Path

import cv2
import numpy as np

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "server"))  # 复用服务端引擎（含串行锁与 letterbox）

from app.engine import Engine  # noqa: E402

IMG_EXTS = {".jpg", ".jpeg", ".png", ".bmp", ".webp"}


def load_classes(path: Path) -> list[str]:
    """一行一个类名；兼容 data.yaml 的 names 列表（从文件中找 `names:` 后逐行提取 - 项）。"""
    lines = [ln.strip() for ln in path.read_text(encoding="utf-8").splitlines()]
    if any(ln.startswith("names:") for ln in lines):
        names = []
        for ln in lines:
            if ln.startswith("- "):
                names.append(ln[2:].strip().strip("'\""))
        return names
    return [ln for ln in lines if ln and not ln.startswith("#")]


def detect_all(engine: Engine, frame, classes: list[str], conf_thres: float):
    """一次前向解出全部类别（letterbox + Engine._run 串行入口 + NMS）。

    与 Engine.detect_class 同思路，但一次拿回所有类别的框，避免每类一次前向。
    """
    canvas, r, dx, dy = engine._letterbox(frame)
    rgb = cv2.cvtColor(canvas, cv2.COLOR_BGR2RGB).astype(np.float32) / 255.0
    blob = np.transpose(rgb, (2, 0, 1))[None]
    out = engine._run(blob)[0][0]  # (4+num_cls, N)
    pred = out.T
    results = []
    for ci in range(len(classes)):
        scores = pred[:, 4 + ci]
        m = scores > conf_thres
        if not m.any():
            continue
        boxes = np.stack([
            (pred[m, 0] - pred[m, 2] / 2 - dx) / r,
            (pred[m, 1] - pred[m, 3] / 2 - dy) / r,
            (pred[m, 0] + pred[m, 2] / 2 - dx) / r,
            (pred[m, 1] + pred[m, 3] / 2 - dy) / r,
        ], axis=1)
        keep = engine._nms(boxes, scores[m], 0.5)
        for i in keep:
            results.append((ci, boxes[i].tolist(), float(scores[m][i])))
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

    engine = Engine(model_path=args.model)

    counts: dict[str, int] = defaultdict(int)
    conf_sum: dict[str, float] = defaultdict(float)
    for img_path in images:
        # cv2.imread 不支持中文路径，统一 imdecode
        img = cv2.imdecode(np.fromfile(str(img_path), dtype=np.uint8), cv2.IMREAD_COLOR)
        if img is None:
            print(f"[skip] 读图失败: {img_path.name}")
            continue
        h, w = img.shape[:2]
        dets = detect_all(engine, img, classes, args.conf)
        lines = []
        for ci, (x1, y1, x2, y2), conf in dets:
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
        # cv2.imwrite 对含中文的路径静默失败（项目路径含「监控识别」），统一 imencode + Python 写盘
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
