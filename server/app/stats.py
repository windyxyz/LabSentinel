"""统计聚合查询：按天/周/小时/汇总（报表页数据源，任务21 增强）。

时间解析用 ts 字段的 ISO 前缀：substr(ts,1,10)=日期、substr(ts,12,2)=小时。
"""
import json
import sqlite3  # noqa: F401  （保留给类型标注/兼容）
from datetime import date, timedelta

from .store import connect

_EMPTY = {"total": 0, "l1": 0, "l2": 0, "l3": 0}


def _conn(store):
    return connect(store.path)


def _agg(rows) -> dict:
    """[(分组键, level, type, n)] → 键 -> {total,l1,l2,l3,<type>:n} 聚合。"""
    out: dict = {}
    for key, level, type_, n in rows:
        row = out.setdefault(key, dict(_EMPTY))
        row["total"] += n
        lv = f"l{int(level)}"
        if lv in row:
            row[lv] += n
        row[type_] = row.get(type_, 0) + n
    return out


def _base_where(camera_id=None, status=None, start_ts=None, end_ts=None):
    """公共过滤片段（含开头 AND）。"""
    sql, args = "", []
    if camera_id is not None:
        sql += " AND camera_id=?"
        args.append(camera_id)
    if status is not None:
        sql += " AND status=?"
        args.append("new" if status == "unhandled" else status)
    if start_ts:
        sql += " AND ts >= ?"
        args.append(start_ts)
    if end_ts:
        sql += " AND ts <= ?"
        args.append(end_ts)
    return sql, args


def daily(store, days: int = 7, camera_id: int | None = None, status: str | None = None,
          start_ts: str | None = None, end_ts: str | None = None) -> dict:
    """最近 N 天（含今天）逐日统计；给了 start_ts/end_ts 则按自定义窗口展开
    （超 400 天退化为只返回有数据的天，防展开爆炸）。缺数据的天补零。"""
    if start_ts and end_ts:
        where, wargs = _base_where(camera_id, status, start_ts, end_ts)
        sql = ("SELECT substr(ts,1,10) d, level, type, COUNT(*) n FROM events WHERE 1=1" +
               where + " GROUP BY d, level, type")
        with _conn(store) as c:
            agg = _agg(c.execute(sql, wargs).fetchall())
        d0 = date.fromisoformat(start_ts[:10])
        d1 = date.fromisoformat(end_ts[:10])
        if (d1 - d0).days > 400:
            out = [{"date": k, **v} for k, v in sorted(agg.items())]
        else:
            out = [{"date": (d0 + timedelta(days=i)).isoformat(),
                    **agg.get((d0 + timedelta(days=i)).isoformat(), dict(_EMPTY))}
                   for i in range((d1 - d0).days + 1)]
        return {"days": out}
    days = max(1, min(int(days), 90))
    start = (date.today() - timedelta(days=days - 1)).isoformat()
    where, wargs = _base_where(camera_id, status)
    sql = ("SELECT substr(ts,1,10) d, level, type, COUNT(*) n FROM events WHERE ts >= ?" +
           where + " GROUP BY d, level, type")
    with _conn(store) as c:
        agg = _agg(c.execute(sql, [start] + wargs).fetchall())
    out = []
    for i in range(days):
        d = (date.today() - timedelta(days=days - 1 - i)).isoformat()
        out.append({"date": d, **agg.get(d, dict(_EMPTY))})
    return {"days": out}


def weekly(store, weeks: int = 4, camera_id: int | None = None, status: str | None = None) -> dict:
    """按 ISO 周聚合（周一为起点），最近 N 周（含本周），缺数据周补零。
    SQLite 技巧：date(d,'-6 days','weekday 1') = d 所在周的周一（跨年正确）。"""
    weeks = max(1, min(int(weeks), 26))
    today = date.today()
    first_monday = today - timedelta(days=today.weekday()) - timedelta(weeks=weeks - 1)
    where, wargs = _base_where(camera_id, status, first_monday.isoformat(), None)
    sql = ("SELECT date(substr(ts,1,10),'-6 days','weekday 1') wk, level, type, COUNT(*) n"
           " FROM events WHERE ts >= ?" + where + " GROUP BY wk, level, type")
    with _conn(store) as c:
        agg = _agg(c.execute(sql, [first_monday.isoformat()] + wargs).fetchall())
    out = []
    for i in range(weeks):
        wk = (first_monday + timedelta(weeks=i)).isoformat()
        out.append({"week_start": wk, **agg.get(wk, dict(_EMPTY))})
    return {"weeks": out}


