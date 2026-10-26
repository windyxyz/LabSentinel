"""单元测试：任务21 报表增强（周聚合边界/汇总一致性/summary 与 daily 吻合）。运行: pytest tests/ -q"""
import os
import sys
from datetime import date, timedelta

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import api as api_mod  # noqa: E402
from app.main import app  # noqa: E402
from app.stats import summary, weekly  # noqa: E402
from app.store import Store  # noqa: E402


@pytest.fixture()
def s(tmp_path):
    return Store(path=tmp_path / "t.db")


def _insert(s, ts, level, type_, camera_id=1):
    with s._lock, s._conn() as c:
        c.execute("INSERT INTO events(ts,camera_id,type,level,track_id,detail,snapshot,video,status)"
                  " VALUES(?,?,?,?,?,?,NULL,NULL,'new')", (ts, camera_id, type_, level, 1, ""))


def test_weekly_monday_boundary(s):
    """周聚合按周一为起点：同一周的周中事件归到同一 week_start。"""
    today = date.today()
    monday = today - timedelta(days=today.weekday())
    # 本周的周一与周六各一条 → 同桶
    _insert(s, f"{monday.isoformat()}T09:00:00", 2, "enter")
    _insert(s, f"{(monday + timedelta(days=6)).isoformat()}T10:00:00", 2, "enter")
    out = weekly(s, weeks=2)
    assert len(out["weeks"]) == 2
    this_week = [w for w in out["weeks"] if w["week_start"] == monday.isoformat()][0]
    assert this_week["total"] == 2 and this_week["enter"] == 2
    # 上周桶为空但存在（补零）
    last_week = [w for w in out["weeks"] if w["week_start"] == (monday - timedelta(weeks=1)).isoformat()][0]
    assert last_week["total"] == 0


def test_summary_consistency_with_daily(s):
    """summary 与 daily（同窗口）的各级别/总数必须吻合。"""
    d30 = date.today() - timedelta(days=10)
    _insert(s, f"{d30.isoformat()}T08:00:00", 1, "exit")
    _insert(s, f"{d30.isoformat()}T09:00:00", 2, "enter")
    _insert(s, f"{d30.isoformat()}T10:00:00", 3, "enter_no_ppe")
    _insert(s, f"{date.today().isoformat()}T11:00:00", 2, "enter", camera_id=2)
    sm = summary(s, days=15)
    dl = [r for r in daily_rows(s, 15)]
    assert sm["total"] == sum(r["total"] for r in dl) == 4
    assert sm["by_level"] == {1: 1, 2: 2, 3: 1}
    assert sm["by_type"]["enter"] == 2 and sm["by_type"]["exit"] == 1
    # Top 事件日：10 天前那天 3 条
    assert sm["top_days"][0] == {"date": d30.isoformat(), "total": 3}


def daily_rows(s, days):
    from app.stats import daily
    return daily(s, days=days)["days"]


def test_summary_camera_filter(s):
    _insert(s, f"{date.today().isoformat()}T08:00:00", 2, "enter", camera_id=1)
    _insert(s, f"{date.today().isoformat()}T09:00:00", 2, "enter", camera_id=2)
    sm1 = summary(s, days=7, camera_id=1)
    assert sm1["total"] == 1
    sm_all = summary(s, days=7)
    assert sm_all["total"] == 2


def test_stats_api_weekly_summary(client):
    r = client.get("/api/stats/weekly", params={"weeks": 4})
    assert r.status_code == 200
    body = r.json()
    assert len(body["weeks"]) == 4 and all("week_start" in w for w in body["weeks"])
    r = client.get("/api/stats/summary", params={"days": 30})
    assert r.status_code == 200
    assert {"total", "by_level", "by_type", "top_days"} <= set(r.json())
    # daily 自定义范围参数
    r = client.get("/api/stats/daily", params={
        "start_ts": "2026-01-01", "end_ts": "2026-01-03"})
    assert r.status_code == 200
    assert [d["date"] for d in r.json()["days"]] == ["2026-01-01", "2026-01-02", "2026-01-03"]


@pytest.fixture()
def client(tmp_path, monkeypatch):
    s = Store(path=tmp_path / "t.db")
    monkeypatch.setattr(api_mod, "store", s)
    return TestClient(app)
