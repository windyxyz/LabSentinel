"""摄像头 24h 稳定性验收采样器（任务42）：只读 API，产出 CSV + 摘要。

用法:
    python tools/camera_soak.py                          # 默认 8000，24h，30s 间隔
    python tools/camera_soak.py --base-url http://127.0.0.1:8001 --duration 90m --interval 5
    python tools/camera_soak.py --duration 3600 --camera-id 2 --out logs/soak.csv

每轮采样：GET /api/system（http 耗时 / camera status+fps / today_counts）+
GET /api/events?limit=1（最新事件 id）。CSV 列：
    ts,api_ok,http_ms,status,fps,events_today,latest_event_id,note

摘要：采样数 / API 可用率 / camera running 占比 / fps min-avg-max /
事件增量 / API 失败与 fps=0 次数 / camera 非 running 最长持续时间 / CSV 路径。
Ctrl+C 同样落盘并打印摘要。退出码：0=正常结束或中断；2=参数错误；3=起始即连不上。
"""
import argparse
import csv
import datetime
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

CSV_FIELDS = ["ts", "api_ok", "http_ms", "status", "fps",
              "events_today", "latest_event_id", "note"]


def parse_duration(s: str) -> int:
    """24h / 90m / 3600（秒）→ 秒。"""
    m = re.fullmatch(r"(\d+)\s*(h|m|s?)", s.strip(), re.I)
    if not m:
        raise ValueError(f"无法解析时长: {s!r}（支持 24h / 90m / 3600）")
    n, unit = int(m.group(1)), m.group(2).lower()
    return n * 3600 if unit == "h" else n * 60 if unit == "m" else n


def get_json(base, path, timeout):
    t0 = time.perf_counter()
    req = urllib.request.Request(base + path, headers={"Cache-Control": "no-cache"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        data = json_load(r)
    return data, (time.perf_counter() - t0) * 1000


def json_load(r):
    import json
    return json.loads(r.read() or b"{}")


def sample_once(base, camera_id, timeout, note=""):
    """一轮采样；API 失败也返回行（api_ok=0）。"""
    ts = datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    try:
        st, ms = get_json(base, "/api/system", timeout)
    except Exception as e:  # noqa: BLE001
        return {"ts": ts, "api_ok": 0, "http_ms": "", "status": "", "fps": "",
                "events_today": "", "latest_event_id": "", "note": f"api error: {str(e)[:80]}{note}"}
    cams = st.get("cameras") or []
    cam = next((c for c in cams if c.get("id") == camera_id), None)
    if cam is None and cams:
        cam = cams[0]
    try:
        evs, _ = get_json(base, "/api/events?limit=1", timeout)
        latest = evs[0]["id"] if evs else 0
    except Exception as e:  # noqa: BLE001
        latest, err = 0, f"; events error: {str(e)[:60]}"
    else:
        err = ""
    return {"ts": ts, "api_ok": 1, "http_ms": round(ms, 1),
            "status": cam.get("status", "?") if cam else "no-camera",
            "fps": cam.get("fps", "") if cam else "",
            "events_today": sum((st.get("today_counts") or {}).values()),
            "latest_event_id": latest, "note": (note + err)[:120]}


def summarize(rows, duration, camera_id):
    n = len(rows)
    ok = [r for r in rows if r["api_ok"] == 1]
    run = [r for r in ok if r["status"] == "running"]
    fps_vals = [float(r["fps"]) for r in ok if isinstance(r["fps"], (int, float)) or
                (isinstance(r["fps"], str) and r["fps"].replace(".", "", 1).isdigit())]
    ev_first = rows[0]["latest_event_id"] if rows and rows[0]["latest_event_id"] else 0
    ev_last = rows[-1]["latest_event_id"] if rows and rows[-1]["latest_event_id"] else 0
    # camera 非 running 最长持续时间
    worst, cur = 0.0, None
    for r in rows:
        if r["api_ok"] == 1 and r["status"] != "running":
            t = datetime.datetime.strptime(r["ts"], "%Y-%m-%d %H:%M:%S")
            cur = t if cur is None else cur
        else:
            if cur is not None:
                worst = max(worst, (datetime.datetime.strptime(
                    r["ts"], "%Y-%m-%d %H:%M:%S") - cur).total_seconds())
            cur = None
    if cur is not None:
        worst = max(worst, (datetime.datetime.now() - cur).total_seconds())
    print("\n===== soak 摘要 =====")
    print(f"采样数: {n}（间隔见 CSV；计划时长 {duration}s）")
    print(f"API 可用率: {len(ok)}/{n} = {len(ok) / n * 100:.1f}%" if n else "API 可用率: n/a")
    if ok:
        print(f"camera[{camera_id}] running 占比: {len(run)}/{len(ok)} = {len(run) / len(ok) * 100:.1f}%")
    print(f"API 失败次数: {n - len(ok)} | camera 非 running 采样数: {len(ok) - len(run)}")
    print(f"camera 非 running 最长持续: {worst:.0f}s")
    if fps_vals:
        print(f"fps min/avg/max: {min(fps_vals):.1f} / {sum(fps_vals) / len(fps_vals):.1f} / {max(fps_vals):.1f}"
              f"（fps=0 次数 {sum(1 for v in fps_vals if v == 0)}）")
    print(f"事件增量: {max(0, ev_last - ev_first)}（id {ev_first} → {ev_last}）")


def main():
    ap = argparse.ArgumentParser(description="摄像头 24h 稳定性验收采样器（只读 API）")
    ap.add_argument("--base-url", default="http://127.0.0.1:8000")
    ap.add_argument("--camera-id", type=int, default=None,
                    help="目标摄像头 id（缺省取 /api/system 第一路）")
    ap.add_argument("--duration", default="24h", help="采样时长：24h / 90m / 3600（秒）")
    ap.add_argument("--interval", type=int, default=30, help="采样间隔秒（默认 30）")
    ap.add_argument("--out", default=None, help="CSV 路径（默认 logs/soak-<时间戳>.csv）")
    ap.add_argument("--timeout", type=int, default=5, help="单请求超时秒")
    args = ap.parse_args()
    try:
        duration = parse_duration(args.duration)
    except ValueError as e:
        print(e)
        return 2
    base = args.base_url.rstrip("/")
    out = Path(args.out) if args.out else Path("logs") / f"soak-{datetime.datetime.now():%Y%m%d-%H%M%S}.csv"
    out.parent.mkdir(parents=True, exist_ok=True)

    print(f"[soak] {base} camera={args.camera_id or '第一路'} duration={duration}s "
          f"interval={args.interval}s → {out}")
    rows = []

    def flush():
        with open(out, "w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=CSV_FIELDS)
            w.writeheader()
            w.writerows(rows)
        summarize(rows, duration, args.camera_id)
        print(f"CSV 已落盘: {out}")

    try:
        # 起始连通性：连不上直接退出码 3
        try:
            get_json(base, "/api/system", args.timeout)
        except Exception as e:  # noqa: BLE001
            print(f"[soak] 起始连接失败：{e}")
            return 3
        t0 = time.monotonic()
        while time.monotonic() - t0 < duration:
            rows.append(sample_once(base, args.camera_id, args.timeout))
            flush()
            time.sleep(max(1, args.interval - (time.monotonic() - t0) % args.interval))
    except KeyboardInterrupt:
        print("\n[soak] Ctrl+C：已采样数据落盘")
    flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
