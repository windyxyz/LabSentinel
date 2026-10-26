"""SQLite 存储（stdlib sqlite3；全部使用字面量 SQL + 参数占位符）。"""
import json
import sqlite3
import threading
from datetime import datetime

from .config import settings

_SCHEMA = """
CREATE TABLE IF NOT EXISTS cameras(
  id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, source TEXT, type TEXT, status TEXT DEFAULT 'stopped');
CREATE TABLE IF NOT EXISTS zones(
  id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, name TEXT, polygon TEXT);
CREATE TABLE IF NOT EXISTS rules(
  id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, zone_id INTEGER,
  behavior TEXT, level INTEGER, cooldown_s INTEGER,
  time_start TEXT, time_end TEXT, threshold INTEGER, enabled INTEGER);
CREATE TABLE IF NOT EXISTS events(
  id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT, camera_id INTEGER, type TEXT,
  level INTEGER, track_id INTEGER, detail TEXT, snapshot TEXT, video TEXT,
  status TEXT DEFAULT 'new', handled_by TEXT, handled_ts TEXT);
CREATE TABLE IF NOT EXISTS face_consents(
  subject TEXT PRIMARY KEY, ts TEXT);
CREATE TABLE IF NOT EXISTS alert_deliveries(
  id INTEGER PRIMARY KEY AUTOINCREMENT, event_id INTEGER, channel TEXT,
  status TEXT DEFAULT 'pending', attempts INTEGER DEFAULT 0,
  last_error TEXT, updated_ts TEXT);
CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts);
CREATE INDEX IF NOT EXISTS idx_events_camera ON events(camera_id);
"""

# 事件处置状态流（任务12）：new → acked（已确认）| false_positive（误报）| ignored（忽略）
EVENT_STATUSES = ("new", "acked", "false_positive", "ignored")


def _migrate(c: sqlite3.Connection):
    """旧库兼容：缺列时逐列补（ALTER TABLE ... ADD COLUMN）。"""
    cols = {r[1] for r in c.execute("PRAGMA table_info(events)")}
    if "video" not in cols:
        c.execute("ALTER TABLE events ADD COLUMN video TEXT")
    if "status" not in cols:
        c.execute("ALTER TABLE events ADD COLUMN status TEXT DEFAULT 'new'")
    if "handled_by" not in cols:
        c.execute("ALTER TABLE events ADD COLUMN handled_by TEXT")
    if "handled_ts" not in cols:
        c.execute("ALTER TABLE events ADD COLUMN handled_ts TEXT")