def summary(store, days: int = 30, camera_id: int | None = None, status: str | None = None) -> dict:
    """汇总卡片：总数 / 分级别 / 分类型 / Top 事件日（最多 5 天）。"""
    days = max(1, min(int(days), 90))
    start = (date.today() - timedelta(days=days - 1)).isoformat()
    where, wargs = _base_where(camera_id, status, start, None)
    sql = ("SELECT substr(ts,1,10) d, level, type, COUNT(*) n FROM events WHERE ts >= ?" +
           where + " GROUP BY d, level, type")
    with _conn(store) as c:
        rows = c.execute(sql, [start] + wargs).fetchall()
    by_level = {1: 0, 2: 0, 3: 0}
    by_type: dict = {}
    per_day: dict = {}
    for d, level, type_, n in rows:
        lv = int(level)
        by_level[lv] = by_level.get(lv, 0) + n
        by_type[type_] = by_type.get(type_, 0) + n
        per_day[d] = per_day.get(d, 0) + n
    top_days = [{"date": d, "total": n} for d, n in
                sorted(per_day.items(), key=lambda kv: -kv[1])[:5]]
    return {"days": days, "total": sum(by_level.values()), "by_level": by_level,
            "by_type": by_type, "top_days": top_days}


def hourly(store, date_str: str, camera_id: int | None = None) -> dict:
    """某日 0-23 时逐小时统计，缺数据的小时补零。date_str 格式 YYYY-MM-DD。"""
    try:
        date.fromisoformat(date_str)
    except ValueError as e:
        raise ValueError("date 必须是 YYYY-MM-DD 格式") from e
    sql = "SELECT substr(ts,12,2) h, level, type, COUNT(*) n FROM events WHERE substr(ts,1,10)=?"
    args: list = [date_str]
    if camera_id is not None:
        sql += " AND camera_id=?"
        args.append(camera_id)
    sql += " GROUP BY h, level, type"
    with _conn(store) as c:
        agg = _agg(c.execute(sql, args).fetchall())
    out = []
    for h in range(24):
        key = f"{h:02d}"
        out.append({"hour": h, **agg.get(key, dict(_EMPTY))})
    return {"date": date_str, "hours": out}


