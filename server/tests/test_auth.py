"""单元测试：任务16 管理鉴权（token 关闭兼容/启用后写保护/WS 4401）。运行: pytest tests/ -q"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import api as api_mod  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402


@pytest.fixture()
def client(tmp_path, monkeypatch):
    s = Store(path=tmp_path / "t.db")

    class _StubManager:
        def start_camera(self, cam):
            pass

        def stop_camera(self, cid):
            pass

        def status(self):
            return {"cameras": []}

    monkeypatch.setattr(api_mod, "store", s)
    monkeypatch.setattr(api_mod, "manager", _StubManager())
    return TestClient(app)


def test_auth_disabled_zero_regression(client, monkeypatch):
    """默认（token 空）：写操作不带 token 也正常——现有行为零回归。"""
    monkeypatch.setattr(settings, "admin_token", "")
    r = client.post("/api/cameras", json={"name": "cam", "source": "x.mp4", "type": "file"})
    assert r.status_code == 200
    assert client.delete("/api/cameras/1").status_code == 200


def test_auth_enabled_write_protection(client, monkeypatch):
    monkeypatch.setattr(settings, "admin_token", "s3cret")
    # 无 token → 401
    r = client.post("/api/cameras", json={"name": "cam", "source": "x.mp4", "type": "file"})
    assert r.status_code == 401
    # 错 token → 401
    r = client.post("/api/cameras", json={"name": "cam", "source": "x.mp4", "type": "file"},
                    headers={"X-Admin-Token": "wrong"})
    assert r.status_code == 401
    # 正确 token → 200
    r = client.post("/api/cameras", json={"name": "cam", "source": "x.mp4", "type": "file"},
                    headers={"X-Admin-Token": "s3cret"})
    assert r.status_code == 200
    # 读操作默认放行
    assert client.get("/api/cameras").status_code == 200
    assert client.get("/api/events").status_code == 200
    # /api/system 永不鉴权
    assert client.get("/api/system").status_code == 200


def test_auth_read_protected_flag(client, monkeypatch):
    monkeypatch.setattr(settings, "admin_token", "s3cret")
    monkeypatch.setattr(settings, "auth_read_protected", True)
    assert client.get("/api/events").status_code == 401
    assert client.get("/api/events", headers={"X-Admin-Token": "s3cret"}).status_code == 200
    assert client.get("/api/system").status_code == 200  # 永不拦


def test_ws_close_4401(client, monkeypatch):
    """T-106/Q-03：WS 与 HTTP 读口径一致——未开读保护时仅设管理令牌不要求 WS 令牌
    （修正旧 B-04 反例）；开读保护后无令牌 4401、子协议/查询参数令牌可连。"""
    from starlette.websockets import WebSocketDisconnect
    monkeypatch.setattr(settings, "admin_token", "s3cret")
    monkeypatch.setattr(settings, "auth_read_protected", False)
    with client.websocket_connect("/ws/events") as ws:
        assert ws is not None  # 口径一致：不需要令牌
    monkeypatch.setattr(settings, "auth_read_protected", True)
    with pytest.raises(WebSocketDisconnect) as ei:
        with client.websocket_connect("/ws/events"):
            pass
    assert ei.value.code == 4401
    # 子协议传令牌（T-106 推荐形态，不进 URL）
    with client.websocket_connect("/ws/events", subprotocols=["labsentinel", "bearer.s3cret"]) as ws:
        assert ws is not None
    # 兼容旧 ?token=（弃用期）
    with client.websocket_connect("/ws/events?token=s3cret") as ws:
        assert ws is not None
