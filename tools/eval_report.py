# -*- coding: utf-8 -*-
"""数据飞轮 ④：模型在 test 集上的评估报告（Markdown 表，可多版本对比）。

用法（m0-venv）:
    python tools/eval_report.py --model m0-spike/yolo11n_ppe.onnx \
        --data Resources/datasets/merged/data.yaml --classes m0-spike/yolo11n_ppe.classes.txt
    python tools/eval_report.py --help

铁律：推理只走 server/app/engine.py 的 Engine（Engine._run 全局串行锁），禁止绕过。
输出：逐类 AP50/检出率/平均置信度 + 置信度分布 + Markdown 表（--report 追加对比行）。
"""
import argparse
import csv
import sys
from collections import defaultdict
from pathlib import Path

import cv2
import numpy as np

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "server"))

from app.engine import Engine  # noqa: E402  (统一推理入口，含串行锁)

IMG_EXTS = {".jpg", ".jpeg", ".png", ".bmp", ".webp"}
IOU_MATCH = 0.50


def load_classes(path: Path) -> list[str]:
    """与 tools/preannotate.py 相同的解析：一行一个类名，或 data.yaml 的 names。"""
    lines = [ln.strip() for ln in path.read_text(encoding="utf-8").splitlines()]
    if any(ln.startswith("names:") for ln in lines):
        return [ln[2:].strip().strip("'\"") for ln in lines if ln.startswith("- ")]
    return [ln for ln in lines if ln and not ln.startswith("#")]


def load_gt(label_path: Path, n_cls: int):
    """YOLO txt → {cls: [(cx,cy,w,h), ...]}；兼容分割多边形（>5 字段取外接框）。"""
    gt: dict[int, list] = defaultdict(list)
    if not label_path.exists():
        return gt
    for ln in label_path.read_text(encoding="utf-8").splitlines():
        p = ln.split()
        if len(p) < 5:
            continue
        ci = int(p[0])
        if not 0 <= ci < n_cls:
            continue
        vals = list(map(float, p[1:]))
        if len(vals) > 4 and len(vals) % 2 == 0:
            xs, ys = vals[0::2], vals[1::2]  # 多边形点序列 → 外接框
            x1, x2, y1, y2 = min(xs), max(xs), min(ys), max(ys)
            vals = [(x1 + x2) / 2, (y1 + y2) / 2, x2 - x1, y2 - y1]
        gt[ci].append(tuple(vals[:4]))
    return gt


def iou_xywh(a, b) -> float:
    ax1, ay1 = a[0] - a[2] / 2, a[1] - a[3] / 2
    ax2, ay2 = a[0] + a[2] / 2, a[1] + a[3] / 2
    bx1, by1 = b[0] - b[2] / 2, b[1] - b[3] / 2
    bx2, by2 = b[0] + b[2] / 2, b[1] + b[3] / 2
    ix = max(0, min(ax2, bx2) - max(ax1, bx1))
    iy = max(0, min(ay2, by2) - max(ay1, by1))
    inter = ix * iy
    union = a[2] * a[3] + b[2] * b[3] - inter
    return inter / union if union > 0 else 0.0


def match_and_count(dets, gt, classes, stats):
    """检测框按置信度降序与 GT 做 IoU 0.5 贪心匹配 → 更新逐类 TP/FP/GT 计数。"""
    matched = set()
    for ci, box, conf in sorted(dets, key=lambda d: -d[2]):
        name = classes[ci]
        best_iou, best_j = 0.0, -1
        for j, g in enumerate(gt.get(ci, [])):
            if j in matched:
                continue
            iou = iou_xywh(box, g)
            if iou > best_iou:
                best_iou, best_j = iou, j
        if best_iou >= IOU_MATCH:
            matched.add(best_j)
            stats[name]["tp"] += 1
            stats[name]["confs"].append(conf)
        else:
            stats[name]["fp"] += 1
    for ci, gts in gt.items():
        stats[classes[ci]]["gt"] += len(gts)  # 未匹配上的 GT 即 FN（检出率按 TP/GT 看）


