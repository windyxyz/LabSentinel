"""单元测试：任务136 事件时间线（聚合正确/补零/过滤/权限/与 count 对拍）。
运行: pytest tests/test_event_timeline.py -q"""
from datetime import datetime, timedelta

import pytest
from fastapi.testclient import TestClient

from app import api as api_mod
from app.main import app
from app.stats import timeline
from app.store import Store


def _add(st, cid, ts, level=2):
    with st._conn() as c:
        c.execute("INSERT INTO events(ts,camera_id,type,level,track_id,detail) "
                  "VALUES(?,?,?,?,0,'x')", (ts, cid, "enter", level))


def test_timeline_buckets_and_zero_fill(tmp_path):
    st = Store(tmp_path / "s.db")
    cid = st.add_camera("c", "D:/x.mp4", "file")
    now = datetime.now()
    # 今天 10 点 2 条（1 条 L3）、昨天 23 点 1 条
    _add(st, cid, now.strftime("%Y-%m-%d") + " 10:30:00", 3)
    _add(st, cid, now.strftime("%Y-%m-%d") + " 10:45:00", 2)
    _add(st, cid, (now - timedelta(days=1)).strftime("%Y-%m-%d") + " 23:15:00", 2)
    r = timeline(st, days=3)
    assert r["days"] == 3 and len(r["days_list"]) == 3
    today = [d for d in r["days_list"] if d["date"] == now.strftime("%Y-%m-%d")][0]
    h10 = today["hours"][10]
    assert h10["total"] == 2 and h10["l3"] == 1
    assert sum(h["total"] for h in today["hours"]) == 2
    yesterday = [d for d in r["days_list"] if d["date"] ==
                 (now - timedelta(days=1)).strftime("%Y-%m-%d")][0]
    assert yesterday["hours"][23]["total"] == 1
    # 补零：三天 72 小时格全在
    assert all(len(d["hours"]) == 24 for d in r["days_list"])
    empty_day = [d for d in r["days_list"]
                 if d["date"] == (now - timedelta(days=2)).strftime("%Y-%m-%d")][0]
    assert empty_day["total"] == 0 and all(h["total"] == 0 for h in empty_day["hours"])


def test_timeline_filters(tmp_path):
    st = Store(tmp_path / "s.db")
    c1 = st.add_camera("c1", "D:/a.mp4", "file")
    c2 = st.add_camera("c2", "D:/b.mp4", "file")
    now = datetime.now().strftime("%Y-%m-%d") + " 09:00:00"
    _add(st, c1, now, 3)
    _add(st, c2, now, 2)
    r = timeline(st, days=1, camera_id=c1)
    assert r["days_list"][0]["total"] == 1
    r3 = timeline(st, days=1, level=3)
    assert r3["days_list"][0]["total"] == 1
    # count 对拍
    assert st.count_events(camera_id=c1) == 1
    assert st.count_events(level=3) == 1


# ---- API ----

@pytest.fixture()
def client():
    with TestClient(app) as tc:
        yield tc


def test_api_timeline(client):
    st = Store(api_mod.settings.db_path)  # conftest 隔离库
    cid = st.add_camera("tl", "D:/x.mp4", "file")
    now = datetime.now().strftime("%Y-%m-%d") + " 08:00:00"
    _add(st, cid, now, 3)
    r = client.get("/api/events/timeline?days=2")
    assert r.status_code == 200
    body = r.json()
    today = [d for d in body["days_list"] if d["date"] == datetime.now().strftime("%Y-%m-%d")][0]
    assert today["hours"][8]["total"] == 1 and today["hours"][8]["l3"] == 1
    # count 对拍
    total_all = sum(d["total"] for d in body["days_list"])
    assert total_all == client.get("/api/events/count").json()["count"]
    # days 边界
    assert client.get("/api/events/timeline?days=0").status_code == 400
    assert client.get("/api/events/timeline?days=31").status_code == 400
    assert client.get("/api/events/timeline?days=30").status_code == 200
    # 鉴权
    monkey = pytest.MonkeyPatch()
    monkey.setattr(api_mod.settings, "admin_token", "sec")
    monkey.setattr(api_mod.settings, "auth_read_protected", True)
    assert client.get("/api/events/timeline").status_code == 401
    monkey.undo()
