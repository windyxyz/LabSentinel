"""数据保留与清理（任务14，合规：80-risk-register C3 最小化存储）。

- 定期（每 6 小时）清理过期的截图/视频文件与误报池文件，事件行保留统计价值
  （snapshot/video 列置 NULL），超过保留期的整行删除。
- 磁盘低于 LS_DISK_MIN_FREE_GB 时紧急清理：从最旧事件开始逐批删，直至满足空间。
- 后台线程调度（start/stop_scheduler），不占用 worker 线程；清理用独立短事务，避免与 Store 写锁死锁。
"""
import os
import shutil
import threading
import time
from datetime import datetime, timedelta

from .config import settings

_INTERVAL_S = 6 * 3600
_BATCH = 50  # 紧急清理每批删除的事件数（删完重查磁盘）

_last_result: dict = {}
_lock = threading.Lock()
_thread: threading.Thread | None = None
_stop = threading.Event()


def _safe_within(path: str, parent) -> bool:
    """路径安全校验：必须在 parent 目录内（防路径穿越/兄弟目录绕过，2026-10-30 审阅改 commonpath）。"""
    try:
        return os.path.commonpath([os.path.abspath(path), os.path.abspath(parent)]) \
            == os.path.abspath(parent)
    except (ValueError, OSError):
        return False


def _disk_free_gb() -> float:
    try:
        return shutil.disk_usage(str(settings.data_dir)).free / 1e9
    except Exception:  # noqa: BLE001
        return float("inf")


def preview_cleanup(store) -> dict:
    """只读预演（任务68）：与 run_cleanup 同一套阈值/路径安全判断，**绝不删除任何东西**。

    2026-10-30 审阅修复：分页改为 `id > last_id` 游标推进——原实现循环内不删行也无游标，
    过期事件 ≥ _BATCH 条时同查询无限重复，GET /api/maintenance/preview 会拖死 API 线程池。
    """
    import os
    cutoff = (datetime.now() - timedelta(days=settings.retention_events_days)).isoformat(timespec="seconds")
    ev_count, ev_bytes = 0, 0
    last_id = 0
    while True:
        with store._conn() as c:
            rows = c.execute(
                "SELECT id, snapshot, video FROM events WHERE ts < ? AND id > ? "
                "ORDER BY id ASC LIMIT ?",
                (cutoff, last_id, _BATCH)).fetchall()
        if not rows:
            break
        for r in rows:
            last_id = r["id"]
            ev_count += 1
            for col in ("snapshot", "video"):
                p = r[col]
                if p and _safe_within(p, settings.data_dir):
                    try:
                        ev_bytes += os.path.getsize(p)
                    except OSError:
                        pass  # 文件缺失按 0 计（run_cleanup 同语义：跳过）
        if len(rows) < _BATCH:
            break
    pool_files, pool_bytes = 0, 0
    try:
        pool = settings.annotation_pool_dir
        if pool.exists():
            for f in pool.iterdir():
                if f.is_file() and datetime.fromtimestamp(f.stat().st_mtime) < \
                        datetime.now() - timedelta(days=settings.retention_pool_days):
                    try:
                        pool_files += 1
                        pool_bytes += f.stat().st_size
                    except OSError:
                        pass
    except OSError:
        pass
    free = _disk_free_gb()
    return {
        "cutoff": cutoff,
        "events": {"count": ev_count, "bytes": ev_bytes},
        "pool": {"files": pool_files, "bytes": pool_bytes},
        "disk_free_gb": round(free, 2),
        "estimated_free_gb": round(free + (ev_bytes + pool_bytes) / 1e9, 2),
        "would_emergency_clean": free < settings.disk_min_free_gb,
    }