def main():
    ap = argparse.ArgumentParser(description="模型 test 集评估 → Markdown 报告（飞轮④）")
    ap.add_argument("--model", required=True, help="ONNX 模型路径")
    ap.add_argument("--data", required=True, help="data.yaml（取 test/valid 图片目录）")
    ap.add_argument("--classes", required=True, help="类名文件（与模型输出顺序一致）")
    ap.add_argument("--split", default="test", choices=["test", "valid"], help="评估哪个划分（默认 test）")
    ap.add_argument("--conf", type=float, default=0.25, help="评估置信度阈值（默认 0.25）")
    ap.add_argument("--limit", type=int, default=0, help="只评估前 N 张（0=全部；冒烟用）")
    ap.add_argument("--label", default=None, help="报告行标签（默认模型文件名）")
    ap.add_argument("--report", default=None, help="Markdown 报告路径（追加对比表）")
    ap.add_argument("--provider", default=None, choices=[None, "cpu"],
                    help="推理提供者；默认按 settings 顺序，'cpu' 强制 CPUExecutionProvider"
                         "（新导出模型在 DirectML 下可能触发 ORT 兼容问题时使用）")
    args = ap.parse_args()

    import yaml
    data_cfg = yaml.safe_load(Path(args.data).read_text(encoding="utf-8"))
    img_dir = Path(data_cfg.get(args.split) or "")
    if not img_dir.is_dir():
        sys.exit(f"[error] data.yaml 的 {args.split} 目录不存在: {img_dir}")

    classes = load_classes(Path(args.classes))
    engine = Engine(model_path=args.model, prefer="cpu") if args.provider == "cpu" \
        else Engine(model_path=args.model)

    if args.limit:
        import shutil
        tmp = Path(args.data).parent / f"_eval_{args.limit}"
        if tmp.exists():
            shutil.rmtree(tmp)
        (tmp).mkdir(parents=True)
        for p in sorted(p for p in img_dir.iterdir() if p.suffix.lower() in IMG_EXTS)[:args.limit]:
            shutil.copy2(p, tmp / p.name)
        lbl_dst = tmp.parent / f"_eval_{args.limit}_labels"
        lbl_src = img_dir.parent / "labels"
        if not lbl_dst.exists() and lbl_src.exists():
            lbl_dst.mkdir()
            for p in sorted(lbl_src.iterdir())[:args.limit]:
                shutil.copy2(p, lbl_dst / p.name)
        img_dir, labels_dir = tmp, lbl_dst
    else:
        labels_dir = img_dir.parent / "labels"

    stats = {c: {"gt": 0, "tp": 0, "fp": 0, "confs": []} for c in classes}
    images = sorted(p for p in img_dir.iterdir() if p.suffix.lower() in IMG_EXTS)
    for img_path in images:
        img = cv2.imdecode(np.fromfile(str(img_path), dtype=np.uint8), cv2.IMREAD_COLOR)
        if img is None:
            continue
        h, w = img.shape[:2]
        # 检测：像素级 xyxy（与 preannotate.detect_all 同一套换算），再归一化做 IoU
        canvas, r, dx, dy = engine._letterbox(img)
        rgb = cv2.cvtColor(canvas, cv2.COLOR_BGR2RGB).astype(np.float32) / 255.0
        blob = np.transpose(rgb, (2, 0, 1))[None]
        pred = engine._run(blob)[0][0].T  # (N, 4+nc)
        dets = []
        for ci in range(len(classes)):
            scores = pred[:, 4 + ci]
            m = scores > args.conf
            if not m.any():
                continue
            boxes_px = np.stack([
                (pred[m, 0] - pred[m, 2] / 2 - dx) / r,
                (pred[m, 1] - pred[m, 3] / 2 - dy) / r,
                (pred[m, 0] + pred[m, 2] / 2 - dx) / r,
                (pred[m, 1] + pred[m, 3] / 2 - dy) / r,
            ], axis=1)
            keep = engine._nms(boxes_px, scores[m], 0.5)
            for i in keep:
                x1, y1, x2, y2 = boxes_px[i].tolist()
                cx, cy = (x1 + x2) / 2 / w, (y1 + y2) / 2 / h
                bw, bh = max(0, (x2 - x1)) / w, max(0, (y2 - y1)) / h
                dets.append((ci, (cx, cy, bw, bh), float(scores[m][i])))

        gt = load_gt(labels_dir / (img_path.stem + ".txt"), len(classes))
        match_and_count(dets, gt, classes, stats)

    # ---- 输出 ----
    label = args.label or Path(args.model).stem
    lines = [
        f"## 评估：{label}（{args.split}，{len(images)} 图，conf≥{args.conf}，IoU≥{IOU_MATCH}）",
        "",
        "| 类别 | GT数 | TP | FP | 检出率(P) | 平均置信度 |",
        "|---|---|---|---|---|---|",
    ]
    for c in classes:
        s = stats[c]
        p = s["tp"] / (s["tp"] + s["fp"]) if (s["tp"] + s["fp"]) else 0.0
        conf = sum(s["confs"]) / len(s["confs"]) if s["confs"] else 0.0
        lines.append(f"| {c} | {s['gt']} | {s['tp']} | {s['fp']} | {p:.1%} | {conf:.3f} |")

    out = "\n".join(lines)
    print(out)
    if args.report:
        rp = Path(args.report)
        with open(rp, "a", encoding="utf-8") as f:
            f.write("\n" + out + "\n")
        print(f"[done] 已追加到 {rp}")


if __name__ == "__main__":
    main()
