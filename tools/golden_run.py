"""T-321（L0）黄金语料运行器：把一段视频逐帧喂进真实分析管线，输出事件 JSONL。

用途（L0 对齐基准，76 文档 §7.2.4）：
  1) 确定性证明：同一语料对同一实现跑两次 → golden_diff 一致率必须 100%；
  2) L1 Rust sidecar 对齐：--impl python 与 --impl rust 的输出做同一 diff。

用法:
    python tools/golden_run.py --video m0-spike/data/lab-walk-24076.mp4 \
        --out Resources/logs/golden/py-a.jsonl
    python tools/golden_run.py --video m0-spike/data/lab-walk-24076.mp4 \
        --out Resources/logs/golden/py-b.jsonl
    python tools/golden_diff.py Resources/logs/golden/py-a.jsonl Resources/logs/golden/py-b.jsonl

铁律：
  - 强制 LS_DETERMINISTIC=1：事件 ts / 规则时间窗 / 日历 / 冷却全部取视频时间
    （固定合成基准 2026-01-01 + 帧 PTS），与墙钟零耦合；
  - 按 analysis_fps 抽帧（与生产口径一致）；
  - 推理走 server/app/engine.py 的 Engine（DML 全局串行锁），禁止绕过；
  - --no-model 用恒空引擎（CI 无模型冒烟确定性，不对齐检测质量）。
"""
import argparse
import json
import os
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "server"))

FULL_ZONE = [[0, 0], [1280, 0], [1280, 719], [0, 719]]


def main() -> int:
    ap = argparse.ArgumentParser(description="黄金语料运行器（确定性，L0 对齐基准）")
    ap.add_argument("--video", required=True, help="语料视频路径")
    ap.add_argument("--out", required=True, help="事件 JSONL 输出路径")
    ap.add_argument("--impl", choices=["python", "rust"], default="python",
                    help="实现：python=真实 Python 管线 / rust=转发 ls-analyzer --golden（T-321）")
    ap.add_argument("--fps", type=float, default=5.0, help="分析帧率（rust 实现转发 --fps）")
    ap.add_argument("--model", default=None, help="模型路径（rust 实现转发 --model）")
    ap.add_argument("--no-model", action="store_true",
                    help="用恒空引擎（无模型环境冒烟；不对齐检测质量）")
    a = ap.parse_args()

    os.environ["LS_DETERMINISTIC"] = "1"
    os.environ["LS_LOG_FILE"] = "0"

    # T-321：--impl rust 统一 CLI 转发到 Rust 离线运行器（事件 JSONL 格式一致，
    # 可直接喂 golden_diff.py 做同口径 diff；不再要求用户记两套参数）。
    if a.impl == "rust":
        import shutil
        import subprocess
        exe = os.environ.get("LS_ANALYZER_EXE") or \
            shutil.which("ls-analyzer") or \
            str(Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "engine-rs" / "target"))) /
                "debug" / ("ls-analyzer.exe" if os.name == "nt" else "ls-analyzer"))
        if not Path(exe).is_file():
            print(f"[golden] Rust 运行器不存在: {exe}")
            return 2
        out = Path(a.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        cmd = [str(exe), "--golden", "--video", str(Path(a.video).resolve()),
               "--out", str(out.resolve()), "--fps", str(a.fps)]
        if a.model:
            cmd += ["--model", str(Path(a.model).resolve())]
        if a.no_model:
            cmd.append("--no-model")
        env = {**os.environ}
        if not a.model and not a.no_model:
            env.setdefault("LS_MODEL_FP32", str(ROOT / "m0-spike" / "yolo11n.onnx"))
        # Rust 离线运行器用 ffprobe/ffmpeg 子进程；仓库自带工具的目录加入 PATH
        tools_ffmpeg = str(ROOT / "tools" / "ffmpeg")
        if Path(tools_ffmpeg).is_dir():
            env["PATH"] = tools_ffmpeg + os.pathsep + env.get("PATH", "")
        r = subprocess.run(cmd, env=env, capture_output=False)
        return r.returncode

    import cv2  # noqa: E402
    from app.config import settings  # noqa: E402
    from app.pipeline import CameraWorker  # noqa: E402
    from app.rules import RuleEngine  # noqa: E402
    from app.store import Store  # noqa: E402

    # 隔离数据目录（不碰真实事件库）
    tmp = Path(tempfile.mkdtemp(prefix="ls-golden-"))
    settings.data_dir = tmp / "data"
    settings.snapshot_dir = settings.data_dir / "snapshots"
    settings.video_dir = settings.data_dir / "videos"
    for d in (settings.data_dir, settings.snapshot_dir, settings.video_dir):
        d.mkdir(parents=True, exist_ok=True)
    settings.analysis_fps = 5.0
    settings.zone_enter_frames = 2
    settings.zone_exit_frames = 3
    settings.ppe_model = ""  # 语料对齐人体进出维度；PPE 维度走 T-334 评测口径

    store = Store(tmp / "golden.db")
    cid = store.add_camera("golden", str(a.video), "file")
    zid = store.add_zone(cid, "全画幅", FULL_ZONE)
    for behavior in ("enter", "exit", "enter_no_ppe"):
        store.add_rule({"camera_id": cid, "zone_id": zid, "behavior": behavior,
                        "level": 2, "cooldown_s": 0, "time_start": "00:00",
                        "time_end": "23:59", "threshold": 0, "enabled": True})
    cam = dict(store.camera(cid))
    w = CameraWorker(cam, store, RuleEngine(store))
    w.ppe = None  # 禁用 PPE 门控（与 ppe_model="" 一致）

    if a.no_model:
        class _Det:
            conf = 0.9

            def __init__(self, box):
                self.xyxy = [float(v) for v in box]

        class _FakeEngine:
            def detect_persons(self, frame):
                return []

        engine = _FakeEngine()
    else:
        from app.engine import Engine  # noqa: E402  真模型（DML 全局串行锁内）

        engine = Engine()

    cap = cv2.VideoCapture(str(a.video))
    if not cap.isOpened():
        print(f"[golden] 无法打开视频: {a.video}")
        return 2
    vfps = cap.get(cv2.CAP_PROP_FPS) or 25.0
    step = max(1, round(vfps / max(0.1, settings.analysis_fps)))
    fi = 0
    analyzed = 0
    while True:
        ok, frame = cap.read()
        if not ok:
            break
        fi += 1
        w._det_frame = {"ts_ms": float(cap.get(cv2.CAP_PROP_POS_MSEC)), "index": fi}
        if (fi - 1) % step:  # 按 analysis_fps 抽帧（确定性：按帧序号而非墙钟）
            continue
        frame = cv2.resize(frame, (1280, 720))
        w._analyze(engine, frame)
        analyzed += 1
    cap.release()

    rows = store.events(camera_id=cid, limit=100000)
    out = Path(a.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8") as f:
        for r in rows:
            f.write(json.dumps({
                "camera": r["camera_id"], "type": r["type"], "level": r["level"],
                "track_id": r["track_id"], "detail": r["detail"], "ts": r["ts"],
            }, ensure_ascii=False) + "\n")
    print(f"[golden] video={a.video} frames={fi} analyzed={analyzed} "
          f"events={len(rows)} → {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
