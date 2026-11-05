"""T-101/T-102/T-105/T-106 回归：go2rtc 收敛、WebRTC 代理、CSRF/Host、WS 口径。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import go2rtc  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402


def test_go2rtc_yaml_locked_down(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    y = go2rtc.build_yaml([{"id": 1, "type": "rtsp", "source": "rtsp://u:p@1.2.3.4/stream"}])
    assert 'listen: "127.0.0.1:1984"' in y
    assert not any(line.strip().startswith("origin:") for line in y.splitlines())
    assert "username:" in y and "password:" in y
    assert 'listen: ":8555"' in y
    assert (tmp_path / "go2rtc.secret").exists()
    # 凭据稳定（重启不换）
    assert go2rtc._secret() == go2rtc._secret()


def test_webrtc_endpoint_returns_relative_proxy_url(client):
    from app import api as api_mod
    from app.store import Store
    import tempfile
    from pathlib import Path
    old = api_mod.store
    try:
        s = Store(Path(tempfile.mkdtemp()) / "s.db")
        cid = s.add_camera("cam", "rtsp://1.2.3.4/stream", "rtsp")
        api_mod.store = s
        r = client.get(f"/api/cameras/{cid}/webrtc")
        assert r.status_code == 200
        assert r.json()["ws_url"] == f"/ws/webrtc/{cid}"  # 不再直连 :1984
    finally:
        api_mod.store = old


def test_csrf_cross_origin_write_blocked(client):
    r = client.post("/api/system/shutdown", headers={"Origin": "https://evil.example"})
    assert r.status_code == 403 and r.json()["code"] == "csrf_blocked"


def test_dns_rebinding_host_blocked(client):
    r = client.get("/api/cameras", headers={"Host": "evil.example"})
    assert r.status_code == 400


def test_ws_cross_origin_4403(client):
    # T-106：跨站 WS 直接拒绝（4403）
    from starlette.websockets import WebSocketDisconnect
    with pytest.raises(WebSocketDisconnect) as ei:
        with client.websocket_connect("/ws/events", headers={"Origin": "https://evil.example"}):
            pass
    assert ei.value.code == 4403


@pytest.fixture()
def client():
    with TestClient(app) as c:
        yield c
