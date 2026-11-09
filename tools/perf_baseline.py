"""R2-306 / T-307：推理吞吐压测基线。

用法：
    python tools/perf_baseline.py --duration 60 --cameras 1,4,8
    python tools/perf_baseline.py --duration 1800 --cameras 1,4,8 --out docs/perf-baseline.md

测量口径（与 76 号 §13.5 / 78 号 §14.3 一致）：
- 每路 720p 文件源 @5fps，记录每路分析 fps、CPU%、RSS、句柄数；
- 实现矩阵：Python-CPU（本脚本直接测）、Python-DML（需 dml 变体环境）、
  Rust-tract / Rust-ort（需 ls-analyzer 产物，用 --golden  runner 计时）；
- 事件端到端 P95 由运行中服务的 /api/events 采样，属于 camera_soak.py 的职责，
  本脚本只测**单进程推理吞吐**（避免把网络/DB 抖动混进基线）。

本机不可测的实现会明确标为 "n/a (环境不满足)"，绝不编造数据。
"""
from __future__ import annotations

import argparse
import json
import os
import platform
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "server"))

VIDEO = ROOT / "m0-spike" / "data" / "lab-walk-24076.mp4"


def _machine() -> dict:
    import multiprocessing
    return {
        "os": platform.platform(),
        "python": sys.version.split()[0],
        "cpu_count": multiprocessing.cpu_count(),
        "onnxruntime": _ort_version(),
    }


def _ort_version() -> str:
    try:
        import onnxruntime as ort
        return f"{ort.__version__} providers={ort.get_available_providers()}"
    except Exception as e:  # noqa: BLE001
        return f"n/a ({type(e).__name__})"


def _rss_mb() -> float | None:
    try:
        import psutil
        return round(psutil.Process().memory_info().rss / 1e6, 1)
    except ImportError:
        return None


def bench_python(duration_s: int, cameras: int) -> dict:
    """Feed real frames through Engine.detect_persons(); N cameras = N sequential
    streams of the same video (worst-case CPU contention, as in production)."""
    import cv2
    from app.config import settings
    from app.engine import Engine

    model = settings.model_fp32
    if not model.exists():
        return {"ok": False, "reason": f"模型不存在: {model}"}
    if not VIDEO.exists():
        return {"ok": False, "reason": f"语料不存在: {VIDEO}"}
    eng = Engine()
    caps = [cv2.VideoCapture(str(VIDEO)) for _ in range(cameras)]
    frames = 0
    t0 = time.perf_counter()
    idx = 0
    while time.perf_counter() - t0 < duration_s:
        cap = caps[idx % cameras]
        ok, frame = cap.read()
        if not ok:
            cap.set(cv2.CAP_PROP_POS_FRAMES, 0)
            ok, frame = cap.read()
        if ok:
            eng.detect_persons(frame)
            frames += 1
        idx += 1
    elapsed = time.perf_counter() - t0
    for cap in caps:
        cap.release()
    return {
        "ok": True,
        "implementation": "python-cpu",
        "backend": eng.backend,
        "cameras": cameras,
        "duration_s": round(elapsed, 1),
        "frames": frames,
        "fps_total": round(frames / elapsed, 2),
        "fps_per_camera": round(frames / elapsed / cameras, 2),
        "rss_mb": _rss_mb(),
    }


def bench_rust(runner: Path, duration_s: int, cameras: int, label: str) -> dict:
    """Time ls-analyzer --golden on the same corpus (offline Rust pipeline)."""
    if not runner.is_file():
        return {"ok": False, "implementation": label,
                "reason": f"运行器不存在: {runner}（先 cargo build -p ls-analyzer）"}
    out = ROOT / "Resources" / "logs" / f"perf-{label}.jsonl"
    out.parent.mkdir(parents=True, exist_ok=True)
    import subprocess
    env = {**os.environ, "LS_DETERMINISTIC": "1", "LS_LOG_FILE": "0",
           "LS_INFER_BACKEND": ("ort" if label == "rust-ort" else "tract"),
           "LS_MODEL_FP32": str(ROOT / "m0-spike" / "yolo11n.onnx")}
    # ffprobe/ffmpeg are looked up by the Rust ingest layer via PATH.
    ffdir = str(ROOT / "tools" / "ffmpeg")
    env["PATH"] = ffdir + os.pathsep + env.get("PATH", "")
    t0 = time.perf_counter()
    r = subprocess.run(
        [str(runner), "--golden", "--video", str(VIDEO), "--out", str(out),
         "--fps", "5", "--model", str(ROOT / "m0-spike" / "yolo11n.onnx")],
        env=env, capture_output=True, timeout=duration_s + 600)
    elapsed = time.perf_counter() - t0
    if r.returncode != 0:
        return {"ok": False, "implementation": label, "reason":
                (r.stderr.decode("utf-8", "replace")[-300:] or f"exit {r.returncode}")}
    return {"ok": True, "implementation": label, "cameras": 1,
            "duration_s": round(elapsed, 1),
            "note": "offline golden runner; per-camera fps not comparable to online path"}


def main() -> int:
    ap = argparse.ArgumentParser(description="推理吞吐压测基线（R2-306）")
    ap.add_argument("--duration", type=int, default=60, help="每种实现测量秒数（默认 60；门槛用 1800）")
    ap.add_argument("--cameras", default="1,4,8", help="路数列表")
    ap.add_argument("--rust-exe", default=None, help="ls-analyzer 路径（默认按仓库 target 解析）")
    ap.add_argument("--out", type=Path, default=ROOT / "docs" / "perf-baseline.md")
    a = ap.parse_args()
    cams = [int(x) for x in a.cameras.split(",") if x.strip()]
    target = Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "engine-rs" / "target")))
    runner = Path(a.rust_exe) if a.rust_exe else (
        target / "debug" / ("ls-analyzer.exe" if os.name == "nt" else "ls-analyzer"))
    rows = []
    for n in cams:
        rows.append(bench_python(a.duration, n))
    for label in ("rust-tract", "rust-ort"):
        rows.append(bench_rust(runner, a.duration, 1, label))
    rows.append({"ok": False, "implementation": "python-dml",
                 "reason": "n/a (需 dml 变体环境：requirements-win-dml.lock + DirectX 12 GPU)"})
    report = {
        "generated_ts": time.strftime("%Y-%m-%d %H:%M:%S"),
        "machine": _machine(),
        "video": str(VIDEO.relative_to(ROOT)),
        "thresholds": {
            "cpu_4cam_fps_per_cam": 4.5,
            "dml_8cam_fps_per_cam": 4.5,
            "rust_vs_python": 1.5,
            "rust_rss_ratio": 0.6,
        },
        "rows": rows,
        "note": "完整 1/4/8 路 × 30 分钟矩阵需在目标硬件上重跑（--duration 1800）。",
    }
    a.out.write_text(
        "# 性能基线（R2-306 / T-307）\n\n"
        "```json\n" + json.dumps(report, ensure_ascii=False, indent=2) + "\n```\n\n"
        "门槛见 ADR-0009 §性能门槛；未达标前不推荐用户把 analyzer 切到 rust。\n",
        encoding="utf-8")
    print(f"[perf] 写入 {a.out}")
    for row in rows:
        print(" ", row.get("implementation"), "ok=" + str(row.get("ok")),
              row.get("fps_per_camera", row.get("reason", "")))
    return 0


if __name__ == "__main__":
    sys.exit(main())
