"""SQLite 存储（stdlib sqlite3；全部使用字面量 SQL + 参数占位符）。"""
import json
import logging
import os
import sqlite3
import contextlib
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
    """统一连接入口（任务64）：busy_timeout 用 sqlite3.connect(timeout=秒) 设置（API 参数，
    等价 per-connection PRAGMA busy_timeout；不拼 SQL 语句）。

    WAL 模式下锁竞争会返回 busy 而非立即抛错，等 busy_timeout 毫秒后再报。
    """
    timeout_s = max(0.0, int(settings.sqlite_busy_timeout_ms)) / 1000.0
    if readonly:
        c = sqlite3.connect(ro_uri(path), uri=True, timeout=timeout_s)
    else:
        c = sqlite3.connect(path, timeout=timeout_s)
    c.row_factory = sqlite3.Row
    c.execute("PRAGMA synchronous=NORMAL")  # per-connection；WAL 下 NORMAL 够用且更快
    return c

_SCHEMA = """
CREATE TABLE IF NOT EXISTS cameras(
  id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, source TEXT, type TEXT, status TEXT DEFAULT 'stopped',
  group_name TEXT DEFAULT '');
CREATE TABLE IF NOT EXISTS zones(
  id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, name TEXT, polygon TEXT);
CREATE TABLE IF NOT EXISTS rules(
  id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, zone_id INTEGER,
  behavior TEXT, level INTEGER, cooldown_s INTEGER,
  time_start TEXT, time_end TEXT, threshold INTEGER, enabled INTEGER,
  days_of_week TEXT DEFAULT '1,2,3,4,5,6,7', holiday_only INTEGER DEFAULT 0);
CREATE TABLE IF NOT EXISTS calendar_exceptions(
  date TEXT PRIMARY KEY,
  mode TEXT NOT NULL,
  note TEXT);
CREATE TABLE IF NOT EXISTS rule_templates(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT UNIQUE NOT NULL,
  config_json TEXT NOT NULL,
  created_ts TEXT,
  updated_ts TEXT);
CREATE TABLE IF NOT EXISTS events(
  id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT, camera_id INTEGER, type TEXT,
  level INTEGER, track_id INTEGER, detail TEXT, snapshot TEXT, video TEXT,
  status TEXT DEFAULT 'new', handled_by TEXT, handled_ts TEXT,
  ai_review TEXT, ai_review_ts TEXT, ai_review_model TEXT);
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


def _mig_005(c: sqlite3.Connection):
    """v5：folder_images（任务121 图片文件夹自动识别）。

    每张被识别的图片一行：识别结果 + 关键/违规标记 + 证据副本；唯一键保证重启/重扫不重复。
    原始截图只读，本表不持有也不删除源文件。
    """
    c.execute("""CREATE TABLE IF NOT EXISTS folder_images(
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      camera_id INTEGER NOT NULL,
      path TEXT NOT NULL,
      name TEXT,
      mtime_ns INTEGER NOT NULL DEFAULT 0,
      size INTEGER NOT NULL DEFAULT 0,
      ts TEXT NOT NULL,
      processed_ts TEXT NOT NULL,
      width INTEGER,
      height INTEGER,
      persons INTEGER DEFAULT 0,
      in_zone INTEGER DEFAULT 0,
      ppe_no INTEGER DEFAULT 0,
      ppe_wearing INTEGER DEFAULT 0,
      zones TEXT,
      violations TEXT,
      key INTEGER DEFAULT 0,
      violation INTEGER DEFAULT 0,
      event_id INTEGER,
      snapshot TEXT,
      error TEXT)""")
    c.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_folder_images_uniq "
              "ON folder_images(camera_id, path, mtime_ns, size)")
    c.execute("CREATE INDEX IF NOT EXISTS idx_folder_images_ts "
              "ON folder_images(camera_id, ts)")
    c.execute("CREATE INDEX IF NOT EXISTS idx_folder_images_processed "
              "ON folder_images(processed_ts)")
    c.execute("CREATE INDEX IF NOT EXISTS idx_folder_images_key "
              "ON folder_images(key, violation, ts)")


def _mig_006(c: sqlite3.Connection):
    """v6：规则日历条件（任务112）。

    rules 加星期集合（ISO 1=周一…7=周日，缺省全星期）与"仅假期"标记；
    calendar_exceptions 为日期例外表（holiday=该日按假期 / workday=该日按工作日，
    覆盖周末判断）。旧库 ALTER 加列带默认值，旧行自动获得"全星期/非假期专用"=行为不变。
    """
    cols = {r[1] for r in c.execute("PRAGMA table_info(rules)")}
    if "days_of_week" not in cols:
        c.execute("ALTER TABLE rules ADD COLUMN days_of_week TEXT DEFAULT '1,2,3,4,5,6,7'")
    if "holiday_only" not in cols:
        c.execute("ALTER TABLE rules ADD COLUMN holiday_only INTEGER DEFAULT 0")
    c.execute("""CREATE TABLE IF NOT EXISTS calendar_exceptions(
      date TEXT PRIMARY KEY,
      mode TEXT NOT NULL,
      note TEXT)""")


def _mig_007(c: sqlite3.Connection):
    """v7：摄像头分组（任务111）。cameras 加 group_name（空=未分组），旧行为不变。"""
    cols = {r[1] for r in c.execute("PRAGMA table_info(cameras)")}
    if "group_name" not in cols:
        c.execute("ALTER TABLE cameras ADD COLUMN group_name TEXT DEFAULT ''")


def _mig_008(c: sqlite3.Connection):
    """v8：规则模板库（任务116）。只存规则参数，不绑摄像头/区域。"""
    c.execute("""CREATE TABLE IF NOT EXISTS rule_templates(
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      name TEXT UNIQUE NOT NULL,
      config_json TEXT NOT NULL,
      created_ts TEXT,
      updated_ts TEXT)""")


def _mig_009(c: sqlite3.Connection):
    """v9：VLM 复核列（任务124）。events 加 ai_review(JSON)/ai_review_ts/ai_review_model，
    旧行 NULL=未复核，行为不变。列名为固定字面量（不拼标识符）。"""
    cols = {r[1] for r in c.execute("PRAGMA table_info(events)")}
    if "ai_review" not in cols:
        c.execute("ALTER TABLE events ADD COLUMN ai_review TEXT")
    if "ai_review_ts" not in cols:
        c.execute("ALTER TABLE events ADD COLUMN ai_review_ts TEXT")
    if "ai_review_model" not in cols:
        c.execute("ALTER TABLE events ADD COLUMN ai_review_model TEXT")


def _mig_010(c: sqlite3.Connection):
    """v10：应用配置（网页端告警渠道配置，任务 140）。

    通用 key/value 表，供运行期覆盖 .env 的非敏感/密钥配置；密钥只存本机 DB，
    API 永不回显明文。旧库无表时自动创建，行为不变。
    """
    c.execute("""CREATE TABLE IF NOT EXISTS app_settings(
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL,
      updated_ts TEXT NOT NULL)""")


def _mig_011(c: sqlite3.Connection):
    """v11（T-309）：events.snapshot 循环引用查询 + 投递表状态/渠道索引（只加索引）。"""
    c.execute("CREATE INDEX IF NOT EXISTS idx_events_snapshot ON events(snapshot)")
    c.execute("CREATE INDEX IF NOT EXISTS idx_deliveries_status ON alert_deliveries(status)")
    c.execute("CREATE INDEX IF NOT EXISTS idx_deliveries_channel_status "
              "ON alert_deliveries(channel, status)")


# 版本化迁移（任务69）：只做加列/加表/索引，不做破坏性变更；每个迁移幂等、在事务内执行
MIGRATIONS: list[tuple[int, str, object]] = [
    (1, "events 增加 video/status/handled_by/handled_ts", _mig_001),
    (2, "新增 alert_silences 表（任务81 告警静默）", _mig_002),
    (3, "events 增加 escalated_ts（任务82 未处置升级提醒）", _mig_003),
    (4, "新增 audit_log 表（任务87 操作审计）", _mig_004),
    (5, "新增 folder_images 表（任务121 图片文件夹识别）", _mig_005),
    (6, "rules 星期/假期条件 + calendar_exceptions 日期例外表（任务112）", _mig_006),
    (7, "cameras 增加 group_name（任务111 摄像头分组）", _mig_007),
    (8, "新增 rule_templates 规则模板表（任务116）", _mig_008),
    (9, "events 增加 VLM 复核列 ai_review/ai_review_ts/ai_review_model（任务124）", _mig_009),
    (10, "新增 app_settings 表（网页端告警渠道配置，任务140）", _mig_010),
    (11, "查询索引：events.snapshot / alert_deliveries(status, channel)（T-309/PF-09）", _mig_011),
]
SCHEMA_VERSION_LATEST = max(v for v, _d, _f in MIGRATIONS)


class Store:
    def __init__(self, path=None):
        self.path = str(path or settings.db_path)
        self._lock = threading.Lock()
        # T-302（B-18/PF-11）：线程本地连接（每线程一条，复用连接 + PRAGMA 一次）；
        # T-303：配置版本号（zones/rules/calendar/camera 写入即 bump，供 ConfigCache 失效）
        self._local = threading.local()
        self._config_version: int = 0
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

    @contextlib.contextmanager
    def _conn(self):
        """T-302：线程本地连接；with 提交/回滚（语义不变），连接随线程复用而非每次新建。
        check_same_thread 默认 True 与线程本地一致；异常回溯不再延迟关闭句柄。"""
        c = getattr(self._local, "conn", None)
        if c is None:
            c = connect(self.path)
            self._local.conn = c
        try:
            yield c
            c.commit()
        except Exception:
            c.rollback()
            raise

    def config_version(self) -> int:
        return self._config_version

    def _bump(self) -> None:
        """T-303：配置写入后递增版本（ConfigCache 据此失效）。"""
        self._config_version += 1

    # ---- cameras ----
    def cameras(self):
        with self._conn() as c:
            return [dict(r) for r in c.execute("SELECT * FROM cameras ORDER BY id")]

    def camera(self, cid):
        with self._conn() as c:
            r = c.execute("SELECT * FROM cameras WHERE id=?", (cid,)).fetchone()
            return dict(r) if r else None

    def add_camera(self, name, source, type_, group_name=""):
        with self._lock, self._conn() as c:
            cur = c.execute("INSERT INTO cameras(name,source,type,group_name) VALUES(?,?,?,?)",
                            (name, source, type_, group_name or ""))
            cid = cur.lastrowid
        self._bump()
        return cid

    # ---- 摄像头分组（任务111）----
    def set_camera_group(self, cid, name: str) -> bool:
        with self._lock, self._conn() as c:
            ok = c.execute("UPDATE cameras SET group_name=? WHERE id=?",
                           (name or "", cid)).rowcount > 0
        if ok:
            self._bump()
        return ok

    def camera_groups(self) -> list:
        """分组聚合：[{name, count, running}]；group_name='' 记作"未分组"在 API 层转换。"""
        with self._conn() as c:
            return [dict(r) for r in c.execute(
                "SELECT group_name name, COUNT(*) count, "
                "SUM(CASE WHEN status='running' THEN 1 ELSE 0 END) running "
                "FROM cameras GROUP BY group_name ORDER BY name")]

    def delete_camera(self, cid):
        with self._lock, self._conn() as c:
            c.execute("DELETE FROM cameras WHERE id=?", (cid,))
            c.execute("DELETE FROM zones WHERE camera_id=?", (cid,))
            c.execute("DELETE FROM rules WHERE camera_id=?", (cid,))
        self._bump()

    def clear_config(self):
        """清空 cameras/zones/rules（任务79 replace 导入用）；events 等用户数据不动。"""
        with self._lock, self._conn() as c:
            c.execute("DELETE FROM cameras")
            c.execute("DELETE FROM zones")
            c.execute("DELETE FROM rules")
        self._bump()

    # ---- 应用运行期配置（任务140：网页端告警渠道配置）----
    def app_settings(self, prefix: str = "") -> dict[str, str]:
        with self._conn() as c:
            rows = c.execute("SELECT key,value FROM app_settings").fetchall()
        return {r["key"]: r["value"] for r in rows if r["key"].startswith(prefix)}

    def set_app_settings(self, items: dict[str, str]) -> None:
        if not items:
            return
        now = datetime.now().isoformat(timespec="seconds")
        with self._lock, self._conn() as c:
            c.executemany(
                "INSERT INTO app_settings(key,value,updated_ts) VALUES(?,?,?) "
                "ON CONFLICT(key) DO UPDATE SET value=excluded.value, "
                "updated_ts=excluded.updated_ts",
                [(k, v, now) for k, v in items.items()])

    def delete_app_settings(self, keys: list[str]) -> None:
        if not keys:
            return
        with self._lock, self._conn() as c:
            c.executemany("DELETE FROM app_settings WHERE key=?", [(k,) for k in keys])

    def set_camera_status(self, cid, status):
        with self._lock, self._conn() as c:
            c.execute("UPDATE cameras SET status=? WHERE id=?", (status, cid))

    # ---- 图片文件夹识别（任务121）----
    @staticmethod
    def _folder_row(row) -> dict:
        d = dict(row)
        for k in ("zones", "violations"):
            try:
                d[k] = json.loads(d[k] or "[]")
            except (TypeError, ValueError):
                d[k] = []
        for k in ("key", "violation"):
            d[k] = bool(d.get(k))
        return d

    def folder_image_done(self, camera_id: int, path: str, mtime_ns: int, size: int) -> bool:
        """该文件（路径+大小+修改时间）是否已识别过——重启/重扫去重依据。"""
        with self._conn() as c:
            r = c.execute(
                "SELECT 1 FROM folder_images "
                "WHERE camera_id=? AND path=? AND mtime_ns=? AND size=? LIMIT 1",
                (camera_id, str(path), int(mtime_ns), int(size))).fetchone()
            return r is not None

    def add_folder_image(self, row: dict) -> int:
        """写入一张图片的识别结果；唯一键冲突时返回既有行 id（幂等）。"""
        with self._lock, self._conn() as c:
            cur = c.execute(
                "INSERT OR IGNORE INTO folder_images("
                "camera_id,path,name,mtime_ns,size,ts,processed_ts,width,height,"
                "persons,in_zone,ppe_no,ppe_wearing,zones,violations,key,violation,snapshot,error) "
                "VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                (row["camera_id"], str(row["path"]), row.get("name"), int(row["mtime_ns"]),
                 int(row["size"]), row["ts"], row["processed_ts"], row.get("width"),
                 row.get("height"), int(row.get("persons", 0)), int(row.get("in_zone", 0)),
                 int(row.get("ppe_no", 0)), int(row.get("ppe_wearing", 0)),
                 json.dumps(row.get("zones") or [], ensure_ascii=False),
                 json.dumps(row.get("violations") or [], ensure_ascii=False),
                 1 if row.get("key") else 0, 1 if row.get("violation") else 0,
                 row.get("snapshot"), row.get("error")))
            if cur.rowcount:
                return int(cur.lastrowid)
            r = c.execute(
                "SELECT id FROM folder_images "
                "WHERE camera_id=? AND path=? AND mtime_ns=? AND size=?",
                (row["camera_id"], str(row["path"]), int(row["mtime_ns"]),
                 int(row["size"]))).fetchone()
            return int(r["id"]) if r else 0

    def set_folder_image_event(self, iid: int, event_id: int) -> None:
        with self._lock, self._conn() as c:
            c.execute("UPDATE folder_images SET event_id=? WHERE id=?", (event_id, iid))

    def folder_image(self, iid: int) -> dict | None:
        with self._conn() as c:
            r = c.execute("SELECT * FROM folder_images WHERE id=?", (iid,)).fetchone()
            return self._folder_row(r) if r else None

    # 过滤条件全部走参数占位符（含"未启用"分支），不拼用户输入
    @staticmethod
    def _folder_args(camera_id, key, violation, start_ts, end_ts, error_only) -> list:
        k = None if key is None else (1 if key else 0)
        v = None if violation is None else (1 if violation else 0)
        return [camera_id, camera_id, k, k, v, v, start_ts, start_ts,
                end_ts, end_ts, 1 if error_only else 0]

    def folder_images(self, camera_id=None, key=None, violation=None, start_ts=None,
                      end_ts=None, error_only=False, limit=60, offset=0) -> list[dict]:
        args = self._folder_args(camera_id, key, violation, start_ts, end_ts, error_only)
        args.extend([max(1, min(int(limit), 500)), max(0, int(offset))])
        with self._conn() as c:
            return [self._folder_row(r) for r in c.execute(
                "SELECT * FROM folder_images "
                "WHERE (? IS NULL OR camera_id=?) AND (? IS NULL OR key=?) "
                "AND (? IS NULL OR violation=?) AND (? IS NULL OR ts >= ?) "
                "AND (? IS NULL OR ts <= ?) AND (?=0 OR error IS NOT NULL) "
                "ORDER BY ts DESC, id DESC LIMIT ? OFFSET ?", args)]

    def count_folder_images(self, camera_id=None, key=None, violation=None, start_ts=None,
                            end_ts=None, error_only=False) -> int:
        args = self._folder_args(camera_id, key, violation, start_ts, end_ts, error_only)
        with self._conn() as c:
            return c.execute(
                "SELECT COUNT(*) n FROM folder_images "
                "WHERE (? IS NULL OR camera_id=?) AND (? IS NULL OR key=?) "
                "AND (? IS NULL OR violation=?) AND (? IS NULL OR ts >= ?) "
                "AND (? IS NULL OR ts <= ?) AND (?=0 OR error IS NOT NULL)",
                args).fetchone()["n"]

    def folder_image_summary(self, days: int = 7, camera_id: int | None = None) -> list[dict]:
        """按摄像头聚合近 N 天：总数/关键/违规/人数/在区/错误/最近时间。"""
        cutoff = (datetime.now() - timedelta(days=max(1, days))).isoformat(timespec="seconds")
        with self._conn() as c:
            rows = [dict(r) for r in c.execute(
                "SELECT camera_id, COUNT(*) AS total, "
                "COALESCE(SUM(key),0) AS key_n, COALESCE(SUM(violation),0) AS violation_n, "
                "COALESCE(SUM(persons),0) AS persons_sum, COALESCE(SUM(in_zone),0) AS in_zone_sum, "
                "COALESCE(SUM(CASE WHEN error IS NOT NULL THEN 1 ELSE 0 END),0) AS error_n, "
                "MAX(ts) AS last_image_ts, MAX(processed_ts) AS last_processed_ts "
                "FROM folder_images WHERE ts >= ? AND (? IS NULL OR camera_id=?) "
                "GROUP BY camera_id", (cutoff, camera_id, camera_id))]
        return rows

    def folder_images_older_than(self, cutoff: str, limit: int = 200) -> list[dict]:
        """保留清理：按处理时间取过期行（不按图片自身时间，避免导入旧档案立刻被清）。"""
        with self._conn() as c:
            return [dict(r) for r in c.execute(
                "SELECT id, camera_id, snapshot FROM folder_images WHERE processed_ts < ? "
                "ORDER BY id LIMIT ?", (cutoff, limit))]

    def delete_folder_image(self, iid: int) -> None:
        with self._lock, self._conn() as c:
            c.execute("DELETE FROM folder_images WHERE id=?", (iid,))

    def delete_folder_images_by_camera(self, camera_id: int) -> list[dict]:
        """删除某路图片文件夹的全部识别记录，返回被删行的证据路径（调用方清理文件）。"""
        with self._lock, self._conn() as c:
            rows = [dict(r) for r in c.execute(
                "SELECT id, snapshot FROM folder_images WHERE camera_id=?", (camera_id,))]
            c.execute("DELETE FROM folder_images WHERE camera_id=?", (camera_id,))
        return rows

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
            zid = cur.lastrowid
        self._bump()
        return zid

    def delete_zone(self, zid):
        with self._lock, self._conn() as c:
            c.execute("DELETE FROM zones WHERE id=?", (zid,))
        self._bump()

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
            r["holiday_only"] = bool(r.get("holiday_only"))  # 任务112：与 enabled 同口径转 bool
        return rows

    def add_rule(self, r: dict):
        with self._lock, self._conn() as c:
            cur = c.execute(
                "INSERT INTO rules(camera_id,zone_id,behavior,level,cooldown_s,time_start,time_end,"
                "threshold,enabled,days_of_week,holiday_only) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                (r["camera_id"], r.get("zone_id"), r["behavior"], r.get("level", 2),
                 r.get("cooldown_s", 60), r.get("time_start", "00:00"), r.get("time_end", "23:59"),
                 r.get("threshold", 0), 1 if r.get("enabled", True) else 0,
                 r.get("days_of_week", "1,2,3,4,5,6,7"), 1 if r.get("holiday_only") else 0),
            )
            rid = cur.lastrowid
        self._bump()
        return rid

    def delete_rule(self, rid):
        with self._lock, self._conn() as c:
            c.execute("DELETE FROM rules WHERE id=?", (rid,))
        self._bump()

    # ---- 日历例外（任务112）----
    def calendar_exceptions(self) -> list:
        with self._conn() as c:
            return [dict(r) for r in c.execute(
                "SELECT date, mode, note FROM calendar_exceptions ORDER BY date")]

    def set_calendar_exception(self, d: str, mode: str, note: str = "") -> bool:
        """写入/覆盖一个日期例外（date YYYY-MM-DD，mode holiday|workday）。返回是否覆盖了旧值。"""
        with self._lock, self._conn() as c:
            existed = c.execute(
                "SELECT 1 FROM calendar_exceptions WHERE date=?", (d,)).fetchone() is not None
            c.execute("INSERT OR REPLACE INTO calendar_exceptions(date, mode, note) VALUES(?,?,?)",
                      (d, mode, note))
        self._bump()  # T-303：日历写入使 is_holiday LRU 失效
        return existed

    def delete_calendar_exception(self, d: str) -> bool:
        with self._lock, self._conn() as c:
            ok = c.execute("DELETE FROM calendar_exceptions WHERE date=?", (d,)).rowcount > 0
        if ok:
            self._bump()
        return ok

    # ---- 规则模板（任务116）----
    def rule_templates(self) -> list:
        with self._conn() as c:
            return [dict(r) for r in c.execute(
                "SELECT id, name, config_json, created_ts, updated_ts "
                "FROM rule_templates ORDER BY id")]

    def get_rule_template(self, tid: int) -> dict | None:
        with self._conn() as c:
            row = c.execute("SELECT id, name, config_json, created_ts, updated_ts "
                            "FROM rule_templates WHERE id=?", (tid,)).fetchone()
            return dict(row) if row else None

    def add_rule_template(self, name: str, config_json: str) -> int | None:
        """同名返回 None（调用方转 409）。"""
        with self._lock, self._conn() as c:
            if c.execute("SELECT 1 FROM rule_templates WHERE name=?", (name,)).fetchone():
                return None
            now = datetime.now().isoformat(timespec="seconds")
            cur = c.execute(
                "INSERT INTO rule_templates(name, config_json, created_ts, updated_ts) "
                "VALUES(?,?,?,?)", (name, config_json, now, now))
            return cur.lastrowid

    def update_rule_template(self, tid: int, name: str, config_json: str) -> int:
        """0=不存在；-1=新名与其它模板冲突；1=成功。"""
        with self._lock, self._conn() as c:
            if c.execute("SELECT 1 FROM rule_templates WHERE id=?", (tid,)).fetchone() is None:
                return 0
            if c.execute("SELECT 1 FROM rule_templates WHERE name=? AND id<>?",
                         (name, tid)).fetchone():
                return -1
            c.execute("UPDATE rule_templates SET name=?, config_json=?, updated_ts=? WHERE id=?",
                      (name, config_json, datetime.now().isoformat(timespec="seconds"), tid))
            return 1

    def delete_rule_template(self, tid: int) -> bool:
        with self._lock, self._conn() as c:
            return c.execute("DELETE FROM rule_templates WHERE id=?", (tid,)).rowcount > 0

    def is_holiday(self, d) -> bool:
        """日期是否按假期处理：先查例外表（holiday=是 / workday=否），
        无记录按周末判断（默认周末算假期；LS_WEEKEND_IS_HOLIDAY=0 关闭）。"""
        from datetime import date as _date
        if isinstance(d, str):
            d = _date.fromisoformat(d)
        with self._conn() as c:
            row = c.execute("SELECT mode FROM calendar_exceptions WHERE date=?",
                            (d.isoformat(),)).fetchone()
        if row:
            return row["mode"] == "holiday"
        if not settings.weekend_is_holiday:
            return False
        return d.weekday() >= 5  # 5=周六 6=周日

    # ---- events ----
    def add_event(self, camera_id, type_, level, track_id, detail, snapshot=None,
                  ts: str | None = None) -> int:
        """ts 可注入（T-321 确定性模式：事件时间取视频帧时间，非墙钟）。"""
        with self._lock, self._conn() as c:
            cur = c.execute(
                "INSERT INTO events(ts,camera_id,type,level,track_id,detail,snapshot) VALUES(?,?,?,?,?,?,?)",
                (ts or datetime.now().isoformat(timespec="seconds"),
                 camera_id, type_, level, track_id, detail, snapshot),
            )
            eid = cur.lastrowid
        try:  # 任务124：VLM 误报预分类（默认 none=零开销；入队失败静默，绝不影响事件主流程）
            from .genai import maybe_enqueue
            maybe_enqueue(eid, snapshot, level)
        except Exception:  # noqa: BLE001
            pass
        return eid

    def set_ai_review(self, eid: int, review_json: str, model: str) -> bool:
        """写入 VLM 复核结果（任务124；只写标记列，绝不改 status）。"""
        with self._lock, self._conn() as c:
            return c.execute(
                "UPDATE events SET ai_review=?, ai_review_ts=?, ai_review_model=? WHERE id=?",
                (review_json, datetime.now().isoformat(timespec="seconds"), model, eid)
            ).rowcount > 0

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
        with self._conn() as c:  # B-18：原来直接 .execute() 不进 with，连接悬空
            rows = c.execute(sql, (now,) if active_only else ()).fetchall()
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
        cond = " AND ".join(where) if where else ""
        sql_count = "SELECT COUNT(*) FROM audit_log"
        sql_rows = "SELECT * FROM audit_log"
        if cond:
            where_sql = " WHERE " + cond  # 片段全部为固定字面量，值走参数绑定
            sql_count += where_sql
            sql_rows += where_sql
        sql_rows += " ORDER BY id DESC LIMIT ? OFFSET ?"
        with self._conn() as c:
            total = c.execute(sql_count, args).fetchone()[0]
            rows = c.execute(sql_rows,
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

    @staticmethod
    def _ai_clause(ai_verdict: str | None, where: list, args: list) -> None:
        """AI 复核过滤（任务132）：'none'=未复核（ai_review IS NULL）；
        其它 verdict 用 JSON1 json_extract（主流 SQLite 内建）。"""
        if not ai_verdict:
            return
        v = ai_verdict.strip()
        if v == "none":
            where.append("ai_review IS NULL")
        else:
            where.append("json_extract(ai_review,'$.verdict')=?")
            args.append(v)

    def events(self, limit=50, camera_id=None, level=None, status=None,
               offset=0, start_ts=None, end_ts=None, q: str | None = None,
               ai_verdict: str | None = None):
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
        self._ai_clause(ai_verdict, where, args)
        sql = "SELECT * FROM events"
        if where:
            sql += " WHERE " + " AND ".join(where)
        sql += " ORDER BY id DESC LIMIT ? OFFSET ?"
        args.extend([limit, max(0, offset)])
        with self._conn() as c:
            return [dict(r) for r in c.execute(sql, args)]

    def count_events(self, camera_id=None, level=None, status=None,
                     start_ts=None, end_ts=None, q: str | None = None,
                     ai_verdict: str | None = None) -> int:
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
        self._ai_clause(ai_verdict, where, args)
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

    def event_snapshot_referenced(self, path: str) -> bool:
        """该证据文件是否仍被事件引用（图片记录先过期时不能删，事件保留期内还要能看）。"""
        with self._conn() as c:
            r = c.execute("SELECT 1 FROM events WHERE snapshot=? LIMIT 1", (str(path),)).fetchone()
            return r is not None

    def referenced_media_paths(self) -> set[str]:
        """所有仍被事件/图片记录引用的媒体路径（任务107 孤儿扫描用，一次查完避免 N 次查询）。"""
        out: set[str] = set()
        with self._conn() as c:
            rows = c.execute(
                "SELECT snapshot FROM events WHERE snapshot IS NOT NULL "
                "UNION SELECT video FROM events WHERE video IS NOT NULL "
                "UNION SELECT snapshot FROM folder_images WHERE snapshot IS NOT NULL")
            for r in rows:
                p = r[0]
                if p:
                    out.add(os.path.normcase(os.path.abspath(str(p))))
        return out

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
