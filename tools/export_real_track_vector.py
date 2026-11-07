"""T-324/L1：导出**真实检测序列**的 ByteTrack 轨迹向量（Python supervision = 真值）。

与 export_track_vectors.py（合成向量）互补：按 golden_run 同口径跑真实语料，
逐分析帧记录 ① 检测输入（xyxy+conf，engine.detect_persons 原序）② ByteTrack 输出
（tracker_id + xyxy）。Rust 侧 golden --dets-from 消费同一检测序列 + --dump-tracks，
逐帧 diff 轨迹分配，定位真实序列下的关联分歧帧（检测已证明逐位一致）。

用法:
    python tools/export_real_track_vector.py --video <mp4> --out testdata/vectors/real-<name>.json
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


class DetSpy:
    """记录 engine.detect_persons 的输入序输出（真值检测序列）。"""

    def __init__(self, inner):
        self.inner = inner

    def detect_persons(self, frame):
        return self.inner.detect_persons(frame)


def main() -> int:
    ap = argparse.ArgumentParser(description="真实检测序列轨迹向量导出（L1 对齐）")
    ap.add_argument("--video", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--name", default=None, help="向量用例名（默认视频文件名）")
    a = ap.parse_args()

    os.environ["LS_DETERMINISTIC"] = "1"
    os.environ["LS_LOG_FILE"] = "0"
    import cv2  # noqa: E402
    from app.config import settings  # noqa: E402
    from app.pipeline import CameraWorker  # noqa: E402
    from app.rules import RuleEngine  # noqa: E402
    from app.store import Store  # noqa: E402

    tmp = Path(tempfile.mkdtemp(prefix="ls-trackvec-"))
    settings.data_dir = tmp / "data"
    settings.snapshot_dir = settings.data_dir / "snapshots"
    settings.video_dir = settings.data_dir / "videos"
    for d in (settings.data_dir, settings.snapshot_dir, settings.video_dir):
        d.mkdir(parents=True, exist_ok=True)
    settings.analysis_fps = 5.0
    settings.zone_enter_frames = 2
    settings.zone_exit_frames = 3
    settings.ppe_model = ""

    store = Store(tmp / "vec.db")
    cid = store.add_camera("vec", str(a.video), "file")
    zid = store.add_zone(cid, "全画幅", FULL_ZONE)
    for behavior in ("enter", "exit", "enter_no_ppe"):
        store.add_rule({"camera_id": cid, "zone_id": zid, "behavior": behavior,
                        "level": 2, "cooldown_s": 0, "time_start": "00:00",
                        "time_end": "23:59", "threshold": 0, "enabled": True})
    cam = dict(store.camera(cid))
    w = CameraWorker(cam, store, RuleEngine(store))
    w.ppe = None

    from app.engine import Engine  # noqa: E402  真模型（与 golden 同口径）

    engine = Engine()

    # spy：detect 输出先记录（每分析帧一条），byteTrack 输出回填同帧 "tracks"。
    frames_log = []  # {"frame": i, "dets": [...], "tracks": [...] | None}

    class DetLogger:
        """包装 engine：记录每次 detect_persons 的输出（真值检测序列）。"""

        def __init__(self, inner, log):
            self.inner = inner
            self.log = log

        def detect_persons(self, frame):
            dets = self.inner.detect_persons(frame)
            rows = [[round(float(v), 2) for v in d.xyxy] + [round(float(d.conf), 4)]
                    for d in dets]
            self.log.append({"frame": len(self.log) + 1, "dets": rows, "tracks": None})
            return dets

    class TrackSpy:
        def __init__(self, inner, log):
            self.inner = inner
            self.log = log

        def update_with_detections(self, sdets):
            out = self.inner.update_with_detections(sdets)
            rows = []
            if len(out) > 0:
                for i in range(len(out)):
                    rows.append([round(float(v), 1) for v in out.xyxy[i]]
                                + [int(out.tracker_id[i])])
            rows.sort(key=lambda r: r[4])
            if self.log:
                self.log[-1]["tracks"] = rows
                tr = [(s.external_track_id, round(float(s.mean[0]), 1))
                      for s in self.inner.tracked_tracks]
                lo = [(s.external_track_id, round(float(s.mean[0]), 1))
                      for s in self.inner.lost_tracks]
                self.log[-1]["state"] = {"tracked": tr, "lost": lo}
            return out

    frames_log = []
    det_logger = DetLogger(engine, frames_log)
    w._byteTrack = TrackSpy(w._byteTrack, frames_log)

    cap = cv2.VideoCapture(str(a.video))
    if not cap.isOpened():
        print(f"[vec] 无法打开视频: {a.video}")
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
        if (fi - 1) % step:
            continue
        frame = cv2.resize(frame, (1280, 720))
        w._analyze(det_logger, frame)
        analyzed += 1
    cap.release()

    name = a.name or Path(a.video).stem
    cfg = {"track_activation_threshold": 0.25, "lost_track_buffer": 15,
           "minimum_matching_threshold": 0.8, "frame_rate": 5, "minimum_consecutive_frames": 1}
    out = {"comment": "T-324/L1 真实检测序列轨迹向量：Python supervision 真值；"
                      "Rust --dets-from 消费 dets 断言 tracks 逐帧一致",
           "name": name, "config": cfg, "frames": frames_log}
    outp = Path(a.out)
    outp.parent.mkdir(parents=True, exist_ok=True)
    outp.write_text(json.dumps(out, ensure_ascii=False), encoding="utf-8")
    n_tracks = sum(1 for f in frames_log if f["tracks"])
    print(f"[vec] video={a.video} analyzed={analyzed} frames_with_tracks={n_tracks} → {outp}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
