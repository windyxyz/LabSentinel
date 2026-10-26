"""多路并发压测脚本（任务 07：多路并发压测）。

对指定端口的 LabSentinel 服务端做 N 路并发压测：
  1. 自检 /api/system 可达（不通过直接退出，绝不误发到其他端口的服务）；
  2. 每路注册：POST /api/cameras（file 源 + autostart=true）→ POST /api/zones（大覆盖多边形）
     → POST /api/rules（enter L2 + exit L1，cooldown 10s）；
  3. 预热 --warmup 秒（DirectML 首次初始化需十几秒）后，每 --interval 秒采样：
     GET /api/system（各路 fps/status + today_counts 事件计数）、服务端进程 CPU%
     （psutil 按监听端口反查 PID）、系统整体 CPU%（对照用户其他负载）；
  4. 结束输出每组汇总 Markdown 表格行，追加写入 --report；原始采样序列写入 --json；
  5. teardown：删除本脚本注册的所有摄像头（zones/rules 随摄像头级联删除），
     再等待 --settle 秒让事件视频写盘线程排空。

用法示例：
  python tools/loadtest.py --counts 1,2,4 --minutes 5 --fps 5.0 --port 8002
  python tools/loadtest.py --help

注意：--fps 只是标注服务端当前 LS_ANALYSIS_FPS 档位（换档必须重启服务端换环境变量，
本脚本不改服务端配置）；--port 务必指向压测专用实例，别指向生产实例。
"""
import argparse
import json
import time
import traceback
from datetime import datetime
from pathlib import Path

import httpx
import psutil

ROOT = Path(__file__).resolve().parent.parent
# 覆盖画面大部分区域的安全区（1280x720 像素坐标），保证演示视频里人员频繁进出触发事件
ZONE_POLYGON = [[150, 150], [1100, 150], [1100, 660], [150, 660]]
DEFAULT_VIDEO = str(ROOT / "m0-spike" / "data" / "lab-walk-24076.mp4")
REPORT_SECTION = "## 压测明细（loadtest.py 自动追加）"
TABLE_HEADER = [
    "| 路数 | 分析FPS档 | 各路平均FPS | 最低FPS | 异常采样数 | 服务端CPU%均值/峰值 | 事件增量 | 事件/分钟 | 判定 |",
    "|---|---|---|---|---|---|---|---|---|",
]


def find_server_pid(port: int) -> int | None:
    """按监听端口反查服务端进程 PID（压测对象是同用户启动的 uvicorn）。"""
    for c in psutil.net_connections(kind="inet"):
        if c.status == psutil.CONN_LISTEN and c.laddr and c.laddr.port == port and c.pid:
            return c.pid
    return None


