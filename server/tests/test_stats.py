"""单元测试：统计聚合（按天/按小时/摄像头筛选）。运行: pytest tests/ -q"""
import os
import sqlite3
import sys
from datetime import date, timedelta

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app.stats import daily, hourly  # noqa: E402
from app.store import Store  # noqa: E402


def _insert(s, ts, level, type_, camera_id=1):
    """直接写指定 ts 的事件行（绕过 add_event 的当前时间）。"""
    with sqlite3.connect(s.path) as c:
        c.execute("INSERT INTO events(ts,camera_id,type,level,track_id,detail,snapshot) VALUES(?,?,?,?,?,?,?)",
                  (ts, camera_id, type_, level, 0, "", None))


def test_daily_empty(tmp_path):
    s = Store(path=tmp_path / "t.db")
    out = daily(s, days=7)
    assert len(out["days"]) == 7
    for row in out["days"]:
        assert row["total"] == 0 and row["l1"] == 0 and row["l2"] == 0 and row["l3"] == 0


def test_daily_multi_day_levels(tmp_path):
    s = Store(path=tmp_path / "t.db")
    today = date.today().isoformat()
    yst = (date.today() - timedelta(days=1)).isoformat()
    _insert(s, f"{today}T09:10:00", 2, "enter")
    _insert(s, f"{today}T09:15:00", 3, "enter_no_ppe")
    _insert(s, f"{today}T10:00:00", 1, "exit")
    _insert(s, f"{yst}T08:00:00", 2, "enter", camera_id=2)
    out = daily(s, days=7)
    rows = {r["date"]: r for r in out["days"]}
    assert rows[today]["total"] == 3 and rows[today]["l1"] == 1
    assert rows[today]["l2"] == 1 and rows[today]["l3"] == 1
    assert rows[today]["enter"] == 1 and rows[today]["enter_no_ppe"] == 1 and rows[today]["exit"] == 1
    assert rows[yst]["total"] == 1 and rows[yst]["enter"] == 1
    # 2 天窗口：只剩今天和昨天
    assert len(daily(s, days=2)["days"]) == 2


def test_hourly_and_camera_filter(tmp_path):
    s = Store(path=tmp_path / "t.db")
    d = date.today().isoformat()
    _insert(s, f"{d}T08:05:00", 2, "enter")
    _insert(s, f"{d}T08:59:00", 2, "enter")
    _insert(s, f"{d}T17:30:00", 3, "enter_no_ppe", camera_id=2)
    out = hourly(s, d)
    assert [h["hour"] for h in out["hours"]] == list(range(24))
    assert out["hours"][8]["total"] == 2 and out["hours"][8]["enter"] == 2
    assert out["hours"][17]["total"] == 1 and out["hours"][17]["l3"] == 1
    assert out["hours"][9]["total"] == 0
    # 摄像头筛选：只统计 camera 1
    out1 = hourly(s, d, camera_id=1)
    assert out1["hours"][17]["total"] == 0 and out1["hours"][8]["total"] == 2
    # 非法日期
    try:
        hourly(s, "2026/10/01")
        assert False, "应当抛 ValueError"
    except ValueError:
        pass
