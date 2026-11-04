"""摄像头 24h 稳定性验收采样器（任务42/78）：只读 API，产出 CSV + JSON 摘要，支持多路与接入前预检。

用法:
    python tools/camera_soak.py                          # 默认 8000，24h，30s 间隔，全部摄像头
    python tools/camera_soak.py --base-url http://127.0.0.1:8001 --duration 90m --interval 5
    python tools/camera_soak.py --camera-id 2 --duration 3600 --out logs/soak.csv
    python tools/camera_soak.py --preflight              # 接入前预检（不采样），FAIL 退出码 2

每采样点每路一行，CSV 列（任务78）：
    ts, camera_id, status, fps, frame_age_s, reconnect_count, last_error,
    events_total, api_latency_ms, cpu_percent, rss_mb, disk_free_gb

汇总 JSON（--summary-json）：时长/采样数/每路 running 占比/平均 fps/最大帧龄/重连/错误种类/
事件增长/API p50/p95/max/CPU 峰值/RSS 峰值/磁盘下降/判定 PASS|WARN|FAIL。
Ctrl+C 落盘。退出码：0=正常；2=参数/预检 FAIL；3=起始连不上。
"""
import argparse
import csv
import ctypes
import datetime
import json
import os
import re
import shutil
import sqlite3
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

CSV_FIELDS = ["ts", "camera_id", "status", "fps", "frame_age_s", "reconnect_count",
              "last_error", "events_total", "api_latency_ms", "cpu_percent", "rss_mb",
              "disk_free_gb"]


# ---------- 纯函数（便于单测） ----------

def parse_duration(s: str) -> int:
    """24h / 90m / 3600（秒）→ 秒。"""
    m = re.fullmatch(r"(\d+)\s*(h|m|s?)", s.strip(), re.I)
    if not m:
        raise ValueError(f"无法解析时长: {s!r}（支持 24h / 90m / 3600）")
    n, unit = int(m.group(1)), m.group(2).lower()
    return n * 3600 if unit == "h" else n * 60 if unit == "m" else n


def parse_ffprobe_json(text: str) -> dict | None:
    """ffprobe -of json 输出 → {codec,width,height,fps}；失败/非 JSON 返回 None。"""
    try:
        d = json.loads(text or "")
    except ValueError:
        return None
    streams = d.get("streams") or []
    if not streams:
        return None
    st = streams[0]
    fps = st.get("avg_frame_rate") or st.get("r_frame_rate") or ""
    try:
        num, den = fps.split("/")
        fps_v = round(float(num) / float(den), 1) if float(den) else 0.0
    except (ValueError, ZeroDivisionError):
        fps_v = 0.0
    return {"codec": st.get("codec_name", "?"), "width": st.get("width", 0),
            "height": st.get("height", 0), "fps": fps_v}


def build_row(ts, cam, api_ms, events_total, cpu_percent, rss_mb, disk_free_gb):
    """一路摄像头 → 一行 CSV dict（任务78 列）。cam 为 /api/system 的 cameras[] 元素。"""
    return {
        "ts": ts, "camera_id": cam.get("id", ""), "status": cam.get("status", "?"),
        "fps": cam.get("fps", ""), "frame_age_s": cam.get("frame_age_s", ""),
        "reconnect_count": cam.get("reconnect_count", ""),
        "last_error": (cam.get("last_error") or "")[:80],
        "events_total": events_total, "api_latency_ms": round(api_ms, 1),
        "cpu_percent": cpu_percent, "rss_mb": rss_mb, "disk_free_gb": disk_free_gb,
    }