class LoadTest:
    def __init__(self, args):
        self.args = args
        self.base = (args.base_url or f"http://127.0.0.1:{args.port}").rstrip("/")
        self.client = httpx.Client(base_url=self.base, timeout=httpx.Timeout(15.0, connect=5.0))
        self.pid = find_server_pid(args.port)
        self.proc = psutil.Process(self.pid) if self.pid else None

    # ---- HTTP 基础 ----
    def api(self, method: str, path: str, **kw):
        r = self.client.request(method, path, **kw)
        r.raise_for_status()
        return r.json() if r.content else None

    def event_total(self) -> int:
        """事件总数 = /api/system 的 today_counts 各级别之和（压测用临时库，当天事件即全部）。"""
        return sum(self.api("GET", "/api/system").get("today_counts", {}).values())

    # ---- 注册 / 拆除 ----
    def register(self, count: int) -> list[dict]:
        """注册 count 路：摄像头(file+autostart) + 安全区 + 规则(enter L2 / exit L1, 冷却10s)。"""
        reg = []
        tag = f"lt{self.args.fps:g}x{count}"
        for i in range(count):
            cam = self.api("POST", "/api/cameras", json={
                "name": f"{tag}-c{i + 1}", "source": self.args.video, "type": "file",
                "autostart": True})
            zid = self.api("POST", "/api/zones", json={
                "camera_id": cam["id"], "name": f"{tag}-zone{i + 1}", "polygon": ZONE_POLYGON})["id"]
            for behavior, level in (("enter", 2), ("exit", 1)):
                self.api("POST", "/api/rules", json={
                    "camera_id": cam["id"], "zone_id": zid, "behavior": behavior,
                    "level": level, "cooldown_s": 10})
            reg.append({"camera_id": cam["id"], "zone_id": zid})
            print(f"  已注册第 {i + 1}/{count} 路：cam#{cam['id']} zone#{zid}（worker 已启动）")
        return reg

    def teardown(self, reg: list[dict]):
        for item in reg:
            try:
                self.api("DELETE", f"/api/cameras/{item['camera_id']}")
            except Exception as e:  # noqa: BLE001
                print(f"  [teardown] 删除 cam#{item['camera_id']} 失败: {e}")
        remaining = self.api("GET", "/api/cameras")
        print(f"  teardown 完成：已删除 {len(reg)} 路，服务端剩余摄像头 {len(remaining)} 个")
        if self.args.settle > 0:
            print(f"  等待 {self.args.settle}s 让事件视频写盘线程排空 ...")
            time.sleep(self.args.settle)

    # ---- 采样 ----
    def take_sample(self) -> dict:
        sysinfo = self.api("GET", "/api/system")
        cpu = self.proc.cpu_percent(None) if self.proc else None  # 距上次调用的均值
        return {
            "t": round(time.monotonic(), 1),
            "wall": datetime.now().strftime("%H:%M:%S"),
            "cams": [{"id": c["id"], "fps": c.get("fps", 0.0), "status": c.get("status")}
                     for c in sysinfo.get("cameras", [])],
            "proc_cpu": cpu,
            "sys_cpu": psutil.cpu_percent(None),
            "proc_rss_mb": round(self.proc.memory_info().rss / 1048576, 1) if self.proc else None,
            "events": sum(sysinfo.get("today_counts", {}).values()),
        }

    def run_group(self, count: int) -> dict:
        print(f"\n===== 组：{count} 路 @ 分析FPS={self.args.fps:g}，"
              f"预热 {self.args.warmup:g}s + 采样 {self.args.minutes:g} 分钟 =====")
        reg = self.register(count)
        if self.proc:
            self.proc.cpu_percent(None)  # 建立进程 CPU 基线（预热结束后再正式采样）
        psutil.cpu_percent(None)
        events0 = self.event_total()
        print(f"预热 {self.args.warmup:g}s（DirectML 首次初始化需十几秒）...")
        time.sleep(self.args.warmup)

        samples, t0, deadline, next_t = [], time.monotonic(), None, None
        deadline = t0 + self.args.minutes * 60
        next_t = t0
        while True:
            now = time.monotonic()
            if now < next_t:
                time.sleep(min(next_t - now, 1.0))
                continue
            try:
                s = self.take_sample()
            except Exception:  # noqa: BLE001  单次采样失败不中断整组
                traceback.print_exc()
                next_t += self.args.interval
                continue
            samples.append(s)
            fps_txt = " ".join(f"{c['fps']:.0f}" for c in s["cams"])
            print(f"  [{s['wall']}] fps: {fps_txt} | 进程CPU {s['proc_cpu']:.0f}% "
                  f"| 系统CPU {s['sys_cpu']:.0f}% | 事件 {s['events']} (基准 {events0})")
            next_t += self.args.interval
            if now >= deadline:
                break
        time.sleep(0.5)  # 让最后一次差分窗口落地
        summary = self.summarize(count, samples)
        self.teardown(reg)
        return {"ts": datetime.now().isoformat(timespec="seconds"),
                "base_url": self.base, "group": count, "fps_tier": self.args.fps,
                "minutes": self.args.minutes, "registered": reg,
                "summary": summary, "samples": samples}

    # ---- 汇总 ----
    def summarize(self, count: int, samples: list[dict]) -> dict:
        """汇总一组采样：各路 FPS 均值/最低、异常采样、CPU、事件速率、达标判定。"""
        steady = samples[1:] if len(samples) > 1 else samples  # 第 1 个采样覆盖预热窗口，不算稳态
        cam_ids = sorted({c["id"] for s in samples for c in s["cams"]})
        per_cam = {}
        for cid in cam_ids:
            vals = [c["fps"] for s in samples for c in s["cams"] if c["id"] == cid]
            bad = sum(1 for s in samples
                      if not any(c["id"] == cid and c["status"] == "running" for c in s["cams"]))
            per_cam[cid] = {"fps_mean": round(sum(vals) / len(vals), 2) if vals else 0.0,
                            "fps_min": round(min(vals), 2) if vals else 0.0,
                            "bad_samples": bad}
        cpus = [s["proc_cpu"] for s in steady if s["proc_cpu"] is not None]
        sys_cpus = [s["sys_cpu"] for s in steady if s["sys_cpu"] is not None]
        el = samples[-1]["t"] - samples[0]["t"]
        ev_delta = samples[-1]["events"] - samples[0]["events"]
        rate = ev_delta / el * 60 if el > 0 else 0.0
        bad_total = sum(v["bad_samples"] for v in per_cam.values())
        ok = bad_total == 0 and all(v["fps_mean"] >= 2 for v in per_cam.values())
        return {
            "count": count,
            "fps_means": [round(per_cam[cid]["fps_mean"], 2) for cid in cam_ids],
            "fps_min": round(min(v["fps_min"] for v in per_cam.values()), 2) if per_cam else 0.0,
            "bad_samples": bad_total,
            "cpu_mean": round(sum(cpus) / len(cpus), 1) if cpus else None,
            "cpu_max": round(max(cpus), 1) if cpus else None,
            "sys_cpu_mean": round(sum(sys_cpus) / len(sys_cpus), 1) if sys_cpus else None,
            "rss_mb": samples[-1]["proc_rss_mb"],
            "events_delta": ev_delta,
            "events_per_min": round(rate, 1),
            "elapsed_s": round(el, 1),
            "per_cam": per_cam,
            "ok": ok,
        }

    # ---- 产物落盘 ----
    def append_report(self, summary: dict):
        """把一组汇总追加为报告文件里的 Markdown 表格行（表头不存在时先建节）。"""
        path = Path(self.args.report)
        fps_txt = " / ".join(f"{v:.1f}" for v in summary["fps_means"]) or "-"
        verdict = "达标" if summary["ok"] else "不达标"
        cpu_txt = (f"{summary['cpu_mean']:.0f} / {summary['cpu_max']:.0f}"
                   if summary["cpu_mean"] is not None else "-")
        row = (f"| {summary['count']} | {self.args.fps:g} | {fps_txt} | {summary['fps_min']:.1f} "
               f"| {summary['bad_samples']} | {cpu_txt} | {summary['events_delta']} "
               f"| {summary['events_per_min']:.1f} | {verdict} |")
        text = path.read_text(encoding="utf-8") if path.exists() else ""
        if REPORT_SECTION not in text:
            text = text.rstrip("\n") + "\n\n" + REPORT_SECTION + "\n\n" + "\n".join(TABLE_HEADER) + "\n"
        elif "| 路数 |" not in text:
            text = text.rstrip("\n") + "\n\n" + "\n".join(TABLE_HEADER) + "\n"
        path.write_text(text + row + "\n", encoding="utf-8")
        print(f"  报告行已追加：{path}")
        print(f"  {row}")

    def append_json(self, record: dict):
        path = Path(self.args.json)
        path.parent.mkdir(parents=True, exist_ok=True)
        data = json.loads(path.read_text(encoding="utf-8")) if path.exists() else []
        data.append(record)
        path.write_text(json.dumps(data, ensure_ascii=False, indent=1), encoding="utf-8")
        print(f"  原始采样已写入：{path}")

    # ---- 主流程 ----
    def run(self):
        sysinfo = self.api("GET", "/api/system")  # 自检：服务端可达
        existing = self.api("GET", "/api/cameras")
        print(f"自检通过：服务端 {self.base} v{sysinfo.get('version')} "
              f"backend={sysinfo.get('backend')} PID={self.pid}")
        if existing:
            print(f"[警告] 服务端已有 {len(existing)} 个摄像头（本次压测只增删自己注册的路数）")
        if self.proc is None:
            print("[警告] 未能按端口反查服务端 PID，进程 CPU 将记为 null")
        results = []
        try:
            for count in self.args.counts:
                rec = self.run_group(count)
                self.append_report(rec["summary"])
                self.append_json(rec)
                results.append(rec["summary"])
        finally:
            self.client.close()
        print("\n===== 汇总 =====")
        for s in results:
            print(f"{s['count']} 路 @ {self.args.fps:g}fps：均FPS {s['fps_means']} "
                  f"CPU {s['cpu_mean']}% 事件/分 {s['events_per_min']} → "
                  f"{'达标' if s['ok'] else '不达标'}")
        return results


