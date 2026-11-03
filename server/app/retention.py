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
from pathlib import Path

from .config import settings

_INTERVAL_S = 6 * 3600
_BATCH = 50  # 紧急清理每批删除的事件数（删完重查磁盘）
_ORPHAN_GRACE_S = 3600  # 孤儿媒体宽限：1 小时内可能仍在录制/写入
_ORPHAN_MAX_PER_RUN = 500  # 单轮最多删孤儿文件数，防一次卡死
_ORPHAN_EXTS = {".jpg", ".jpeg", ".png", ".bmp", ".webp",
                ".mp4", ".avi", ".mkv", ".mov"}

_last_result: dict = {}
_lock = threading.Lock()
_thread: threading.Thread | None = None
_stop = threading.Event()


def _safe_within(path: str, parent) -> bool:
    """路径安全校验（任务76）：真实路径包含判断——resolve 后用 is_relative_to，
    防"子串前缀"绕过（如 data-evil 与 data 同级）与 .. 穿越；不存在的文件返回 False。"""
    try:
        p = Path(path).resolve()
        base = Path(parent).resolve()
        if not p.exists():
            return False
        return p == base or p.is_relative_to(base)
    except (OSError, ValueError):
        return False


def _disk_free_gb() -> float:
    try:
        return shutil.disk_usage(str(settings.data_dir)).free / 1e9
    except Exception:  # noqa: BLE001
        return float("inf")


def _iter_orphan_media(store, referenced: set[str]):
    """遍历 snapshot_dir/video_dir 中"未被任何事件/图片记录引用且已过宽限期"的文件。

    不扫 annotation_pool（误报池有独立保留期）；路径必须在 data_dir 内；不递归子目录。
    yield (path, size_bytes)。
    """
    now = time.time()
    data_root = Path(settings.data_dir)
    for d in (settings.snapshot_dir, settings.video_dir):
        try:
            if not Path(d).is_dir():
                continue
            with os.scandir(d) as entries:
                for e in entries:
                    try:
                        if not e.is_file():
                            continue
                        if Path(e.path).suffix.lower() not in _ORPHAN_EXTS:
                            continue
                        key = os.path.normcase(os.path.abspath(e.path))
                        if key in referenced:
                            continue
                        st = e.stat()
                        if now - st.st_mtime < _ORPHAN_GRACE_S:
                            continue  # 新文件/可能仍在写入
                        if not _safe_within(e.path, data_root):
                            continue
                        yield e.path, st.st_size
                    except OSError:
                        continue
        except OSError:
            continue


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
    audit_rows = 0
    try:
        with store._conn() as c:
            # 预演只读：只统计将删除的过期审计行（87 规格口径），实际删除在 run_cleanup
            audit_rows = c.execute("SELECT COUNT(*) FROM audit_log WHERE ts < ?",
                                   (cutoff,)).fetchone()[0]
    except Exception:  # noqa: BLE001  旧库无表时跳过
        pass
    img_cutoff = (datetime.now() - timedelta(days=settings.retention_images_days)).isoformat(
        timespec="seconds")
    img_rows = 0
    try:
        with store._conn() as c:
            img_rows = c.execute(
                "SELECT COUNT(*) FROM folder_images WHERE processed_ts < ?",
                (img_cutoff,)).fetchone()[0]
    except Exception:  # noqa: BLE001  旧库无表时跳过
        pass
    orph_files, orph_bytes = 0, 0
    try:
        refs = store.referenced_media_paths()
        for _p, size in _iter_orphan_media(store, refs):
            orph_files += 1
            orph_bytes += size
    except Exception as e:  # noqa: BLE001  孤儿扫描失败不应阻塞预演其它部分
        print(f"[retention] 孤儿预演跳过: {e}")
    free = _disk_free_gb()
    return {
        "cutoff": cutoff,
        "events": {"count": ev_count, "bytes": ev_bytes},
        "pool": {"files": pool_files, "bytes": pool_bytes},
        "audit": {"count": audit_rows},
        "images": {"count": img_rows, "cutoff": img_cutoff},
        "orphans": {"files": orph_files, "bytes": orph_bytes},
        "disk_free_gb": round(free, 2),
        "estimated_free_gb": round(free + (ev_bytes + pool_bytes + orph_bytes) / 1e9, 2),
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

    # 3.5) 审计日志过期行（任务87：与事件同保留口径）
    try:
        with store._lock, store._conn() as c:
            stats["deleted_audit"] = c.execute(
                "DELETE FROM audit_log WHERE ts < ?", (cutoff,)).rowcount
    except Exception as e:  # noqa: BLE001  旧库无 audit_log 表时跳过
        stats["deleted_audit"] = 0
        print(f"[retention] 审计清理跳过: {e}")

    # 3.6) 图片文件夹识别记录（任务121）：只删派生的证据副本与缩略图，原始截图永不删除
    img_cutoff = (datetime.now() - timedelta(days=settings.retention_images_days)).isoformat(
        timespec="seconds")
    deleted_images = 0
    try:
        while True:
            rows = store.folder_images_older_than(img_cutoff, limit=_BATCH)
            if not rows:
                break
            for r in rows:
                p = r["snapshot"]
                if p and _safe_within(p, settings.data_dir) \
                        and not store.event_snapshot_referenced(p):
                    try:
                        stats["freed_mb"] += os.path.getsize(p) / 1e6
                        os.remove(p)
                        stats["deleted_files"] += 1
                    except OSError as e:
                        print(f"[retention] 图片证据删除失败 {p}: {e}")
                thumb = Path(str(settings.data_dir)) / "thumbs" / f"img_{r['id']}.jpg"
                try:
                    thumb.unlink(missing_ok=True)
                except OSError:
                    pass
                store.delete_folder_image(r["id"])
                deleted_images += 1
            if len(rows) < _BATCH:
                break
    except Exception as e:  # noqa: BLE001  旧库无 folder_images 表时不阻塞其它清理
        print(f"[retention] 图片记录清理跳过: {e}")
    stats["deleted_images"] = deleted_images

    # 3.7) 孤儿媒体（任务107）：无事件/图片记录引用、超过 1h 宽限、data_dir 内的媒体文件
    deleted_orphans, freed_orphan_bytes, orphans_limited = 0, 0, False
    try:
        refs = store.referenced_media_paths()
        for p, size in _iter_orphan_media(store, refs):
            if deleted_orphans >= _ORPHAN_MAX_PER_RUN:
                orphans_limited = True
                break
            try:
                os.remove(p)
                deleted_orphans += 1
                freed_orphan_bytes += size
            except FileNotFoundError:
                pass
            except OSError as e:
                print(f"[retention] 孤儿媒体删除失败 {p}: {e}")
    except Exception as e:  # noqa: BLE001  孤儿扫描失败不阻塞其它清理
        print(f"[retention] 孤儿媒体清理跳过: {e}")
    stats["deleted_orphans"] = deleted_orphans
    stats["freed_orphan_mb"] = round(freed_orphan_bytes / 1e6, 3)
    stats["orphans_limited"] = orphans_limited
    if deleted_orphans:
        print(f"[retention] 孤儿媒体清理 {deleted_orphans} 个 / "
              f"{stats['freed_orphan_mb']:.2f}MB"
              + ("（达单轮上限，剩余下轮）" if orphans_limited else ""))

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
