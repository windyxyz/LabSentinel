"""单元测试：任务112 规则日期/星期/假期条件（迁移 v6 / 引擎日历过滤 / calendar CRUD / 鉴权）。
运行: pytest tests/test_rule_calendar.py -q"""
from datetime import date, datetime, timedelta

import pytest
from fastapi.testclient import TestClient

from app import api as api_mod
from app.main import app
from app.rules import RuleEngine
from app.store import Store


@pytest.fixture()
def st(tmp_path, monkeypatch):
    from app.config import settings
    monkeypatch.setattr(settings, "weekend_is_holiday", True)
    return Store(tmp_path / "s.db")


def _rule(store, cid, behavior="enter", **kw):
    rid = store.add_rule({"camera_id": cid, "zone_id": None, "behavior": behavior,
                          "level": 2, "cooldown_s": 0, "time_start": "00:00",
                          "time_end": "23:59", "threshold": 0, "enabled": True, **kw})
    return store.rules(cid)[[r["id"] for r in store.rules(cid)].index(rid)]


def _next_weekday(target: int) -> date:
    """下一个 ISO weekday==target 的日期（含今天）。"""
    d = date.today()
    while d.isoweekday() != target:
        d += timedelta(days=1)
    return d


def test_weekday_filter(st, tmp_path):
    cid = st.add_camera("c", "D:/x.mp4", "file")
    eng = RuleEngine(st)
    monday = _next_weekday(1)
    tuesday = monday + timedelta(days=1)
    # 周一规则
    r_mon = _rule(st, cid, days_of_week="1")
    # 用注入日期直接测 _matches_calendar（match 内部取 datetime.now，单测走纯函数）
    assert eng._matches_calendar(r_mon, datetime.combine(monday, datetime.min.time())) is True
    assert eng._matches_calendar(r_mon, datetime.combine(tuesday, datetime.min.time())) is False
    # 周二规则
    r_tue = _rule(st, cid, days_of_week="2")
    assert eng._matches_calendar(r_tue, datetime.combine(tuesday, datetime.min.time())) is True
    assert eng._matches_calendar(r_tue, datetime.combine(monday, datetime.min.time())) is False
    # 全星期缺省：任意天都过
    r_all = _rule(st, cid)
    assert eng._matches_calendar(r_all, datetime.combine(tuesday, datetime.min.time())) is True
    # 坏数据（空/乱串）静默不匹配
    r_bad = _rule(st, cid, days_of_week="")
    assert eng._matches_calendar(r_bad, datetime.combine(monday, datetime.min.time())) is False
    r_bad2 = _rule(st, cid, days_of_week="9,x")
    assert eng._matches_calendar(r_bad2, datetime.combine(monday, datetime.min.time())) is False


def test_holiday_only(st, monkeypatch):
    cid = st.add_camera("c", "D:/x.mp4", "file")
    eng = RuleEngine(st)
    wed = _next_weekday(3)  # 工作日（周三）
    sat = _next_weekday(6)  # 周末（默认算假期）
    r = _rule(st, cid, holiday_only=True)
    assert eng._matches_calendar(r, datetime.combine(wed, datetime.min.time())) is False  # 普通工作日
    assert eng._matches_calendar(r, datetime.combine(sat, datetime.min.time())) is True  # 周末=假期
    # workday 例外覆盖周末
    st.set_calendar_exception(sat.isoformat(), "workday", "补班")
    assert eng._matches_calendar(r, datetime.combine(sat, datetime.min.time())) is False
    # holiday 例外覆盖工作日
    st.set_calendar_exception(wed.isoformat(), "holiday", "国庆")
    assert eng._matches_calendar(r, datetime.combine(wed, datetime.min.time())) is True
    # 周末不算假期开关
    from app.config import settings
    monkeypatch.setattr(settings, "weekend_is_holiday", False)
    sat2 = _next_weekday(6) + timedelta(days=7)
    assert eng._matches_calendar(r, datetime.combine(sat2, datetime.min.time())) is False


def test_calendar_combines_with_time_window(st):
    cid = st.add_camera("c", "D:/x.mp4", "file")
    eng = RuleEngine(st)
    wed = _next_weekday(3)
    r = _rule(st, cid, days_of_week="3", holiday_only=True, time_start="08:00", time_end="18:00")
    st.set_calendar_exception(wed.isoformat(), "holiday")
    at_noon = datetime.combine(wed, datetime.min.time()).replace(hour=12)
    at_night = datetime.combine(wed, datetime.min.time()).replace(hour=23)
    assert eng._matches_calendar(r, at_noon) and eng._in_time_window(r, at_noon)  # 组合全过
    assert not eng._in_time_window(r, at_night)  # 假期但时间窗外


