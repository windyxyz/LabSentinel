"""未处置事件升级提醒（任务82）：L3（可配）事件超时未处置 → 兜底再提醒一次。

- 后台线程每 60s 检查（与 retention 同模式；异常捕获不拖垮服务）；
- 命中条件：status=new 且 escalated_ts IS NULL 且 ts < now-after_min 且 level >= min_level；
- 提醒 = 复用现有渠道发一条（render_alert + ⏰ 前缀）；渠道未配置只记日志不建投递行
  （与任务 51 configured 语义一致）；
- 提醒后写 escalated_ts（发送失败也标记，防每分钟重试风暴）；每轮最多 batch 条防启动洪峰。
- `LS_ESCALATE_AFTER_MIN=0` 关闭。
"""
import threading
import time
from datetime import datetime, timedelta

from .config import settings

_INTERVAL_S = 60
_thread: threading.Thread | None = None
_stop = threading.Event()
_last_result: dict = {}
_lock = threading.Lock()


def find_unhandled(store, now: datetime, after_min: int, min_level: int, batch: int) -> list[dict]:
    """超时未处置且未提醒过的事件（旧→新排序，限 batch 条）。"""
    cutoff = (now - timedelta(minutes=after_min)).isoformat(timespec="seconds")
    with store._conn() as c:
        rows = c.execute(
            "SELECT * FROM events WHERE status='new' AND escalated_ts IS NULL "
            "AND ts < ? AND level >= ? ORDER BY id ASC LIMIT ?",
            (cutoff, min_level, batch)).fetchall()
    return [dict(r) for r in rows]


def mark_escalated(store, eid: int, ts: datetime) -> None:
    with store._lock, store._conn() as c:
        c.execute("UPDATE events SET escalated_ts=? WHERE id=?",
                  (ts.isoformat(timespec="seconds"), eid))


def escalate_once(store, now: datetime | None = None, send=None) -> dict:
    """跑一轮：返回 {checked, escalated}。send 可注入（测试）；缺省走 dispatcher。"""
    after_min = settings.escalate_after_min
    if after_min <= 0:
        return {"checked": 0, "escalated": 0, "disabled": True}
    now = now or datetime.now()
    from .alerts import dispatcher, render_alert
    rows = find_unhandled(store, now, after_min,
                          settings.escalate_min_level, settings.escalate_batch)
    escalated = 0
    for r in rows:
        waited = int((now - datetime.fromisoformat(r["ts"])).total_seconds() // 60)
        text = f"⏰ 未处置提醒（已 {waited} 分钟）\n{render_alert(r)}"
        if send is not None:
            ok = bool(send(r, text))
        else:
            ok = _dispatch_reminder(r, text)
        mark_escalated(store, r["id"], now)
        escalated += 1
        if not ok:
            print(f"[escalate] event#{r['id']} 提醒发送失败（已标记 escalated_ts，不重试风暴）")
    with _lock:
        _last_result.clear()
        _last_result.update({"checked": len(rows), "escalated": escalated,
                             "ts": now.isoformat(timespec="seconds")})
    return {"checked": len(rows), "escalated": escalated}


def _dispatch_reminder(r: dict, text: str) -> bool:
    """提醒走现有渠道（复用 send 路径：未配置渠道自动跳过、不建投递行）。"""
    from .alerts import dispatcher
    if dispatcher.store is None:
        dispatcher.bind_store(dispatcher.store or _noop_store())
    ok = True
    for ch in dispatcher.channels:
        cfg = getattr(ch, "configured", None)
        if callable(cfg) and not cfg():
            continue  # 未配置渠道：跳过不建投递行（与 51 语义一致）
        try:
            if not ch.send(r["level"], text):
                ok = False
        except Exception as e:  # noqa: BLE001
            print(f"[escalate] {getattr(ch, 'name', ch)} 提醒失败: {e}")
            ok = False
    return ok


class _noop_store:
    """dispatcher.store 为 None 时的占位（提醒场景不落投递行）。"""

    def add_delivery(self, *a, **k):
        return None

    def update_delivery(self, *a, **k):
        return None


def status(store) -> dict:
    now = datetime.now()
    pending = 0
    escalated_today = 0
    try:
        today = now.date().isoformat()
        with store._conn() as c:
            pending = c.execute(
                "SELECT COUNT(*) FROM events WHERE status='new' AND level >= ?",
                (settings.escalate_min_level,)).fetchone()[0]
            escalated_today = c.execute(
                "SELECT COUNT(*) FROM events WHERE escalated_ts LIKE ?", (f"{today}%",)).fetchone()[0]
    except Exception:  # noqa: BLE001
        pass
    return {"enabled": settings.escalate_after_min > 0,
            "after_min": settings.escalate_after_min,
            "min_level": settings.escalate_min_level,
            "pending": pending, "escalated_today": escalated_today,
            "last": dict(_last_result)}


def _loop(store):
    while not _stop.is_set():
        try:
            escalate_once(store)
        except Exception as e:  # noqa: BLE001
            print(f"[escalate] 检查异常: {e}")
        _stop.wait(_INTERVAL_S)


def start_scheduler(store):
    global _thread
    _stop.clear()
    _thread = threading.Thread(target=_loop, args=(store,), daemon=True, name="escalation")
    _thread.start()
    print(f"[escalate] 已启动（每 {_INTERVAL_S}s 检查；超 {settings.escalate_after_min}min 未处置 "
          f"L{settings.escalate_min_level}+ 提醒一次，batch={settings.escalate_batch}）")


def stop_scheduler():
    _stop.set()
    t = _thread
    if t and t.is_alive():
        t.join(timeout=2.0)