class Store:
    def __init__(self, path=None):
        self.path = str(path or settings.db_path)
        self._lock = threading.Lock()
        with self._conn() as c:
            c.executescript(_SCHEMA)
            _migrate(c)

    def _conn(self):
        c = sqlite3.connect(self.path)
        c.row_factory = sqlite3.Row
        return c

    # ---- cameras ----
    def cameras(self):
        with self._conn() as c:
            return [dict(r) for r in c.execute("SELECT * FROM cameras ORDER BY id")]

    def camera(self, cid):
        with self._conn() as c:
            r = c.execute("SELECT * FROM cameras WHERE id=?", (cid,)).fetchone()
            return dict(r) if r else None

    def add_camera(self, name, source, type_):
        with self._lock, self._conn() as c:
            cur = c.execute("INSERT INTO cameras(name,source,type) VALUES(?,?,?)", (name, source, type_))
            return cur.lastrowid

    def delete_camera(self, cid):
        with self._lock, self._conn() as c:
            c.execute("DELETE FROM cameras WHERE id=?", (cid,))
            c.execute("DELETE FROM zones WHERE camera_id=?", (cid,))
            c.execute("DELETE FROM rules WHERE camera_id=?", (cid,))

    def set_camera_status(self, cid, status):
        with self._lock, self._conn() as c:
            c.execute("UPDATE cameras SET status=? WHERE id=?", (status, cid))

    # ---- zones ----
    def zones(self, camera_id=None):
        if camera_id is None:
            with self._conn() as c:
                rows = [dict(r) for r in c.execute("SELECT * FROM zones ORDER BY id")]
        else:
            with self._conn() as c:
                rows = [dict(r) for r in c.execute(
                    "SELECT * FROM zones WHERE camera_id=? ORDER BY id", (camera_id,))]
        for r in rows:
            r["polygon"] = json.loads(r["polygon"])
        return rows

    def add_zone(self, camera_id, name, polygon):
        with self._lock, self._conn() as c:
            cur = c.execute(
                "INSERT INTO zones(camera_id,name,polygon) VALUES(?,?,?)",
                (camera_id, name, json.dumps(polygon)),
            )
            return cur.lastrowid

    def delete_zone(self, zid):
        with self._lock, self._conn() as c:
            c.execute("DELETE FROM zones WHERE id=?", (zid,))

    # ---- rules ----
    def rules(self, camera_id=None):
        if camera_id is None:
            with self._conn() as c:
                rows = [dict(r) for r in c.execute("SELECT * FROM rules ORDER BY id")]
        else:
            with self._conn() as c:
                rows = [dict(r) for r in c.execute(
                    "SELECT * FROM rules WHERE camera_id=? ORDER BY id", (camera_id,))]
        for r in rows:
            r["enabled"] = bool(r["enabled"])
        return rows

    def add_rule(self, r: dict):
        with self._lock, self._conn() as c:
            cur = c.execute(
                "INSERT INTO rules(camera_id,zone_id,behavior,level,cooldown_s,time_start,time_end,threshold,enabled)"
                " VALUES(?,?,?,?,?,?,?,?,?)",
                (r["camera_id"], r.get("zone_id"), r["behavior"], r.get("level", 2),
                 r.get("cooldown_s", 60), r.get("time_start", "00:00"), r.get("time_end", "23:59"),
                 r.get("threshold", 0), 1 if r.get("enabled", True) else 0),
            )
            return cur.lastrowid

    def delete_rule(self, rid):
        with self._lock, self._conn() as c:
            c.execute("DELETE FROM rules WHERE id=?", (rid,))

    # ---- events ----
    def add_event(self, camera_id, type_, level, track_id, detail, snapshot=None) -> int:
        with self._lock, self._conn() as c:
            cur = c.execute(
                "INSERT INTO events(ts,camera_id,type,level,track_id,detail,snapshot) VALUES(?,?,?,?,?,?,?)",
                (datetime.now().isoformat(timespec="seconds"), camera_id, type_, level, track_id, detail, snapshot),
            )
            return cur.lastrowid

    def events(self, limit=50, camera_id=None, level=None, status=None,
               offset=0, start_ts=None, end_ts=None):
        """事件查询；status 过滤（'unhandled'='new'）；offset 分页；start_ts/end_ts 时间范围（含边界）。"""
        where, args = [], []
        if camera_id is not None:
            where.append("camera_id=?")
            args.append(camera_id)
        if level is not None:
            where.append("level=?")
            args.append(level)
        if status is not None:
            where.append("status=?")
            args.append("new" if status == "unhandled" else status)
        if start_ts:
            where.append("ts >= ?")
            args.append(start_ts)
        if end_ts:
            where.append("ts <= ?")
            args.append(end_ts)
        sql = "SELECT * FROM events"
        if where:
            sql += " WHERE " + " AND ".join(where)
        sql += " ORDER BY id DESC LIMIT ? OFFSET ?"
        args.extend([limit, max(0, offset)])
        with self._conn() as c:
            return [dict(r) for r in c.execute(sql, args)]

    def count_events(self, camera_id=None, level=None, status=None,
                     start_ts=None, end_ts=None) -> int:
        """与 events 相同过滤条件的总数（任务19 count 端点）。"""
        where, args = [], []
        if camera_id is not None:
            where.append("camera_id=?")
            args.append(camera_id)
        if level is not None:
            where.append("level=?")
            args.append(level)
        if status is not None:
            where.append("status=?")
            args.append("new" if status == "unhandled" else status)
        if start_ts:
            where.append("ts >= ?")
            args.append(start_ts)
        if end_ts:
            where.append("ts <= ?")
            args.append(end_ts)
        sql = "SELECT COUNT(*) n FROM events"
        if where:
            sql += " WHERE " + " AND ".join(where)
        with self._conn() as c:
            return c.execute(sql, args).fetchone()["n"]

    def events_older_than_first(self, cutoff, camera_id=None, status=None,
                                before_id=None, limit=200):
        """导出用分页游标：取 cutoff（窗口起点）**之后**的事件（id < before_id 续页），新→旧。"""
        where, args = ["ts >= ?"], [cutoff]
        if camera_id is not None:
            where.append("camera_id=?")
            args.append(camera_id)
        if status is not None:
            where.append("status=?")
            args.append("new" if status == "unhandled" else status)
        if before_id is not None:
            where.append("id < ?")
            args.append(before_id)
        sql = ("SELECT * FROM events WHERE " + " AND ".join(where) +
               " ORDER BY id DESC LIMIT ?")
        args.append(limit)
        with self._conn() as c:
            return [dict(r) for r in c.execute(sql, args)]

    def events_between(self, start_iso, end_iso):
        with self._conn() as c:
            return [dict(r) for r in c.execute(
                "SELECT * FROM events WHERE ts >= ? AND ts < ? ORDER BY ts", (start_iso, end_iso))]

    def set_event_status(self, eid, status, handled_by="admin"):
        """处置事件：更新 status/handled_by/handled_ts。非法状态由调用方（API 层）校验。"""
        with self._lock, self._conn() as c:
            cur = c.execute(
                "UPDATE events SET status=?, handled_by=?, handled_ts=? WHERE id=?",
                (status, handled_by, datetime.now().isoformat(timespec="seconds"), eid))
            return cur.rowcount > 0

    def event_snapshot(self, eid):
        with self._conn() as c:
            r = c.execute("SELECT snapshot FROM events WHERE id=?", (eid,)).fetchone()
            return r["snapshot"] if r else None

    def set_event_video(self, eid, path):
        with self._lock, self._conn() as c:
            c.execute("UPDATE events SET video=? WHERE id=?", (path, eid))

    def event_video(self, eid):
        with self._conn() as c:
            r = c.execute("SELECT video FROM events WHERE id=?", (eid,)).fetchone()
            return r["video"] if r else None

    # ---- 人脸同意记录（合规：注册人员必须留痕，见 app/face.py）----
    def add_face_consent(self, subject):
        with self._lock, self._conn() as c:
            c.execute("INSERT OR REPLACE INTO face_consents(subject,ts) VALUES(?,?)",
                      (subject, datetime.now().isoformat(timespec="seconds")))

    def face_consents(self):
        with self._conn() as c:
            return [dict(r) for r in c.execute("SELECT * FROM face_consents ORDER BY ts")]

    def face_consent_ts(self, subject):
        with self._conn() as c:
            r = c.execute("SELECT ts FROM face_consents WHERE subject=?", (subject,)).fetchone()
            return r["ts"] if r else None

    def clear_face_consents(self):
        with self._lock, self._conn() as c:
            c.execute("DELETE FROM face_consents")

    def today_event_counts(self):
        today = datetime.now().strftime("%Y-%m-%d")
        with self._conn() as c:
            rows = c.execute(
                "SELECT level, COUNT(*) n FROM events WHERE ts LIKE ? GROUP BY level", (today + "%",)
            ).fetchall()
        return {r["level"]: r["n"] for r in rows}

    def count_events_today_handled(self):
        """今日已被处置（status != 'new'）的事件数——仪表盘"未处置"计算用。"""
        today = datetime.now().strftime("%Y-%m-%d")
        with self._conn() as c:
            r = c.execute(
                "SELECT COUNT(*) n FROM events WHERE ts LIKE ? AND status != 'new'",
                (today + "%",)).fetchone()
        return r["n"] if r else 0

    # ---- 告警投递记录（任务13：重试/死信/可观测）----
    def add_delivery(self, event_id: int, channel: str) -> int:
        with self._lock, self._conn() as c:
            cur = c.execute(
                "INSERT INTO alert_deliveries(event_id,channel,status,attempts,updated_ts)"
                " VALUES(?,?,'pending',0,?)", (event_id, channel,
                                               datetime.now().isoformat(timespec="seconds")))
            return cur.lastrowid

    def update_delivery(self, did: int, status: str, attempts: int, last_error: str | None = None):
        with self._lock, self._conn() as c:
            c.execute(
                "UPDATE alert_deliveries SET status=?, attempts=?, last_error=?, updated_ts=? WHERE id=?",
                (status, attempts, last_error, datetime.now().isoformat(timespec="seconds"), did))

    def deliveries(self, status: str | None = None, limit: int = 50):
        if status:
            sql = "SELECT * FROM alert_deliveries WHERE status=? ORDER BY id DESC LIMIT ?"
            args: list = [status, limit]
        else:
            sql = "SELECT * FROM alert_deliveries ORDER BY id DESC LIMIT ?"
            args = [limit]
        with self._conn() as c:
            return [dict(r) for r in c.execute(sql, args)]

    def delivery(self, did: int):
        with self._conn() as c:
            r = c.execute("SELECT * FROM alert_deliveries WHERE id=?", (did,)).fetchone()
            return dict(r) if r else None

    def delivery_counts(self) -> dict:
        with self._conn() as c:
            rows = c.execute(
                "SELECT status, COUNT(*) n FROM alert_deliveries GROUP BY status").fetchall()
        d = {r["status"]: r["n"] for r in rows}
        return {"pending": d.get("pending", 0), "failed": d.get("failed", 0),
                "ok": d.get("ok", 0)}

    def delivery_counts_by_channel(self) -> dict:
        """按渠道分组投递计数（任务23A 渠道状态页）：{channel: {ok, failed, pending}}。"""
        with self._conn() as c:
            rows = c.execute(
                "SELECT channel, status, COUNT(*) n FROM alert_deliveries GROUP BY channel, status"
            ).fetchall()
        out: dict = {}
        for r in rows:
            out.setdefault(r["channel"], {"ok": 0, "failed": 0, "pending": 0})
            if r["status"] in ("ok", "failed", "pending"):
                out[r["channel"]][r["status"]] = r["n"]
        return out
