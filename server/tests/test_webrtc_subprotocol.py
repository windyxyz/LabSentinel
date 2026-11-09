"""R2-104/N-05 回归：WebRTC 信令 WS 必须回显 Sec-WebSocket-Protocol。

RFC 6455 §4.1：客户端发送了子协议而服务端响应不带该头 → 浏览器必须断开。
首启令牌默认开启后前端 WebrtcPlayer.vue 会带 ['labsentinel', 'bearer.<token>']，
旧实现 `await ws.accept()` 不回显 → 实时画面/大屏失效。
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from fastapi.testclient import TestClient  # noqa: E402

from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402


def _patch_proxy(monkeypatch, seen: dict):
    """避免真的连 127.0.0.1:1984；只记录握手是否完成。"""
    import app.webrtc_proxy as wp

    async def fake_proxy(ws, cid):
        seen["accepted"] = True
        seen["cid"] = cid
        # 保持连接直到客户端关闭（TestClient 退出 with 时断开）
        while True:
            await ws.receive_text()

    monkeypatch.setattr(wp, "proxy", fake_proxy)


def test_webrtc_ws_echoes_subprotocol(monkeypatch, tmp_path):
    monkeypatch.setattr(settings, "db_path", tmp_path / "ws.db")
    monkeypatch.setattr(settings, "admin_token", "")
    monkeypatch.setattr(settings, "auth_read_protected", False)
    seen: dict = {}
    _patch_proxy(monkeypatch, seen)
    with TestClient(app) as c:
        with c.websocket_connect("/ws/webrtc/1",
                                 subprotocols=["labsentinel", "bearer.s3cret"]) as ws:
            assert ws.accepted_subprotocol == "labsentinel"
            assert seen.get("accepted") is True
            assert seen.get("cid") == 1


def test_webrtc_ws_without_subprotocol_still_connects(monkeypatch, tmp_path):
    monkeypatch.setattr(settings, "db_path", tmp_path / "ws2.db")
    monkeypatch.setattr(settings, "admin_token", "")
    monkeypatch.setattr(settings, "auth_read_protected", False)
    seen: dict = {}
    _patch_proxy(monkeypatch, seen)
    with TestClient(app) as c:
        with c.websocket_connect("/ws/webrtc/2") as ws:
            assert ws.accepted_subprotocol is None
            assert seen.get("accepted") is True
