"""SQLite 存储（stdlib sqlite3；全部使用字面量 SQL + 参数占位符）。"""
import json
import logging
import sqlite3
import threading
from datetime import datetime, timedelta
from pathlib import Path

from .config import settings

log = logging.getLogger("app.store")


def ro_uri(path) -> str:
    """只读 SQLite URI（任务76）：Path.as_uri() 正确百分号编码空格/中文/#/? 等，
    避免 f"file:{path}?mode=ro" 在特殊字符路径下解析失败。"""
    return Path(path).resolve().as_uri() + "?mode=ro"


def connect(path, *, readonly: bool = False) -> sqlite3.Connection:
    """统一连接入口（任务64）：每条连接设 busy_timeout（PRAGMA 是 per-connection 的）。

    WAL 模式下锁竞争会返回 busy 而非立即抛错，等 busy_timeout 毫秒后再报。
    """
    if readonly:
        c = sqlite3.connect(ro_uri(path), uri=True)
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


def _mig_002(c: sqlite3.Connection):
    """v2：alert_silences 表（任务81 告警静默，持久化重启不丢）。"""
    c.execute("""CREATE TABLE IF NOT EXISTS alert_silences(
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      scope TEXT NOT NULL,
      target TEXT,
      reason TEXT,
      created_ts TEXT NOT NULL,
      until_ts TEXT NOT NULL,
      created_by TEXT)""")
    c.execute("CREATE INDEX IF NOT EXISTS idx_silence_until ON alert_silences(until_ts)")


def _mig_003(c: sqlite3.Connection):
    """v3：events 加 escalated_ts（任务82 未处置升级提醒）+ 状态/时间索引。"""
    cols = {r[1] for r in c.execute("PRAGMA table_info(events)")}
    if "escalated_ts" not in cols:
        c.execute("ALTER TABLE events ADD COLUMN escalated_ts TEXT")
    c.execute("CREATE INDEX IF NOT EXISTS idx_events_status_ts ON events(status, ts)")


def _mig_004(c: sqlite3.Connection):
    """v4：audit_log（任务87 操作审计）。"""
    c.execute("""CREATE TABLE IF NOT EXISTS audit_log(
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      ts TEXT NOT NULL,
      request_id TEXT,
      actor TEXT,
      action TEXT NOT NULL,
      target TEXT,
      status_code INTEGER,
      detail TEXT)""")
    c.execute("CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_log(ts)")


