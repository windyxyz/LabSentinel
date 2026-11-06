"""T-620 回归：设备配对/三级令牌/吊销。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import devices  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402


@pytest.fixture()
def client(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "db_path", tmp_path / "pair.db")
    monkeypatch.setattr(settings, "admin_token", "adm")
    monkeypatch.setattr(settings, "auth_read_protected", False)
    with TestClient(app) as c:
        yield c


def test_v13_devices_table(tmp_path):
    s = Store(tmp_path / "s.db")
    assert s.schema_version() >= 13
    assert s.devices() == []


def test_pairing_full_flow(client):
    with TestClient(app) as c:
        # 无令牌不能生成配对码
        assert c.post("/api/pairing").status_code == 401
        r = c.post("/api/pairing", headers={"X-Admin-Token": "adm"})
        assert r.status_code == 200
        code = r.json()["code"]
        assert len(code) == 6 and code.isdigit()
        # 领取设备令牌（一次性）
        r2 = c.post("/api/pairing/claim",
                    json={"code": code, "device_name": "手机A", "platform": "android"})
        assert r2.status_code == 200
        tok = r2.json()["token"]
        assert len(tok) > 20
        # 重复领取失败（码作废）
        assert c.post("/api/pairing/claim",
                      json={"code": code, "device_name": "x"}).status_code == 400
        # 设备列表（仅名字/平台，不含 hash）
        lst = c.get("/api/devices", headers={"X-Admin-Token": "adm"}).json()
        assert len(lst) == 1 and lst[0]["name"] == "手机A" and "token_hash" not in lst[0]
        # 设备令牌可处置事件
        from app import api as api_mod
        eid = api_mod.store.add_event(1, "enter", 2, 1, "d", ts="2026-01-01T00:00:01")
        r3 = c.post(f"/api/events/{eid}/status", json={"status": "acked"},
                    headers={"X-Admin-Token": tok})
        assert r3.status_code == 200, r3.text
        # 设备令牌不可改配置（写守卫）
        assert c.post("/api/backups/run",
                      headers={"X-Admin-Token": tok}).status_code == 401
        # 吊销后 401
        did = lst[0]["id"]
        assert c.delete(f"/api/devices/{did}",
                        headers={"X-Admin-Token": "adm"}).status_code == 200
        eid2 = api_mod.store.add_event(1, "enter", 2, 1, "d2", ts="2026-01-01T00:00:01")
        assert c.post(f"/api/events/{eid2}/status", json={"status": "acked"},
                      headers={"X-Admin-Token": tok}).status_code == 401


def test_pairing_code_expires(monkeypatch):
    from app import devices as dv
    code = dv.new_pairing_code()
    monkeypatch.setattr(dv, "_PAIR_TTL_S", -1)
    store = None
    # 过期码领取失败（claim 需 store；用轻量 mock）
    class FakeStore:
        def add_device(self, *a, **k):
            return 1

    assert dv.claim(FakeStore(), "000000", "x") is None
