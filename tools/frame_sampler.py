# -*- coding: utf-8 -*-
"""数据飞轮 ①：从视频批量抽帧（按时间间隔 + 帧差去重）。

用法：
  python tools/frame_sampler.py --video m0-spike/data/lab-walk-24076.mp4 --out Resources/datasets/frames
  python tools/frame_sampler.py --video-dir m0-spike/data --out Resources/datasets/frames --interval 2.0 --max 300

输出：
  {video_stem}_{秒数:06.1f}.jpg   （如 lab-walk-24076_0003.2.jpg）
  manifest.csv                    （source,timestamp,frame_file,resolution）
"""
import argparse
import csv
import sys
from pathlib import Path

import cv2

VIDEO_EXTS = {".mp4", ".avi", ".mkv", ".mov", ".flv", ".ts", ".m4v"}


def sample_video(video: Path, out_dir: Path, interval: float, max_frames: int,
                 diff_thresh: float, manifest_rows: list) -> int:
    """抽帧单个视频。返回实际保留的帧数。

    去重逻辑：与"上一张保留帧"做灰度缩小 absdiff，平均差 < diff_thresh 视为静态画面丢弃。
    这样既按时间间隔采样，又能跳过长时间无变化的监控画面。
    """
    cap = cv2.VideoCapture(str(video))
    if not cap.isOpened():
        print(f"[skip] 无法打开: {video}")
        return 0
    fps = cap.get(cv2.CAP_PROP_FPS) or 25.0
    step = max(1, int(round(fps * interval)))
    w = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
    h = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
    kept, idx, prev_small = 0, 0, None
    while kept < max_frames:
        ok = cap.grab()  # 只解码需要的帧，跳帧更快
        if not ok:
            break
        if idx % step == 0:
            ok, frame = cap.retrieve()
            if not ok:
                break
            small = cv2.resize(cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY), (160, 90))
            sec = idx / fps
            if prev_small is None or \
                    float(cv2.absdiff(small, prev_small).mean()) >= diff_thresh:
                name = f"{video.stem}_{sec:06.1f}.jpg"
                path = out_dir / name
                # imencode+tofile：cv2.imwrite 依赖导入时的 PATH 注入（cv2 DLL），
                # 独立工具进程里即使 ASCII 路径也静默返回 False（任务41 实测）
                ok_jpg, buf = cv2.imencode(".jpg", frame)
                if ok_jpg:
                    buf.tofile(str(path))
                manifest_rows.append({
                    "source": str(video), "timestamp": f"{sec:.2f}",
                    "frame_file": name, "resolution": f"{w}x{h}",
                })
                kept += 1
                prev_small = small
        idx += 1
    cap.release()
    print(f"[{video.name}] 保留 {kept} 帧")
    return kept


def main():
    ap = argparse.ArgumentParser(description="视频批量抽帧（间隔采样 + 帧差去重）")
    ap.add_argument("--video", help="单个视频路径")
    ap.add_argument("--video-dir", help="视频目录（批量）")
    ap.add_argument("--out", required=True, help="帧输出目录")
    ap.add_argument("--interval", type=float, default=2.0, help="采样间隔秒（默认 2.0）")
    ap.add_argument("--max", type=int, default=300, help="每个视频最多保留帧数（默认 300）")
    ap.add_argument("--diff-thresh", type=float, default=8.0,
                    help="帧差去重阈值（0-255 灰度平均差，越小保留越多；默认 8.0）")
    args = ap.parse_args()

    if not args.video and not args.video_dir:
        ap.error("--video 与 --video-dir 至少给一个")
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    videos = []
    if args.video:
        videos.append(Path(args.video))
    if args.video_dir:
        videos += sorted(p for p in Path(args.video_dir).iterdir()
                         if p.suffix.lower() in VIDEO_EXTS)
    if not videos:
        print("没有找到视频文件")
        return 1

    rows: list[dict] = []
    total = 0
    for v in videos:
        total += sample_video(v, out_dir, args.interval, args.max, args.diff_thresh, rows)

    manifest = out_dir / "manifest.csv"
    with manifest.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=["source", "timestamp", "frame_file", "resolution"])
        writer.writeheader()
        writer.writerows(rows)
    print(f"完成：{len(videos)} 个视频，共 {total} 帧 → {out_dir}（manifest: {manifest}）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
