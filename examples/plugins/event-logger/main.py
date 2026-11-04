"""event-logger 示例插件（任务126）：轮询 /api/events 把新事件追加写 CSV。

演示插件一期契约：环境注入 LS_API_BASE/LS_PLUGIN_ID/LS_PLUGIN_DATA（读令牌受权限控制）；
退出码任意（宿主只记录，不自动重启）。停 Ctrl-C / 管理页"停止"。
"""
import csv
import os
import time
import urllib.parse
import urllib.request
from pathlib import Path

BASE = os.environ.get("LS_API_BASE", "http://127.0.0.1:8000")
DATA = os.environ.get("LS_PLUGIN_DATA", ".")
TOKEN = os.environ.get("LS_PLUGIN_TOKEN", "")  # 读保护开启时宿主注入（read 权限）
POLL = float(os.environ.get("POLL_SECONDS", "10"))
OUT = Path(DATA) / "events.csv"
SEEN = Path(DATA) / "seen.txt"


def fetch():
    host = urllib.parse.urlparse(BASE).hostname or ""
    if host not in ("127.0.0.1", "::1", "localhost"):  # 示例插件只打本机 API
        raise SystemExit(f"event-logger: LS_API_BASE 仅允许回环地址，收到: {host}")
    import httpx
    with httpx.Client(base_url=BASE.rstrip("/"), timeout=5.0) as client:
        r = client.get("/api/events?limit=50")  # 相对路径，base 已校验回环
        r.raise_for_status()
        return r.json()


def main():
    print(f"event-logger 启动: base={BASE} out={OUT} poll={POLL}s", flush=True)
    try:
        seen = set(SEEN.open(encoding="ascii").read().split())
    except OSError:
        seen = set()
    while True:
        try:
            for ev in fetch():
                eid = str(ev["id"])
                if eid in seen:
                    continue
                seen.add(eid)
                new_file = not OUT.exists()
                with OUT.open("a", newline="", encoding="utf-8-sig") as f:
                    w = csv.writer(f)
                    if new_file:
                        w.writerow(["id", "ts", "camera_id", "type", "level", "detail"])
                    w.writerow([ev["id"], ev["ts"], ev["camera_id"], ev["type"],
                                ev["level"], ev.get("detail", "")])
                print(f"记录事件 #{eid}", flush=True)
            with SEEN.open("w", encoding="ascii") as f:
                f.write("\n".join(sorted(seen)[-500:]))
        except Exception as e:  # noqa: BLE001  服务不可达等：下一轮再试
            print(f"轮询失败（忽略）: {e}", flush=True)
        time.sleep(POLL)


if __name__ == "__main__":
    main()
