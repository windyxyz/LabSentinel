"""SQLite 存储（stdlib sqlite3；全部使用字面量 SQL + 参数占位符）。"""
import json
import logging
import sqlite3
import threading
from datetime import datetime

from .config import settings

log = logging.getLogger("app.store")


def connect(path, *, readonly: bool = False) -> sqlite3.Connection:
    """统一连接入口（任务64）：每条连接设 busy_timeout（PRAGMA 是 per-connection 的）。

    WAL 模式下锁竞争会返回 busy 而非立即抛错，等 busy_timeout 毫秒后再报。
    """
    if readonly:
        c = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    else:
        c = sqlite3.connect(path)
    c.row_factory = sqlite3.Row
    c.execute(f"PRAGMA busy_timeout={int(settings.sqlite_busy_timeout_ms)}")
    c.execute("PRAGMA synchronous=NORMAL")  # per-connection；WAL 下 NORMAL 够用且更快
    return c

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


def _mig_001(c: sqlite3.Connection):
    """v1：events 补 video/status/handled_by/handled_ts 列（幂等：缺哪列补哪列）。"""
    cols = {r[1] for r in c.execute("PRAGMA table_info(events)")}
    if "video" not in cols:
        c.execute("ALTER TABLE events ADD COLUMN video TEXT")
    if "status" not in cols:
        c.execute("ALTER TABLE events ADD COLUMN status TEXT DEFAULT 'new'")
    if "handled_by" not in cols:
        c.execute("ALTER TABLE events ADD COLUMN handled_by TEXT")
    if "handled_ts" not in cols:
        c.execute("ALTER TABLE events ADD COLUMN handled_ts TEXT")


# 版本化迁移（任务69）：只做加列/加表/索引，不做破坏性变更；每个迁移幂等、在事务内执行
MIGRATIONS: list[tuple[int, str, object]] = [
    (1, "events 增加 video/status/handled_by/handled_ts", _mig_001),
]
SCHEMA_VERSION_LATEST = max(v for v, _d, _f in MIGRATIONS)


class Store:
    def __init__(self, path=None):
        self.path = str(path or settings.db_path)
        self._lock = threading.Lock()
        with self._conn() as c:
            self._enable_wal(c)  # 任务64：WAL + synchronous=NORMAL（建库一次；失败只警告）
            c.executescript(_SCHEMA)
            c.execute("CREATE TABLE IF NOT EXISTS schema_version("
                      "version INTEGER PRIMARY KEY, applied_at TEXT)")
            self._migrate(c)

    def schema_version(self) -> int:
        with self._conn() as c:
            row = c.execute("SELECT COALESCE(MAX(version), 0) FROM schema_version").fetchone()
            return int(row[0])

    @staticmethod
    def _migrate(c: sqlite3.Connection) -> None:
        """执行未应用的迁移（任务69）。旧库兼容：无版本记录但结构已最新 → 直接标记为最新。"""
        current = c.execute("SELECT COALESCE(MAX(version), 0) FROM schema_version").fetchone()[0]
        if current == 0:
            # 探测：无版本记录的旧库，若最新迁移的目标结构已齐（events 全部目标列存在），
            # 视为"已迁移过"直接标记最新；部分缺列则从 v1 逐个跑（迁移函数幂等补列）
            cols = {r[1] for r in c.execute("PRAGMA table_info(events)")}
            need_cols = {"video", "status", "handled_by", "handled_ts"}
            if need_cols <= cols:
                now = datetime.now().isoformat(timespec="seconds")
                for v, _d, _fn in MIGRATIONS:
                    c.execute("INSERT OR IGNORE INTO schema_version(version, applied_at) "
                              "VALUES(?,?)", (v, now))
                current = SCHEMA_VERSION_LATEST
        for ver, desc, fn in MIGRATIONS:
            if ver <= current:
                continue
            # 显式 BEGIN/COMMIT：python sqlite3 默认只对 DML 开隐式事务，DDL 会立即自动提交——
            # 必须 BEGIN 才能拿到 SQLite 原生的事务性 DDL（失败迁移整体回滚，含 ALTER/CREATE）
            try:
                c.execute("BEGIN")
                fn(c)
                c.execute("INSERT INTO schema_version(version, applied_at) VALUES(?,?)",
                          (ver, datetime.now().isoformat(timespec="seconds")))
                c.execute("COMMIT")
            except Exception as e:  # noqa: BLE001
                try:
                    c.execute("ROLLBACK")
                except sqlite3.Error:
                    pass
                raise RuntimeError(f"数据库迁移 v{ver}（{desc}）失败：{e}——"
                                   "请先备份数据库后反馈此错误") from e

    @staticmethod
    def _enable_wal(c: sqlite3.Connection) -> None:
        try:
            mode = c.execute("PRAGMA journal_mode=WAL").fetchone()[0]
            if str(mode).lower() != "wal":
                log.warning("journal_mode 返回 %s（期望 wal）——内存/只读库属预期，继续用默认模式", mode)
        except sqlite3.DatabaseError as e:
            log.warning("WAL 启用失败（%s），继续用默认 journal 模式", e)

    def _conn(self):
        return connect(self.path)

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


# 兼容别名：旧测试/外部脚本直接 import _migrate（现实现为 Store._migrate 静态方法）
_migrate = Store._migrate
