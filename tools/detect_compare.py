"""T-321 L1 对齐诊断：同帧对比 Python Engine 与 Rust tract 的检测框 IoU。

Python 与 Rust 跑同一视频同一步长抽帧，各自输出每分析帧 person 框；
按贪心 IoU 匹配后打印最低 IoU/分数差分布（§13.8 要求 IoU ≥ 0.99）。
用法: python tools/detect_compare.py --video <mp4> [--fps 5]
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "server"))

import cv2  # noqa: E402

# R2-506: no developer-machine paths; allow LS_ANALYZER_EXE / CARGO_TARGET_DIR.
_suffix = ".exe" if os.name == "nt" else ""
RS_EXE = Path(os.environ.get(
    "LS_ANALYZER_EXE",
    str(Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "engine-rs" / "target")))
        / "debug" / f"ls-analyzer{_suffix}")))
FFMPEG = ROOT / "tools" / "ffmpeg" / "ffmpeg.exe"
MODEL = ROOT / "m0-spike" / "yolo11n.onnx"


def iou(a, b):
    ix = max(0.0, min(a[2], b[2]) - max(a[0], b[0]))
    iy = max(0.0, min(a[3], b[3]) - max(a[1], b[1]))
    inter = ix * iy
    ua = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter
    return inter / ua if ua > 0 else 0.0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--video", required=True)
    ap.add_argument("--fps", type=float, default=5.0)
    a = ap.parse_args()

    out = subprocess.run([str(ROOT / "tools/ffmpeg/ffprobe.exe"), "-v", "error",
                          "-select_streams", "v:0", "-show_entries", "stream=r_frame_rate",
                          "-of", "csv=p=0", a.video], capture_output=True, text=True).stdout
    num, den = out.strip().split("/")
    vfps = float(num) / float(den)
    step = max(1, round(vfps / a.fps))

    # Rust 侧：--golden --dump-dets
    dump = Path(tempfile.mkdtemp(prefix="ls-cmp-")) / "rs-dets.jsonl"
    env = {**os.environ, "PATH": str(FFMPEG.parent) + os.pathsep + os.environ["PATH"]}
    r = subprocess.run([str(RS_EXE), "--golden", a.video, "--out", str(dump.parent / "rs-ev.jsonl"),
                        "--fps", str(a.fps), "--model", str(MODEL), "--dump-dets", str(dump)],
                       capture_output=True, text=True, env=env)
    if r.returncode != 0:
        print(r.stdout, r.stderr)
        return 2
    rs = {j["frame"]: list(zip(j["boxes"], j["scores"]))
          for j in map(json.loads, dump.read_text(encoding="utf-8").splitlines())}

    # Python 侧：真模型逐帧
    os.environ.setdefault("LS_DETERMINISTIC", "1")
    from app.engine import Engine
    engine = Engine()
    cap = cv2.VideoCapture(a.video)
    fi = analyzed = 0
    n_pair = n_match = 0
    ious, score_diffs = [], []
    worst = []
    while True:
        ok, frame = cap.read()
        if not ok:
            break
        fi += 1
        if (fi - 1) % step:
            continue
        analyzed += 1
        f = cv2.resize(frame, (1280, 720))
        py = [([float(v) for v in d.xyxy], float(d.conf)) for d in engine.detect_persons(f)]
        rdet = rs.get(analyzed, [])
        rboxes = [([float(v) for v in b], float(s)) for b, s in rdet]
        for pb, pc in py:
            best, bi = 0.0, None
            for rb, rc in rboxes:
                v = iou(pb, rb)
                if v > best:
                    best, bi = v, (rb, rc)
            n_pair += 1
            if bi is not None and best > 0.3:
                n_match += 1
                ious.append(best)
                score_diffs.append(abs(pc - bi[1]))
                if best < 0.995:
                    worst.append((analyzed, round(best, 4), pb, bi[0]))
            else:
                worst.append((analyzed, 0.0, pb, None))
    cap.release()
    print(f"[cmp] analyzed={analyzed} py_boxes={n_pair} matched={n_match}")
    if ious:
        ious.sort()
        print(f"[cmp] IoU min={ious[0]:.4f} p50={ious[len(ious)//2]:.4f} "
              f">=0.99 比例={sum(1 for v in ious if v >= 0.99)}/{len(ious)}")
        print(f"[cmp] |score diff| max={max(score_diffs):.4f}")
    for w in worst[:10]:
        print("  low-iou:", w)
    return 0


if __name__ == "__main__":
    sys.exit(main())
