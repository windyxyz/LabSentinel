"""R2-103/N-02 回归：设备令牌必须能读（读保护开启时）且不能写。

77 号 Q-11 口径「设备令牌 = 查看 + 处置」。局域网绑定会强制开启
auth_read_protected，若 require_read/_ws_auth_check 只认 admin/read_token，
配对后的手机在唯一需要它的场景里拿不到任何数据。
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import devices  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402


@pytest.fixture()
def client(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "db_path", tmp_path / "dev-read.db")
    monkeypatch.setattr(settings, "admin_token", "adm")
    monkeypatch.setattr(settings, "auth_read_protected", True)  # 局域网强制开启
    monkeypatch.setattr(settings, "host", "0.0.0.0")
    with TestClient(app) as c:
        yield c


def _pair(client) -> str:
    code = client.post("/api/pairing",
                       headers={"X-Admin-Token": "adm"}).json()["code"]
    return client.post("/api/pairing/claim",
                       json={"code": code, "device_name": "手机", "platform": "android"}
                       ).json()["token"]


def test_device_token_can_read_with_read_protection(client):
    tok = _pair(client)
    # X-Admin-Token 形式
    assert client.get("/api/events", headers={"X-Admin-Token": tok}).status_code == 200
    # 移动端惯例：Authorization: Bearer
    assert client.get("/api/events",
                      headers={"Authorization": f"Bearer {tok}"}).status_code == 200
    # 匿名仍被拒
    assert client.get("/api/events").status_code == 401


def test_device_token_cannot_write(client):
    tok = _pair(client)
    assert client.post("/api/backups/run",
                       headers={"X-Admin-Token": tok}).status_code == 401
    assert client.post("/api/backups/run",
                       headers={"Authorization": f"Bearer {tok}"}).status_code == 401


def test_revoked_device_token_loses_read(client):
    tok = _pair(client)
    from app import api as api_mod
    did = api_mod.store.devices()[0]["id"]
    assert client.delete(f"/api/devices/{did}",
                         headers={"X-Admin-Token": "adm"}).status_code == 200
    assert client.get("/api/events", headers={"X-Admin-Token": tok}).status_code == 401


def test_touch_device_throttled(client):
    """读请求密集时不应每次都写 SQLite（R2-103 节流）。"""
    tok = _pair(client)
    from app import api as api_mod
    calls = {"n": 0}
    orig = api_mod.store.touch_device

    def counted(did):
        calls["n"] += 1
        return orig(did)

    api_mod.store.touch_device = counted
    try:
        for _ in range(10):
            assert client.get("/api/events", headers={"X-Admin-Token": tok}).status_code == 200
        assert calls["n"] <= 1, calls
    finally:
        api_mod.store.touch_device = orig
