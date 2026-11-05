"""T-324：导出 ByteTrack 轨迹一致性共享向量（Python supervision = 真值实现）。

生成 testdata/vectors/track_sequences.json：合成检测序列（单人/双人交叉/遮挡重现/
低分补救/轨迹交换防护），Rust ls-track 消费同一向量断言逐帧 (id, box) 一致。
运行（项目 venv，含 supervision）:
    python tools/export_track_vectors.py
"""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "server"))

import numpy as np  # noqa: E402
import supervision as sv  # noqa: E402

OUT = ROOT / "testdata" / "vectors" / "track_sequences.json"

CFG = {"track_activation_threshold": 0.25, "lost_track_buffer": 30,
       "minimum_matching_threshold": 0.8, "frame_rate": 30, "minimum_consecutive_frames": 1}


def _seq(name, frames):
    return {"name": name, "frames": frames}


def case_single():
    frames = []
    for i in range(30):
        x = 100 + i * 4
        frames.append([[x, 100, x + 200, 500, 0.9]])
    return _seq("single-walk", frames)


def case_two_crossing():
    frames = []
    ax, bx = 60.0, 1000.0
    for i in range(40):
        ax += 22
        bx -= 22
        frames.append([[ax, 100, ax + 150, 480, 0.92], [bx, 150, bx + 150, 520, 0.88]])
    return _seq("two-crossing", frames)


def case_occlusion_gap():
    frames = []
    for i in range(10):
        frames.append([[100 + i * 3, 100, 300 + i * 3, 500, 0.9]])
    for _ in range(12):  # 遮挡 12 帧（< lost_track_buffer=30）
        frames.append([])
    for i in range(10):
        frames.append([[140 + i * 3, 100, 340 + i * 3, 500, 0.9]])
    return _seq("occlusion-reappear", frames)


def case_low_score_rescue():
    """高置信度丢一帧后以低分(0.15~0.25)出现——第二轮关联应接回同 id。"""
    frames = []
    for i in range(8):
        frames.append([[200 + i * 5, 100, 400 + i * 5, 500, 0.9]])
    frames.append([[235, 100, 435, 500, 0.2]])  # 低分帧（第二轮关联）
    for i in range(8):
        frames.append([[240 + i * 5, 100, 440 + i * 5, 500, 0.9]])
    return _seq("low-score-rescue", frames)


def case_unconfirmed_false_start():
    """单帧闪现（低分）+ 之后不再出现——unconfirmed 被移除，不产生长期轨迹。"""
    frames = [[[500, 100, 700, 500, 0.3]]]
    for _ in range(10):
        frames.append([])
    return _seq("false-start", frames)


def main():
    cases = [case_single(), case_two_crossing(), case_occlusion_gap(),
             case_low_score_rescue(), case_unconfirmed_false_start()]
    out_cases = []
    for case in cases:
        tracker = sv.ByteTrack(**CFG)
        frames_out = []
        for dets in case["frames"]:
            arr = np.asarray(dets, dtype=np.float32).reshape(-1, 5)
            xyxy = arr[:, :4] if arr.size else np.empty((0, 4), dtype=np.float32)
            conf = arr[:, 4] if arr.size else np.empty((0,), dtype=np.float32)
            d = sv.Detections(xyxy=xyxy, confidence=conf)
            d = tracker.update_with_detections(d)
            rows = []
            if len(d) > 0:
                for i in range(len(d)):
                    rows.append([round(float(v), 1) for v in d.xyxy[i]]
                                + [int(d.tracker_id[i])])
            rows.sort(key=lambda r: r[4])
            frames_out.append(rows)
        out_cases.append({"name": case["name"], "config": CFG, "frames": frames_out})
        ids = {r[4] for fr in frames_out for r in fr}
        print(f"[export] {case['name']}: frames={len(frames_out)} ids={sorted(ids)}")
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps({"comment": "T-324 ByteTrack 轨迹向量：Python supervision 0.25.1 真值；Rust 逐帧断言 (id, box) 一致",
                               "cases": out_cases}, ensure_ascii=False, indent=1), encoding="utf-8")
    print(f"[export] → {OUT}")


if __name__ == "__main__":
    main()