def _pct(vals):
    if not vals:
        return None
    v = sorted(vals)
    n = len(v)
    return {"p50": round(v[n // 2], 1),
            "p95": round(v[min(n - 1, int(n * 0.95))], 1),
            "max": round(v[-1], 1)}


def summarize(rows, duration_s, stall_s=60,
              min_api_rate=99.0, min_running_rate=99.0, max_reconnect=10) -> dict:
    """采样行 → 汇总 dict（含 PASS/WARN/FAIL 判定；阈值参数化便于单测）。"""
    n = len(rows)
    ok = [r for r in rows if r["api_latency_ms"] != ""]
    api_lat = [float(r["api_latency_ms"]) for r in ok if r["api_latency_ms"] != ""]
    cams = {}
    for r in ok:
        cams.setdefault(str(r["camera_id"]), []).append(r)
    per_cam = {}
    for cid, rs in sorted(cams.items()):
        run = [r for r in rs if r["status"] == "running"]
        fps = [float(r["fps"]) for r in rs if r["fps"] not in ("", None)]
        ages = [float(r["frame_age_s"]) for r in rs if r["frame_age_s"] not in ("", None)]
        recon = sum(int(r["reconnect_count"] or 0) for r in rs)
        errs = sorted({r["last_error"] for r in rs if r["last_error"]})
        per_cam[cid] = {
            "samples": len(rs),
            "running_rate_pct": round(len(run) / len(rs) * 100, 1),
            "fps_avg": round(sum(fps) / len(fps), 1) if fps else None,
            "fps_max": max(fps) if fps else None,
            "max_frame_age_s": max(ages) if ages else None,
            "stall_samples": sum(1 for a in ages if a > stall_s),
            "reconnect_total": recon,
            "errors": errs,
        }
    ev_first = min((int(r["events_total"]) for r in rows
                    if r["events_total"] not in ("", None)), default=0)
    ev_last = max((int(r["events_total"]) for r in rows
                   if r["events_total"] not in ("", None)), default=0)
    cpu = [float(r["cpu_percent"]) for r in rows if r["cpu_percent"] not in ("", None)]
    rss = [float(r["rss_mb"]) for r in rows if r["rss_mb"] not in ("", None)]
    disk = [float(r["disk_free_gb"]) for r in rows if r["disk_free_gb"] not in ("", None)]
    api_rate = round(len(ok) / n * 100, 1) if n else 0.0
    cam_rates = [c["running_rate_pct"] for c in per_cam.values()]
    max_recon = max((c["reconnect_total"] for c in per_cam.values()), default=0)
    max_age = max((c["max_frame_age_s"] or 0) for c in per_cam.values())
    if not n or api_rate < min_api_rate or (cam_rates and min(cam_rates) < min_running_rate):
        verdict = "FAIL"
    elif max_recon > max_reconnect or (max_age and max_age > stall_s * 3) or \
            any(c["errors"] for c in per_cam.values()):
        verdict = "WARN"
    else:
        verdict = "PASS"
    return {
        "duration_s": duration_s, "samples": n, "api_rate_pct": api_rate,
        "api_latency_ms": _pct(api_lat), "cameras": per_cam,
        "events_growth": max(0, ev_last - ev_first),
        "cpu_percent_max": max(cpu) if cpu else None,
        "rss_mb_max": max(rss) if rss else None,
        "disk_free_gb": {"first": disk[0] if disk else None,
                         "last": disk[-1] if disk else None,
                         "drop_gb": round(disk[0] - disk[-1], 2) if len(disk) >= 2 else None},
        "verdict": verdict,
        "thresholds": {"min_api_rate": min_api_rate, "min_running_rate": min_running_rate,
                       "max_reconnect": max_reconnect, "stall_s": stall_s},
    }


def render_summary(s: dict) -> str:
    lines = ["\n===== soak 摘要 =====",
             f"时长 {s['duration_s']}s | 采样 {s['samples']} | API 可用率 {s['api_rate_pct']}%",
             f"API 延迟 ms: {s['api_latency_ms']}",
             f"事件增长: {s['events_growth']} | CPU 峰值: {s['cpu_percent_max']}% | "
             f"RSS 峰值: {s['rss_mb_max']}MB | 磁盘下降: {s['disk_free_gb']['drop_gb']}GB"]
    for cid, c in s["cameras"].items():
        lines.append(f"camera[{cid}]: running {c['running_rate_pct']}% | fps_avg {c['fps_avg']} | "
                     f"最大帧龄 {c['max_frame_age_s']}s | 停滞采样 {c['stall_samples']} | "
                     f"重连 {c['reconnect_total']} | 错误 {c['errors'] or '无'}")
    lines.append(f"判定: {s['verdict']}")
    return "\n".join(lines)


def parse_ffprobe_stream(ffprobe: Path, url: str, timeout: int = 20) -> dict | None:
    """对单个 RTSP 地址跑 ffprobe，返回解析结果或 None（失败）。"""
    import subprocess
    try:
        r = subprocess.run(
            [str(ffprobe), "-v", "error", "-rtsp_transport", "tcp", "-select_streams", "v:0",
             "-show_entries", "stream=codec_name,width,height,avg_frame_rate",
             "-of", "json", url],
            capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return parse_ffprobe_json(r.stdout)


# ---------- 采样与进程指标 ----------

def get_json(base, path, timeout):
    host = urllib.parse.urlparse(base).hostname or ""
    if host not in ("127.0.0.1", "::1", "localhost"):  # 压测工具只打本机实例
        raise SystemExit(f"[soak] 仅允许回环地址作为 base-url，收到: {host}")
    import httpx
    t0 = time.perf_counter()
    with httpx.Client(base_url=base.rstrip("/"), timeout=timeout) as client:
        r = client.get(path)  # 相对路径，base 已校验回环
    data = r.json() if r.status_code == 200 else {}
    return data, (time.perf_counter() - t0) * 1000


class SystemMetrics:
    """系统级 CPU%（GetSystemTimes，标准库 ctypes，无需 psutil）与磁盘余量。
    CPU 为整机口径（含其它进程），如实标注；用于 24h 观察趋势足够。"""

    def __init__(self):
        self._prev = self._times()

    @staticmethod
    def _times():
        class FT(ctypes.Structure):
            _fields_ = [("idle", ctypes.c_ulonglong), ("kernel", ctypes.c_ulonglong),
                        ("user", ctypes.c_ulonglong)]
        st = FT()
        if not ctypes.windll.kernel32.GetSystemTimes(ctypes.byref(st), ctypes.byref(st),
                                                     ctypes.byref(st)):
            return None
        return (st.idle, st.kernel + st.user)

    def cpu_percent(self) -> str:
        cur = self._times()
        if cur is None or self._prev is None:
            return ""
        d_idle = cur[0] - self._prev[0]
        d_total = cur[1] - self._prev[1]
        self._prev = cur
        if d_total <= 0:
            return ""
        return round((1 - d_idle / d_total) * 100, 1)


def disk_free_gb() -> float:
    try:
        return round(shutil.disk_usage(os.getcwd()).free / 1e9, 1)
    except OSError:
        return ""


def sample_all(base, timeout, metrics: SystemMetrics, stall_s: float):
    """一轮采样 → 每路一行（无摄像头时单行 camera_id=''）。"""
    ts = datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    st, ms = get_json(base, "/api/system", timeout)
    evs, _ = get_json(base, "/api/events/count", timeout)
    total = evs.get("count", 0) if isinstance(evs, dict) else 0
    cpu = metrics.cpu_percent()
    disk = disk_free_gb()
    cams = st.get("cameras") or []
    if not cams:
        return [build_row(ts, {"id": "", "status": "no-camera"}, ms, total, cpu, "", disk)]
    return [build_row(ts, cam, ms, total, cpu, "", disk) for cam in cams]


# ---------- preflight（接入前预检） ----------

def detect_tool(root: Path, name: str) -> Path | None:
    for c in (root / "tools" / "ffmpeg" / name, root / "tools" / name, root / name):
        if c.exists():
            return c
    return None


def preflight(base, timeout, root: Path) -> tuple[list[str], bool]:
    """接入前检查 → (输出行, 是否有 FAIL)。"""
    lines, has_fail = [], False

    def row(name, status, detail):
        nonlocal has_fail
        if status == "FAIL":
            has_fail = True
        lines.append(f"[{status:<4}] {name}  {detail}")

    # 1) 服务可达 + 摄像头列表
    try:
        st, _ = get_json(base, "/api/system", timeout)
        cams = st.get("cameras") or []
        row("服务可达", "PASS", f"{base} backend={st.get('backend')} version={st.get('version')}")
    except Exception as e:  # noqa: BLE001
        row("服务可达", "FAIL", f"{base} 连不上: {e}——先启动服务再预检")
        return lines, True
    db_cams, _ = get_json(base, "/api/cameras", timeout)
    # 2) RTSP 源 ffprobe（H.265 → WARN；失败 → FAIL）
    ffprobe = detect_tool(root, "ffprobe.exe") or detect_tool(root, "ffprobe")
    if ffprobe is None:
        row("ffprobe", "FAIL", "未找到 ffprobe（tools/ffmpeg/ffprobe.exe）——无法预检编码")
        has_fail = True
    rtsp = [c for c in db_cams if c.get("type") == "rtsp"]
    if not rtsp:
        row("RTSP 源", "WARN", "当前没有 type=rtsp 摄像头——预检仅覆盖服务与本地环境")
    for c in rtsp:
        if ffprobe is None:
            break
        info = parse_ffprobe_stream(ffprobe, c["source"]) if c.get("source") else None
        if info is None:
            row(f"RTSP #{c['id']} {c.get('name')}", "FAIL",
                f"ffprobe 失败（地址/网络/账号）：{c.get('source', '')[:40]}...")
        elif info["codec"] in ("hevc", "h265"):
            row(f"RTSP #{c['id']} {c.get('name')}", "WARN",
                f"H.265 需 go2rtc 转码（{info['width']}x{info['height']}@{info['fps']}）——建议切子码流 H.264")
        else:
            row(f"RTSP #{c['id']} {c.get('name')}", "PASS",
                f"{info['codec']} {info['width']}x{info['height']}@{info['fps']}")
        src = (c.get("source") or "").lower()
        if "admin:admin@" in src or ":12345@" in src:
            row(f"RTSP #{c['id']} 凭证", "WARN", "疑似默认密码（admin:admin / 12345）——请改强密码")
    # 3) 模型/词表
    for p in (root / "models" / "yolo11n_ppe.onnx", root / "m0-spike" / "yolo11n_ppe.onnx"):
        if p.exists() and p.stat().st_size > 1_000_000:
            row("模型", "PASS", f"{p.name} {p.stat().st_size / 1e6:.1f}MB")
            break
    else:
        row("模型", "FAIL", "未找到 yolo11n_ppe.onnx（>1MB）")
        has_fail = True
    # 4) DB integrity
    db = None
    for cand in (root / "server" / "data" / "labsentinel.db",
                 Path(os.environ.get("LOCALAPPDATA", "")) / "LabSentinel" / "data" / "labsentinel.db"):
        if cand.exists():
            db = cand
            break
    if db is None:
        row("数据库", "WARN", "未找到（首次启动自动创建）")
    else:
        try:
            con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
            ok = con.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
            con.close()
            row("数据库", "PASS" if ok else "FAIL", f"integrity={ok and 'ok' or 'damaged'} {db.name}")
        except sqlite3.Error as e:
            row("数据库", "FAIL", f"打不开: {e}")
            has_fail = True
    # 5) 磁盘（24h 粗估 ≥20GB）
    free = shutil.disk_usage(str(root)).free / 1e9
    row("磁盘", "PASS" if free >= 20 else "WARN", f"free {free:.1f}GB（24h 验收建议 ≥20GB）")
    return lines, has_fail


# ---------- main ----------

def main():
    ap = argparse.ArgumentParser(description="摄像头 24h 稳定性验收采样器（只读 API，任务42/78）")
    ap.add_argument("--base-url", default="http://127.0.0.1:8000")
    ap.add_argument("--camera-id", type=int, action="append", dest="camera_ids",
                    help="目标摄像头 id（可重复；缺省=全部摄像头）")
    ap.add_argument("--all", action="store_true", help="采样全部摄像头（默认行为，保留参数兼容）")
    ap.add_argument("--duration", default="24h", help="采样时长：24h / 90m / 3600（秒）")
    ap.add_argument("--interval", type=int, default=30, help="采样间隔秒（默认 30）")
    ap.add_argument("--out", default=None, help="CSV 路径（默认 logs/soak-<时间戳>.csv）")
    ap.add_argument("--summary-json", default=None, help="汇总 JSON 路径（缺省与 CSV 同名 .json）")
    ap.add_argument("--stall-s", type=float, default=60, help="超过该秒数无新帧判画面停滞（默认 60）")
    ap.add_argument("--timeout", type=int, default=5, help="单请求超时秒")
    ap.add_argument("--preflight", action="store_true", help="只做接入前预检后退出（FAIL 退出码 2）")
    args = ap.parse_args()
    try:
        duration = parse_duration(args.duration)
    except ValueError as e:
        print(e)
        return 2
    base = args.base_url.rstrip("/")
    root = Path(__file__).resolve().parent.parent

    if args.preflight:
        lines, has_fail = preflight(base, args.timeout, root)
        print("\n".join(lines))
        print(f"\n===== preflight {'FAIL' if has_fail else 'PASS'} =====")
        return 2 if has_fail else 0

    out = Path(args.out) if args.out else \
        Path("logs") / f"soak-{datetime.datetime.now():%Y%m%d-%H%M%S}.csv"
    summary_path = Path(args.summary_json) if args.summary_json else out.with_suffix(".json")
    out.parent.mkdir(parents=True, exist_ok=True)
    metrics = SystemMetrics()
    print(f"[soak] {base} cameras={args.camera_ids or '全部'} duration={duration}s "
          f"interval={args.interval}s → {out}")
    rows = []

    def flush():
        with out.open("w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=CSV_FIELDS)
            w.writeheader()
            w.writerows(rows)
        s = summarize(rows, duration, stall_s=args.stall_s)
        summary_path.write_text(json.dumps(s, ensure_ascii=False, indent=2), encoding="utf-8")
        print(render_summary(s))
        print(f"CSV: {out}\nJSON: {summary_path}")

    try:
        try:
            get_json(base, "/api/system", args.timeout)
        except Exception as e:  # noqa: BLE001
            print(f"[soak] 起始连接失败：{e}")
            return 3
        t0 = time.monotonic()
        while time.monotonic() - t0 < duration:
            new_rows = sample_all(base, args.timeout, metrics, args.stall_s)
            if args.camera_ids:
                new_rows = [r for r in new_rows if r["camera_id"] in args.camera_ids]
            rows.extend(new_rows)
            flush()
            time.sleep(max(1, args.interval - (time.monotonic() - t0) % args.interval))
    except KeyboardInterrupt:
        print("\n[soak] Ctrl+C：已采样数据落盘")
    flush()
    s = summarize(rows, duration, stall_s=args.stall_s)
    return 0 if s["verdict"] != "FAIL" else 2


if __name__ == "__main__":
    sys.exit(main())
