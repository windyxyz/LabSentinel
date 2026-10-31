"""单元测试：任务81 告警静默（match_silence/dispatcher 零投递/API CRUD/迁移 v2）。"""
import os
import sqlite3
import sys
from datetime import datetime, timedelta

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import alerts as al  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402

NOW = datetime(2026, 10, 31, 12, 0, 0)


def sil(scope="global", target=None, until=None):
    return {"id": 1, "scope": scope, "target": target,
            "until_ts": (until or "2026-10-31T13:00:00")}


def ev(camera_id=1, level=2, type_="enter"):
    return {"id": 9, "camera_id": camera_id, "level": level, "type": type_}


def test_match_silence_scopes():
    g = sil("global")
    assert al.match_silence([g], ev()) is g  # global 全匹配
    cam = sil("camera", "1")
    assert al.match_silence([cam], ev(camera_id=1)) is cam
    assert al.match_silence([cam], ev(camera_id=2)) is None
    lv = sil("level", "3")
    assert al.match_silence([lv], ev(level=3)) is lv
    assert al.match_silence([lv], ev(level=2)) is None
    ty = sil("type", "enter_no_ppe")
    assert al.match_silence([ty], ev(type_="enter_no_ppe")) is ty
    assert al.match_silence([ty], ev()) is None


def test_match_silence_expiry_and_multi():
    expired = sil("global", until="2026-10-31T11:00:00")
    assert al.match_silence([expired], ev(), now=NOW) is None  # 过期不命中
    fresh = sil("global", until="2026-10-31T13:00:00")
    assert al.match_silence([expired, fresh], ev(), now=NOW) is fresh  # 多条取任一命中


def test_dispatcher_silenced_zero_send(tmp_path, monkeypatch):
    s = Store(tmp_path / "s.db")
    s.add_silence("global", None, "维护窗口", duration_s=3600)
    d = al.AlertDispatcher(store=s)
    sends = {"n": 0}

    class FakeCh:
        name = "fake"

        def configured(self):
            return True

        def send(self, level, text):
            sends["n"] += 1
            return True

    d.channels = [FakeCh()]
    d.dispatch({"id": 5, "ts": "2026-10-31T12:00:00", "camera_id": 1, "type": "enter",
                "level": 2, "track_id": 1, "detail": "静默期事件"})
    import time as _t
    _t.sleep(0.3)  # 线程池异步投递
    assert sends["n"] == 0  # 静默：零发送
    assert s.delivery_counts()["fake"] == {"ok": 0, "failed": 0, "pending": 0} \
        if isinstance(s.delivery_counts(), dict) and "fake" in s.delivery_counts() else True
    rows = s.deliveries(status=None, limit=50)
    assert not [r for r in rows if r["channel"] == "fake"]  # 无投递行
    # 解除静默 → 正常投递
    for r in s.silences(active_only=True):
        s.delete_silence(r["id"])
    d.dispatch({"id": 6, "ts": "2026-10-31T12:01:00", "camera_id": 1, "type": "enter",
                "level": 2, "track_id": 1, "detail": "恢复后事件"})
    _t.sleep(0.3)
    assert sends["n"] == 1
    rows = s.deliveries(status=None, limit=50)
    assert [r for r in rows if r["channel"] == "fake"]


def test_migration_v2_on_legacy_db(tmp_path, monkeypatch):
    """v1 旧库（有 schema_version=1、无 alert_silences 表）→ 打开 Store 自动建表并升到 v2。"""
    db = tmp_path / "legacy.db"
    con = sqlite3.connect(db)
    # 真实"v1 已应用"旧库形状：events 含 v1 产物列（video/status/...），缺的是 v2 的表
    con.execute("CREATE TABLE events(id INTEGER PRIMARY KEY, ts TEXT, camera_id INTEGER, "
                "type TEXT, level INTEGER, track_id INTEGER, detail TEXT, snapshot TEXT, "
                "video TEXT, status TEXT DEFAULT 'new', handled_by TEXT, handled_ts TEXT)")
    con.execute("CREATE TABLE schema_version(version INTEGER PRIMARY KEY, applied_at TEXT)")
    con.execute("INSERT INTO schema_version(version, applied_at) VALUES(1, '2026-01-01T00:00:00')")
    con.commit()
    con.close()
    monkeypatch.setattr(settings, "db_path", db)
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    s = Store(str(db))
    assert s.schema_version() == 4  # 迁移跑到最新（v4，任务87）
    con = sqlite3.connect(db)
    tables = {r[0] for r in con.execute("select name from sqlite_master where type='table'")}
    cols = {r[1] for r in con.execute("PRAGMA table_info(events)")}
    con.close()
    assert "alert_silences" in tables and "escalated_ts" in cols


def test_silence_api_crud_and_validation(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "admin_token", "")
    with TestClient(app) as c:
        cid = c.post("/api/cameras", json={"name": "cam81", "type": "file",
                                           "source": "x.mp4", "autostart": False}).json()["id"]
        # 创建
        r = c.post("/api/alert-silences", json={"scope": "global", "duration_s": 3600,
                                                "reason": "设备维护"})
        assert r.status_code == 200
        sid = r.json()["id"]
        lst = c.get("/api/alert-silences", params={"active": 1}).json()
        assert any(x["id"] == sid and x["remaining_s"] > 0 for x in lst)
        # camera 静默：目标存在
        assert c.post("/api/alert-silences", json={"scope": "camera", "target": str(cid),
                                                   "duration_s": 3600}).status_code == 200
        # 校验
        assert c.post("/api/alert-silences", json={"scope": "camera"}).status_code == 400
        assert c.post("/api/alert-silences",
                      json={"scope": "camera", "target": "9999"}).status_code == 404
        assert c.post("/api/alert-silences",
                      json={"scope": "level", "target": "9"}).status_code == 400
        assert c.post("/api/alert-silences",
                      json={"scope": "type", "target": "nope"}).status_code == 400
        assert c.post("/api/alert-silences", json={"scope": "global",
                                                   "duration_s": 10}).status_code == 400
        # 解除 + 404
        assert c.delete(f"/api/alert-silences/{sid}").status_code == 200
        assert c.delete(f"/api/alert-silences/{sid}").status_code == 404


def test_silence_api_auth(monkeypatch):
    """默认（未启用鉴权）：GET/POST 均 200；启用令牌：写操作 401/带令牌 200。"""
    with TestClient(app) as c:
        assert c.post("/api/alert-silences",
                      json={"scope": "global", "duration_s": 3600}).status_code == 200
    monkeypatch.setattr(settings, "admin_token", "tok81")
    with TestClient(app) as c:
        assert c.get("/api/alert-silences").status_code == 200  # 读未保护（默认口径）
        assert c.post("/api/alert-silences", json={"scope": "global"}).status_code == 401
        r = c.post("/api/alert-silences", json={"scope": "global", "duration_s": 3600},
                   headers={"X-Admin-Token": "tok81"})
        assert r.status_code == 200
        assert c.delete("/api/alert-silences/9999",
                        headers={"X-Admin-Token": "tok81"}).status_code == 404