# 版本化迁移（任务69）：只做加列/加表/索引，不做破坏性变更；每个迁移幂等、在事务内执行
MIGRATIONS: list[tuple[int, str, object]] = [
    (1, "events 增加 video/status/handled_by/handled_ts", _mig_001),
    (2, "新增 alert_silences 表（任务81 告警静默）", _mig_002),
    (3, "events 增加 escalated_ts（任务82 未处置升级提醒）", _mig_003),
    (4, "新增 audit_log 表（任务87 操作审计）", _mig_004),
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
        """执行未应用的迁移（任务69/81）。旧库兼容：无版本记录但 events 已含 v1 目标列
        （前框架时代手工补列的库）→ 仅标记 v1 已应用，**后续迁移照常执行**。"""
        current = c.execute("SELECT COALESCE(MAX(version), 0) FROM schema_version").fetchone()[0]
        if current == 0:
            cols = {r[1] for r in c.execute("PRAGMA table_info(events)")}
            need_cols = {"video", "status", "handled_by", "handled_ts"}
            if need_cols <= cols:
                # 81 修复：只标 v1。曾误标全部版本 → 全新库/旧库都跳过 v2+（alert_silences
                # 等新表缺失，单测抓到）。v1 在全新库上幂等无操作，标它只为旧库免重复 ALTER。
                now = datetime.now().isoformat(timespec="seconds")
                if MIGRATIONS:
                    c.execute("INSERT OR IGNORE INTO schema_version(version, applied_at) "
                              "VALUES(?,?)", (MIGRATIONS[0][0], now))
                current = MIGRATIONS[0][0] if MIGRATIONS else 0
                c.commit()  # 探测的 INSERT 有隐式事务，不提交会让下面 v2 的 BEGIN 撞车
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

    def clear_config(self):
        """清空 cameras/zones/rules（任务79 replace 导入用）；events 等用户数据不动。"""
        with self._lock, self._conn() as c:
            c.execute("DELETE FROM cameras")
            c.execute("DELETE FROM zones")
            c.execute("DELETE FROM rules")

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

    def event_by_id(self, eid: int) -> dict | None:
        """按 id 单查事件（2026-10-30 审阅新增：替代 events(limit=1) 取"最新行"的竞态写法）。"""
        with self._conn() as c:
            r = c.execute("SELECT * FROM events WHERE id=?", (eid,)).fetchone()
            return dict(r) if r else None

    def zone(self, zid: int) -> dict | None:
        """按 id 单查安全区（add_rule 存在性校验用）。"""
        with self._conn() as c:
            r = c.execute("SELECT * FROM zones WHERE id=?", (zid,)).fetchone()
            return dict(r) if r else None

    def delivery_by_id(self, did: int) -> dict | None:
        """按 id 单查投递记录（同上，替代全表拉取）。"""
        with self._conn() as c:
            r = c.execute("SELECT * FROM alert_deliveries WHERE id=?", (did,)).fetchone()
            return dict(r) if r else None

    # ---- 告警静默（任务81）----
    def add_silence(self, scope: str, target: str | None, reason: str | None,
                    created_by: str | None = None, duration_s: int = 3600) -> dict:
        now = datetime.now()
        with self._lock, self._conn() as c:
            cur = c.execute(
                "INSERT INTO alert_silences(scope,target,reason,created_ts,until_ts,created_by) "
                "VALUES(?,?,?,?,?,?)",
                (scope, target, reason, now.isoformat(timespec="seconds"),
                 (now + timedelta(seconds=duration_s)).isoformat(timespec="seconds"), created_by))
            sid = cur.lastrowid
        return self.silence(sid)

    def silence(self, sid: int) -> dict | None:
        with self._conn() as c:
            r = c.execute("SELECT * FROM alert_silences WHERE id=?", (sid,)).fetchone()
            return dict(r) if r else None

    def silences(self, active_only: bool = True) -> list[dict]:
        now = datetime.now().isoformat(timespec="seconds")
        sql = "SELECT * FROM alert_silences"
        if active_only:
            sql += " WHERE until_ts > ?"
        rows = self._conn().execute(sql, (now,) if active_only else ()).fetchall()
        return [dict(r) for r in rows]

    def delete_silence(self, sid: int) -> bool:
        with self._lock, self._conn() as c:
            n = c.execute("DELETE FROM alert_silences WHERE id=?", (sid,)).rowcount
        return n > 0

    # ---- 操作审计（任务87）----
    def add_audit(self, ts: str, request_id: str | None, actor: str, action: str,
                  target: str | None, status_code: int) -> None:
        """写审计行（失败由调用方兜底 WARNING，不影响业务响应）。"""
        with self._lock, self._conn() as c:
            c.execute("INSERT INTO audit_log(ts,request_id,actor,action,target,status_code) "
                      "VALUES(?,?,?,?,?,?)",
                      (ts, request_id, actor, action, target, status_code))

    def audit(self, limit=50, offset=0, action: str | None = None,
              actor: str | None = None, start_ts: str | None = None,
              end_ts: str | None = None) -> tuple[list[dict], int]:
        """审计查询（新→旧），返回 (items, total)。"""
        where, args = [], []
        if action:
            where.append("action LIKE ?")
            args.append(action + "%")  # 前缀匹配（如 action=POST /api/cameras）
        if actor:
            where.append("actor=?")
            args.append(actor)
        if start_ts:
            where.append("ts >= ?")
            args.append(start_ts)
        if end_ts:
            where.append("ts <= ?")
            args.append(end_ts)
        cond = " WHERE " + " AND ".join(where) if where else ""
        with self._conn() as c:
            total = c.execute("SELECT COUNT(*) FROM audit_log" + cond, args).fetchone()[0]
            rows = c.execute("SELECT * FROM audit_log" + cond +
                             " ORDER BY id DESC LIMIT ? OFFSET ?",
                             [*args, max(1, min(limit, 200)), max(0, offset)]).fetchall()
        return [dict(r) for r in rows], total

    @staticmethod
    def _like_escape(q: str) -> str:
        """任务84：LIKE 通配符转义（配合 ESCAPE '\\'），用户输入的 %/_ 按字面匹配。"""
        return q.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_")

    @staticmethod
    def _q_clause(q: str | None, where: list, args: list) -> None:
        """q 关键词过滤：对 detail/type 做安全 LIKE（1–100 字符，去空白，空则不过滤）。"""
        if q is None:
            return
        q = q.strip()[:100]
        if not q:
            return
        where.append("(detail LIKE '%'||?||'%' ESCAPE '\\' OR type LIKE '%'||?||'%' ESCAPE '\\')")
        args.extend([Store._like_escape(q), Store._like_escape(q)])

    def events(self, limit=50, camera_id=None, level=None, status=None,
               offset=0, start_ts=None, end_ts=None, q: str | None = None):
        """事件查询；status 过滤（'unhandled'='new'）；offset 分页；start_ts/end_ts 时间范围（含边界）；
        q 关键词（任务84，detail/type LIKE，通配符转义）。"""
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
        self._q_clause(q, where, args)
        sql = "SELECT * FROM events"
        if where:
            sql += " WHERE " + " AND ".join(where)
        sql += " ORDER BY id DESC LIMIT ? OFFSET ?"
        args.extend([limit, max(0, offset)])
        with self._conn() as c:
            return [dict(r) for r in c.execute(sql, args)]

    def count_events(self, camera_id=None, level=None, status=None,
                     start_ts=None, end_ts=None, q: str | None = None) -> int:
        """与 events 相同过滤条件的总数（任务19 count 端点；任务84 q 关键词）。"""
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
        self._q_clause(q, where, args)
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