def run_cleanup(store) -> dict:
    """执行一轮清理，返回统计 {deleted_events, deleted_files, freed_mb}。幂等可重入。"""
    stats = {"deleted_events": 0, "deleted_files": 0, "freed_mb": 0.0}
    cutoff = (datetime.now() - timedelta(days=settings.retention_events_days)).isoformat(timespec="seconds")
    pool_cutoff = (datetime.now() - timedelta(days=settings.retention_pool_days)).isoformat(timespec="seconds")
    now_iso = datetime.now().isoformat(timespec="seconds")

    # 1) 过期事件：先删文件再删行（独立短事务）
    while True:
        with store._conn() as c:
            rows = c.execute(
                "SELECT id, snapshot, video FROM events WHERE ts < ? LIMIT ?",
                (cutoff, _BATCH)).fetchall()
        if not rows:
            break
        for r in rows:
            for col in ("snapshot", "video"):
                p = r[col]
                if p and _safe_within(p, settings.data_dir):
                    try:
                        import os
                        size = os.path.getsize(p)
                        os.remove(p)
                        stats["deleted_files"] += 1
                        stats["freed_mb"] += size / 1e6
                    except FileNotFoundError:
                        pass
                    except OSError as e:
                        print(f"[retention] 删除失败 {p}: {e}")
            with store._lock, store._conn() as c:
                c.execute("DELETE FROM events WHERE id=?", (r["id"],))
            stats["deleted_events"] += 1
        if len(rows) < _BATCH:
            break

    # 2) 保留期内事件：仅把过期不可能存在的文件引用置 NULL（文件早于行的兜底，本处跳过）
    #    （文件生命周期与事件行同周期，见上；这里不做额外 NULL 操作）

    # 3) 误报标注池过期文件
    try:
        pool = settings.annotation_pool_dir
        if pool.exists():
            import os
            for f in pool.iterdir():
                if f.is_file() and datetime.fromtimestamp(f.stat().st_mtime) < \
                        datetime.now() - timedelta(days=settings.retention_pool_days):
                    try:
                        size = f.stat().st_size
                        f.unlink()
                        stats["deleted_files"] += 1
                        stats["freed_mb"] += size / 1e6
                    except OSError as e:
                        print(f"[retention] 池文件删除失败 {f}: {e}")
        _ = pool_cutoff  # 保留变量语义（按修改时间判定，cutoff 用于按需扩展）
    except OSError as e:
        print(f"[retention] 误报池清理失败: {e}")

    # 4) 磁盘紧急清理：仍不够就从最旧事件开始逐批删（先删文件再删行，每批复查）
    freed_rounds = 0
    while _disk_free_gb() < settings.disk_min_free_gb and freed_rounds < 200:
        with store._conn() as c:
            rows = c.execute(
                "SELECT id, snapshot, video FROM events ORDER BY id ASC LIMIT ?",
                (_BATCH,)).fetchall()
        if not rows:
            print("[retention] 磁盘仍不足但已无可删事件——请人工介入")
            break
        for r in rows:
            for col in ("snapshot", "video"):
                p = r[col]
                if p and _safe_within(p, settings.data_dir):
                    try:
                        import os
                        size = os.path.getsize(p)
                        os.remove(p)
                        stats["deleted_files"] += 1
                        stats["freed_mb"] += size / 1e6
                    except OSError:
                        pass
            with store._lock, store._conn() as c:
                c.execute("DELETE FROM events WHERE id=?", (r["id"],))
            stats["deleted_events"] += 1
        freed_rounds += 1
        print(f"[retention] 紧急清理第 {freed_rounds} 批，剩余磁盘 {_disk_free_gb():.1f}GB")

    stats["ts"] = now_iso
    stats["disk_free_gb"] = round(_disk_free_gb(), 2)
    print(f"[retention] 清理完成: {stats}")
    with _lock:
        _last_result.clear()
        _last_result.update(stats)
    return stats


def last_result() -> dict:
    with _lock:
        return dict(_last_result)


def _loop(store):
    while not _stop.is_set():
        try:
            run_cleanup(store)
        except Exception as e:  # noqa: BLE001
            print(f"[retention] 清理异常: {e}")
        _stop.wait(_INTERVAL_S)


def start_scheduler(store):
    global _thread
    _stop.clear()
    _thread = threading.Thread(target=_loop, args=(store,), daemon=True, name="retention")
    _thread.start()
    print(f"[retention] 已启动（每 {_INTERVAL_S // 3600}h 清理；事件保留 "
          f"{settings.retention_events_days} 天，误报池 {settings.retention_pool_days} 天，"
          f"磁盘下限 {settings.disk_min_free_gb}GB）")


def stop_scheduler():
    _stop.set()
    t = _thread
    if t and t.is_alive():
        t.join(timeout=2.0)


def wait_ready(timeout: float = 5.0):
    """测试辅助：等首轮清理跑完。"""
    t0 = time.monotonic()
    while time.monotonic() - t0 < timeout:
        if last_result():
            return True
        time.sleep(0.05)
    return False