def quality(store, days: int = 7, camera_id: int | None = None) -> dict:
    """处置质量统计（任务102）：误报率/未处置/L3，按状态/级别/类型/摄像头/逐日分组。

    口径（务必与文档一致）：false_positive_rate = false_positive / (acked + false_positive + ignored)
    ——分母是**已处置**事件（acked/误报/忽略），不含未处置(new)；无已处置事件时为 0。
    它是"已处置里误报占比"，不是"每天误报次数"（每天误报次数看 daily[].false_positive）。
    """
    from datetime import datetime
    now = datetime.now()
    # T-203（B-06）：库内 ts 是 ISO 'T' 分隔；"%Y-%m-%d %H:%M:%S"（空格）与它做字符串
    # 比较时 'T'(0x54)>' '(0x20) → 当天事件全部被 end_ts 排除。统一 isoformat。
    start_ts = (now - timedelta(days=days)).isoformat(timespec="seconds")
    end_ts = now.isoformat(timespec="seconds")
    where, args = ["ts >= ?", "ts <= ?"], [start_ts, end_ts]
    if camera_id is not None:
        where.append("camera_id=?")
        args.append(camera_id)
    w = " WHERE " + " AND ".join(where)
    with _conn(store) as c:
        total = c.execute("SELECT COUNT(*) n FROM events" + w, args).fetchone()["n"]
        by_status = {r["status"]: r["n"] for r in c.execute(
            "SELECT status, COUNT(*) n FROM events" + w + " GROUP BY status", args)}
        by_level = {str(r["level"]): r["n"] for r in c.execute(
            "SELECT level, COUNT(*) n FROM events" + w + " GROUP BY level", args)}
        by_type = {r["type"]: r["n"] for r in c.execute(
            "SELECT type, COUNT(*) n FROM events" + w + " GROUP BY type", args)}
        by_camera = [
            {"camera_id": r["camera_id"], "name": r["name"] or "", "total": r["total"],
             "false_positive": r["fp"],
             "rate": round(r["fp"] / r["total"], 3) if r["total"] else 0.0}
            for r in c.execute(
                "SELECT e.camera_id camera_id, c.name name, COUNT(*) total, "
                "SUM(CASE WHEN e.status='false_positive' THEN 1 ELSE 0 END) fp "
                "FROM events e LEFT JOIN cameras c ON c.id = e.camera_id" + w +
                " GROUP BY e.camera_id ORDER BY total DESC", args)]
        daily = [{"date": r["d"], "total": r["total"], "false_positive": r["fp"]}
                 for r in c.execute(
                     "SELECT substr(ts,1,10) d, COUNT(*) total, "
                     "SUM(CASE WHEN status='false_positive' THEN 1 ELSE 0 END) fp "
                     "FROM events" + w + " GROUP BY d ORDER BY d", args)]
    by_ai = {"likely_true": 0, "likely_false": 0, "uncertain": 0}
    for r in c.execute(
            "SELECT ai_review FROM events" + w + " AND ai_review IS NOT NULL", args):
        try:
            v = json.loads(r["ai_review"]).get("verdict")
        except ValueError:
            continue
        if v in by_ai:
            by_ai[v] += 1
    fp = by_status.get("false_positive", 0)
    handled = by_status.get("acked", 0) + fp + by_status.get("ignored", 0)
    return {
        "days": days, "start_ts": start_ts, "end_ts": end_ts,
        "total": total, "by_status": by_status, "by_level": by_level, "by_type": by_type,
        "by_camera": by_camera,
        "false_positive_rate": round(fp / handled, 3) if handled else 0.0,
        "unhandled": by_status.get("new", 0),
        "l3_total": by_level.get("3", 0),
        "by_ai_verdict": by_ai,  # 任务124：VLM 预分类计数（仅供参考，误报率仍以人工处置为准）
        "daily": daily,
    }


def timeline(store, days: int = 7, camera_id: int | None = None,
             level: int | None = None) -> dict:
    """事件时间线（任务136）：按 日期+小时 分桶，Frigate review 式热力条数据源。

    days 1-30；空数据补零；SQL group by substr(ts,1,10), substr(ts,12,2)——不加载事件行。
    """
    from datetime import datetime, timedelta as _td
    now = datetime.now()
    d0 = (now - _td(days=days - 1)).date()
    # 从 d0 当天 00:00 起算，保证查询范围与返回的 days 个自然日完全一致
    start_ts = f"{d0.isoformat()} 00:00:00"
    where, args = ["ts >= ?"], [start_ts]
    if camera_id is not None:
        where.append("camera_id=?")
        args.append(camera_id)
    if level is not None:
        where.append("level=?")
        args.append(level)
    sql = ("SELECT substr(ts,1,10) d, substr(ts,12,2) h, COUNT(*) n, "
           "SUM(CASE WHEN level>=3 THEN 1 ELSE 0 END) l3 "
           "FROM events WHERE " + " AND ".join(where) + " GROUP BY d, h")
    with _conn(store) as c:
        agg = {(r["d"], int(r["h"])): (r["n"], r["l3"] or 0)
               for r in c.execute(sql, args)}
    out = []
    for i in range(days):
        d = (d0 + _td(days=i)).isoformat()
        hours = []
        day_total = 0
        for h in range(24):
            n, l3 = agg.get((d, h), (0, 0))
            day_total += n
            hours.append({"hour": h, "total": n, "l3": l3})
        out.append({"date": d, "total": day_total, "hours": hours})
    return {"days": days, "days_list": out}
