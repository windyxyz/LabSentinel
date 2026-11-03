"""定时自动备份（任务92）：按周期备份数据库到 data/backups/，保留最近 N 份 auto zip。

- 线程模式与 retention 一致（start_scheduler/stop_scheduler，main lifespan 调用）；
- 复用 scripts/backup_data.py（subprocess）；失败只记日志不退出服务；
- 只管理 `auto-*.zip`，不删手动备份；启动时 age > interval 即补一次。
"""
import threading
import time
from datetime import datetime
from pathlib import Path

from .config import settings

_CHECK_S = 3600  # 检查周期上限 1h（实际 min(1h, interval/4)）
_thread: threading.Thread | None = None
_stop = threading.Event()
_running = threading.Event()
_lock = threading.Lock()


def backup_dir() -> Path:
    return settings.data_dir / "backups"


def auto_files() -> list[Path]:
    """按 mtime 新→旧排序的 auto-*.zip 列表。"""
    d = backup_dir()
    if not d.exists():
        return []
    return sorted(d.glob("auto-*.zip"), key=lambda p: p.stat().st_mtime, reverse=True)


def _needs_backup(files: list[Path], now: float, interval_h: float) -> bool:
    if not files:
        return True
    latest_mtime = files[0].stat().st_mtime
    return (now - latest_mtime) > interval_h * 3600


def _prune(files: list[Path], keep: int) -> int:
    removed = 0
    for old in files[keep:]:
        old.unlink(missing_ok=True)
        removed += 1
    return removed


def _find_backup_script(here: Path | None = None) -> Path | None:
    """定位 scripts/backup_data.py。

    源码布局 <仓库根>/server/app/backups.py → parents[2]=仓库根；便携布局
    <包根>/app/server/app/backups.py → parents[2]=<包>/app（差一层，2026-10-31 VM 验收实测
    自动备份因此静默失效）→ 追加 parents[3] 兜底。here 参数供测试注入假布局。
    """
    here = here or Path(__file__).resolve()
    for i in (2, 3):
        if i < len(here.parents):
            cand = here.parents[i] / "scripts" / "backup_data.py"
            if cand.exists():
                return cand
    return None


def run_backup() -> tuple[bool, str]:
    """执行一次自动备份（subprocess backup_data.py）；返回 (ok, 消息)。"""
    script = _find_backup_script()
    if script is None:
        return False, "backup_data.py 不存在"
    out = backup_dir() / f"auto-{datetime.now():%Y%m%d-%H%M%S}.zip"
    out.parent.mkdir(parents=True, exist_ok=True)
    import subprocess
    import sys
    try:
        r = subprocess.run(
            [sys.executable, str(script), "backup",
             "--data-dir", str(settings.data_dir), "--out", str(out), "--no-media"],
            capture_output=True, text=True, timeout=300)
        if r.returncode != 0:
            return False, f"backup_data.py exit {r.returncode}: {r.stderr.strip()[:200]}"
        files = auto_files()
        _prune(files, settings.backup_keep)
        return True, f"OK {out.name} ({out.stat().st_size / 1e6:.1f}MB)"
    except Exception as e:  # noqa: BLE001
        return False, f"备份异常: {e}"


def _loop():
    interval_h = settings.backup_interval_h
    check_s = min(_CHECK_S, max(60, int(interval_h * 3600 / 4))) if interval_h > 0 else _CHECK_S
    while not _stop.is_set():
        try:
            files = auto_files()
            if _needs_backup(files, time.time(), interval_h):
                if not _running.is_set():
                    _running.set()
                    try:
                        ok, msg = run_backup()
                        print(f"[auto-backup] {'✓' if ok else '✗'} {msg}")
                    finally:
                        _running.clear()
        except Exception as e:  # noqa: BLE001
            print(f"[auto-backup] 异常: {e}")
        _stop.wait(check_s)


def start_scheduler():
    global _thread
    if settings.backup_interval_h <= 0:
        print("[auto-backup] 已关闭（LS_BACKUP_INTERVAL_H=0）")
        return
    _stop.clear()
    _thread = threading.Thread(target=_loop, daemon=True, name="auto-backup")
    _thread.start()
    print(f"[auto-backup] 已启动（每 {settings.backup_interval_h}h 备份；保留 "
          f"{settings.backup_keep} 份；media={'含' if settings.backup_include_media else '不含'}）")


def stop_scheduler():
    _stop.set()
    t = _thread
    if t and t.is_alive():
        t.join(timeout=2.0)


def status() -> dict:
    files = auto_files()
    now = time.time()
    interval_h = settings.backup_interval_h
    last = None
    if files:
        mtime = files[0].stat().st_mtime
        last = {"name": files[0].name, "size": files[0].stat().st_size,
                "mtime": datetime.fromtimestamp(mtime).isoformat(timespec="seconds"),
                "age_h": round((now - mtime) / 3600, 1)}
    total = sum(f.stat().st_size for f in files)
    return {"enabled": interval_h > 0, "interval_h": interval_h,
            "keep": settings.backup_keep,
            "include_media": settings.backup_include_media,
            "last": last, "count": len(files), "total_bytes": total,
            "running": _running.is_set()}
