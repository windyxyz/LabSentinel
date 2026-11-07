"""单元测试：事件闭环（状态流/迁移/误报回流池）。运行: pytest tests/ -q"""
import os
import shutil
import sqlite3
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import api as api_mod  # noqa: E402
from app.store import EVENT_STATUSES, Store, _migrate  # noqa: E402
from app.main import app  # noqa: E402


def test_migrate_old_db_adds_status_columns(tmp_path):
    """旧库（无 status/handled_by/handled_ts 列）打开自动补列，幂等。"""
    db = tmp_path / "old.db"
    with sqlite3.connect(db) as c:
        c.execute("""CREATE TABLE events(
          id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT, camera_id INTEGER, type TEXT,
          level INTEGER, track_id INTEGER, detail TEXT, snapshot TEXT, video TEXT)""")
        c.execute("INSERT INTO events(ts,camera_id,type,level,track_id,detail,snapshot)"
                  " VALUES('2026-01-01T08:00:00',1,'enter',2,1,'旧事件',NULL)")
    s = Store(path=db)
    row = s.events(limit=1)[0]
    assert row["status"] == "new"  # 迁移后旧事件默认 new
    assert row["handled_by"] is None and row["handled_ts"] is None
    with sqlite3.connect(db) as c:
        _migrate(c)  # 幂等
        _migrate(c)


def test_set_event_status_flow(tmp_path):
    s = Store(path=tmp_path / "t.db")
    eid = s.add_event(1, "enter", 2, 7, "进入安全区")
    assert s.events(limit=5)[0]["status"] == "new"
    assert s.set_event_status(eid, "acked") is True
    row = s.events(limit=5)[0]
    assert row["status"] == "acked" and row["handled_by"] == "admin"
    assert row["handled_ts"] is not None
    # 不存在的事件返回 False
    assert s.set_event_status(9999, "ignored") is False
    assert EVENT_STATUSES == ("new", "acked", "false_positive", "ignored")


def test_events_status_filter(tmp_path):
    s = Store(path=tmp_path / "t.db")
    e1 = s.add_event(1, "enter", 2, 1, "a")
    e2 = s.add_event(1, "exit", 1, 2, "b")
    s.set_event_status(e2, "false_positive")
    assert [r["id"] for r in s.events(limit=50)] == [e2, e1]  # 全部
    assert [r["id"] for r in s.events(limit=50, status="new")] == [e1]
    assert [r["id"] for r in s.events(limit=50, status="false_positive")] == [e2]
    assert [r["id"] for r in s.events(limit=50, status="unhandled")] == [e1]


@pytest.fixture()
def client(tmp_path, monkeypatch):
    s = Store(path=tmp_path / "t.db")
    monkeypatch.setattr(api_mod, "store", s)
    # 截图目录与误报池都指到 tmp，避免污染真实 data/
    from app.config import settings
    snap_dir = tmp_path / "snaps"
    snap_dir.mkdir()
    monkeypatch.setattr(settings, "snapshot_dir", snap_dir)
    monkeypatch.setattr(settings, "annotation_pool_dir", tmp_path / "pool")
    c = TestClient(app)
    return c, s, tmp_path


def _add_event_with_snapshot(s, snap_dir):
    eid = s.add_event(1, "enter_no_ppe", 3, 5, "进入未穿")
    p = snap_dir / f"ev_{eid}.jpg"
    p.write_bytes(b"\xff\xd8fakejpeg")
    with s._lock, s._conn() as c:
        c.execute("UPDATE events SET snapshot=? WHERE id=?", (str(p), eid))
    return eid, p


def test_status_api_and_pool(client):
    c, s, tmp_path = client
    eid, snap = _add_event_with_snapshot(s, tmp_path / "snaps")
    # 非法 status → 422（T-505：EventStatusBody Literal）
    r = c.post(f"/api/events/{eid}/status", json={"status": "whatever"})
    assert r.status_code == 422
    r = c.post(f"/api/events/{eid}/status", json={"status": "new"})
    assert r.status_code == 422  # new 不是处置动作
    # 不存在 → 404
    r = c.post("/api/events/9999/status", json={"status": "acked"})
    assert r.status_code == 404
    # 正常确认
    r = c.post(f"/api/events/{eid}/status", json={"status": "acked"})
    assert r.status_code == 200 and r.json()["status"] == "acked"
    # 误报 → 截图复制进标注池（原截图保留）
    r = c.post(f"/api/events/{eid}/status", json={"status": "false_positive"})
    assert r.status_code == 200
    pool = tmp_path / "pool"
    files = list(pool.glob("*.jpg"))
    assert len(files) == 1 and f"ev{eid}" in files[0].name
    assert snap.exists()  # 原件未动
    # events 列表带 status 字段 + status 过滤
    rows = c.get("/api/events", params={"status": "false_positive"}).json()
    assert len(rows) == 1 and rows[0]["id"] == eid and rows[0]["status"] == "false_positive"
    # stats 按状态过滤
    daily = c.get("/api/stats/daily", params={"days": 7, "status": "false_positive"}).json()
    assert sum(d["total"] for d in daily["days"]) == 1
    # system 带未处置计数
    sysr = c.get("/api/system").json()
    assert "unhandled_today" in sysr