def test_migration_v6_defaults_and_old_rule_unchanged(tmp_path):
    """旧库（无 v6 列）迁移后：新列默认全星期/非假期，旧行为不变。"""
    db = tmp_path / "old.db"
    import sqlite3
    con = sqlite3.connect(str(db))
    con.executescript("""
      CREATE TABLE rules(id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, zone_id INTEGER,
        behavior TEXT, level INTEGER, cooldown_s INTEGER, time_start TEXT, time_end TEXT,
        threshold INTEGER, enabled INTEGER);
      INSERT INTO rules(camera_id,zone_id,behavior,level,cooldown_s,time_start,time_end,threshold,enabled)
        VALUES(1,NULL,'enter',2,60,'00:00','23:59',0,1);
    """)
    con.commit()
    con.close()
    st = Store(db)  # 触发迁移到 v6
    assert st.schema_version() >= 6
    r = st.rules(1)[0]
    assert r["days_of_week"] == "1,2,3,4,5,6,7" and r["holiday_only"] == 0
    eng = RuleEngine(st)
    any_day = datetime.combine(_next_weekday(4), datetime.min.time())
    assert eng._matches_calendar(r, any_day) is True  # 旧行为：全星期生效


# ---- API：calendar CRUD/格式校验/鉴权 ----

@pytest.fixture()
def client():
    with TestClient(app) as tc:
        yield tc


def test_calendar_api_crud_and_validation(client):
    d = _next_weekday(6).isoformat()
    # 新增
    r = client.post("/api/calendar", json={"date": d, "mode": "holiday", "note": "测试假"})
    assert r.status_code == 200 and r.json()["replaced"] is False
    # 重复日期 → 覆盖语义（upsert，replaced=true）
    r2 = client.post("/api/calendar", json={"date": d, "mode": "workday", "note": "补班"})
    assert r2.status_code == 200 and r2.json()["replaced"] is True
    lst = client.get("/api/calendar").json()["exceptions"]
    assert len(lst) == 1 and lst[0]["mode"] == "workday" and lst[0]["note"] == "补班"
    # 格式校验
    assert client.post("/api/calendar", json={"date": "2026-13-99", "mode": "holiday"}
                       ).status_code == 400
    assert client.post("/api/calendar", json={"date": "2026-01-01", "mode": "x"}
                       ).status_code == 400
    assert client.post("/api/calendar", json={"date": "", "mode": "holiday"}).status_code == 400
    # 删除 + 404
    assert client.delete(f"/api/calendar/{d}").json() == {"ok": True}
    assert client.delete(f"/api/calendar/{d}").status_code == 404
    assert client.get("/api/calendar").json()["exceptions"] == []


def test_calendar_auth(client, monkeypatch):
    monkeypatch.setattr(api_mod.settings, "admin_token", "sec")
    monkeypatch.setattr(api_mod.settings, "auth_read_protected", True)
    d = _next_weekday(6).isoformat()
    assert client.get("/api/calendar").status_code == 401  # 读保护
    assert client.get("/api/calendar", headers={"X-Admin-Token": "sec"}).status_code == 200
    assert client.post("/api/calendar", json={"date": d, "mode": "holiday"}).status_code == 401
    assert client.post("/api/calendar", json={"date": d, "mode": "holiday"},
                       headers={"X-Admin-Token": "sec"}).status_code == 200
    monkeypatch.setattr(api_mod.settings, "admin_token", "")
    monkeypatch.setattr(api_mod.settings, "auth_read_protected", False)


def test_rule_create_days_validation(client):
    cam = client.post("/api/cameras", json={"name": "c112", "source": "D:/x.mp4",
                                            "type": "file", "autostart": False}).json()
    ok = client.post("/api/rules", json={"camera_id": cam["id"], "behavior": "enter",
                                         "days_of_week": "1,5", "holiday_only": True})
    assert ok.status_code == 200
    rules = client.get(f"/api/rules?camera_id={cam['id']}").json()
    assert rules[-1]["days_of_week"] == "1,5" and rules[-1]["holiday_only"] is True
    assert client.post("/api/rules", json={"camera_id": cam["id"], "behavior": "enter",
                                           "days_of_week": "9"}).status_code == 400
    assert client.post("/api/rules", json={"camera_id": cam["id"], "behavior": "enter",
                                           "days_of_week": ""}).status_code == 400