def main():
    ap = argparse.ArgumentParser(
        description="LabSentinel 多路并发压测（任务07）：注册 N 路文件摄像头，周期采样 FPS/CPU/事件速率",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter)
    ap.add_argument("--counts", default="1,2,4,8", help="每组并发路数，逗号分隔，逐组顺序执行")
    ap.add_argument("--minutes", type=float, default=5.0, help="每组采样时长（分钟）")
    ap.add_argument("--fps", type=float, default=5.0,
                    help="服务端 LS_ANALYSIS_FPS 档位标注（换档需重启服务端，脚本不改配置）")
    ap.add_argument("--port", type=int, default=8002, help="压测目标服务端端口（勿指向生产实例）")
    ap.add_argument("--base-url", default=None, help="覆盖默认 http://127.0.0.1:<port>")
    ap.add_argument("--warmup", type=float, default=30.0, help="正式采样前预热秒数（DirectML 首初始化约 10-20s）")
    ap.add_argument("--interval", type=float, default=30.0, help="采样间隔（秒）")
    ap.add_argument("--settle", type=float, default=15.0, help="teardown 后等待视频写盘排空的秒数")
    ap.add_argument("--video", default=DEFAULT_VIDEO, help="压测用演示视频（多路共用同一路径）")
    ap.add_argument("--report", default=str(ROOT / "docs" / "51-loadtest-report.md"),
                    help="Markdown 表格行追加写入的报告文件")
    ap.add_argument("--json", default="D:/tmp/loadtest/loadtest-raw.json",
                    help="原始采样 JSON 落盘路径（临时产物）")
    args = ap.parse_args()
    args.counts = [int(x) for x in str(args.counts).split(",") if x.strip()]
    LoadTest(args).run()


if __name__ == "__main__":
    main()
