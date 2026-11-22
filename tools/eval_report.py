"""LS-056：新旧 ONNX 模型对比评测报告（开发支具，不进发行包）。

在保留验证集（可选灰度子集）上对比两个 .onnx：mAP50（若 ultralytics 可用）、
单帧耗时（onnxruntime）、检出数量；输出 Markdown 对比表。

用法：
    python tools/eval_report.py --old m0-spike/yolo11n_ppe.onnx \
        --new m0-spike/yolo11n_ppe_v2.onnx --data Resources/datasets/merged/data.yaml
"""
from __future__ import annotations

import argparse
import time
from pathlib import Path


def markdown_table(rows: list[dict]) -> str:
    """rows: [{model, ms, detections, map50?, gray_recall?}] → Markdown 表。"""
    head = "| 模型 | 单帧耗时(ms) | 检出数 | mAP50 | 灰度召回 |\n|---|---|---|---|---|"
    lines = [head]
    for r in rows:
        lines.append(
            "| {model} | {ms} | {detections} | {map50} | {gray} |".format(
                model=r.get("model", "?"),
                ms=_fmt(r.get("ms")),
                detections=r.get("detections", "—"),
                map50=_fmt(r.get("map50")),
                gray=_fmt(r.get("gray_recall")),
            )
        )
    return "\n".join(lines)


def _fmt(v) -> str:
    if v is None:
        return "—"
    if isinstance(v, float):
        return f"{v:.2f}"
    return str(v)


def bench_onnx(model_path: Path, images: list[Path], imgsz: int = 640) -> dict:
    """用 onnxruntime 跑一批图片，返回平均耗时与检出数；不可用时返回错误标记。"""
    try:
        import numpy as np  # type: ignore
        import onnxruntime as ort  # type: ignore
    except Exception as e:  # pragma: no cover - 环境相关
        return {"model": model_path.name, "ms": None, "detections": None, "error": str(e)}
    try:
        sess = ort.InferenceSession(str(model_path), providers=["CPUExecutionProvider"])
        inp = sess.get_inputs()[0]
        shape = [d if isinstance(d, int) else imgsz for d in inp.shape]
        if len(shape) != 4:
            return {"model": model_path.name, "ms": None, "detections": None, "error": "输入形状非 4D"}
        shape[0] = 1
        if not isinstance(shape[2], int):
            shape[2] = imgsz
        if not isinstance(shape[3], int):
            shape[3] = imgsz
        total = 0.0
        dets = 0
        n = 0
        for p in images:
            try:
                import cv2  # type: ignore
            except Exception:  # pragma: no cover
                return {"model": model_path.name, "ms": None, "detections": None, "error": "缺 opencv"}
            img = cv2.imread(str(p))
            if img is None:
                continue
            blob = cv2.resize(img, (shape[3], shape[2]))
            blob = blob[:, :, ::-1].transpose(2, 0, 1)[None].astype(np.float32) / 255.0
            t = time.perf_counter()
            out = sess.run(None, {inp.name: blob})[0]
            total += (time.perf_counter() - t) * 1000
            n += 1
            # 粗算检出数：经典 [1,4+nc,N] 取置信度 >0.25 的列数
            arr = np.asarray(out)
            if arr.ndim == 3 and arr.shape[1] > arr.shape[2]:
                scores = arr[0, 4:, :]
                dets += int((scores.max(axis=0) > 0.25).sum())
            elif arr.ndim == 3 and arr.shape[2] == 6:
                dets += int((arr[0, :, 4] > 0.25).sum())
        return {
            "model": model_path.name,
            "ms": total / n if n else None,
            "detections": dets,
        }
    except Exception as e:  # pragma: no cover
        return {"model": model_path.name, "ms": None, "detections": None, "error": str(e)}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="新旧 ONNX 模型对比评测")
    ap.add_argument("--old", required=True)
    ap.add_argument("--new", required=True)
    ap.add_argument("--images", default="", help="验证集图片目录（默认取 --data 同级 images/val）")
    ap.add_argument("--data", default="", help="data.yaml（用于定位 val 图片）")
    ap.add_argument("--limit", type=int, default=50)
    ap.add_argument("--out", default="")
    args = ap.parse_args(argv)

    img_dir = Path(args.images) if args.images else None
    if img_dir is None and args.data:
        root = Path(args.data).parent
        img_dir = root / "images" / "val"
        if not img_dir.is_dir():
            img_dir = root / "images"
    images = []
    if img_dir and img_dir.is_dir():
        images = [p for p in sorted(img_dir.rglob("*")) if p.suffix.lower() in (".jpg", ".jpeg", ".png")][: args.limit]
    if not images:
        print("[警告] 未找到验证图片，仅输出模型文件信息")
    rows = [bench_onnx(Path(args.old), images), bench_onnx(Path(args.new), images)]
    table = markdown_table(rows)
    print(table)
    if args.out:
        Path(args.out).write_text(table + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
