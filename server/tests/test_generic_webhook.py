"""单元测试：任务67 通用 Webhook 渠道（URL 放行 / HMAC 签名 / 2xx 判定 / status&test 集成）。
运行: pytest tests/test_generic_webhook.py -q"""
import hashlib
import hmac as hmac_mod
import json
import os
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import alerts as al  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402

SECRET = "test-secret-67"


@pytest.fixture()
def ch(monkeypatch):
    monkeypatch.setattr(settings, "webhook_url", "https://example.internal/hook")
    monkeypatch.setattr(settings, "webhook_secret", "")
    monkeypatch.setattr(settings, "webhook_allow_http", False)
    return al.GenericWebhookChannel()


def test_configured_blank(ch, monkeypatch):
    assert ch.configured() is True
    for blank in ("", "   "):
        monkeypatch.setattr(settings, "webhook_url", blank)
        assert ch.configured() is False


def test_url_rules(ch, monkeypatch):
    import httpx
    c = al.GenericWebhookChannel(transport=httpx.MockTransport(
        lambda request: httpx.Response(200, json={"ok": 1})))
    # https 通过
    assert c.send(2, "t") is True
    # http 默认拒绝（transport 不应被调用到）
    monkeypatch.setattr(settings, "webhook_url", "http://example.internal/hook")
    assert c.send(2, "t") is False
    # http + allow_http 通过
    monkeypatch.setattr(settings, "webhook_allow_http", True)
    assert c.send(2, "t") is True
    # 非 http(s) 拒绝
    monkeypatch.setattr(settings, "webhook_url", "ftp://example.internal/f")
    assert c.send(2, "t") is False
    # 带凭证 URL 拒绝
    monkeypatch.setattr(settings, "webhook_url", "https://user:pass@example.internal/h")
    assert c.send(2, "t") is False


class _Receiver(BaseHTTPRequestHandler):
    last = {}

    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(n)
        _Receiver.last = {"path": self.path, "body": body, "headers": dict(self.headers)}
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'{"ok":1}')

    def log_message(self, *a):  # 静默
        pass


def test_local_receiver_hmac(monkeypatch):
    srv = ThreadingHTTPServer(("127.0.0.1", 0), _Receiver)
    port = srv.server_address[1]
    th = threading.Thread(target=srv.serve_forever, daemon=True)
    th.start()
    try:
        monkeypatch.setattr(settings, "webhook_url", f"http://127.0.0.1:{port}/hook")
        monkeypatch.setattr(settings, "webhook_allow_http", True)
        monkeypatch.setattr(settings, "webhook_secret", SECRET)
        c = al.GenericWebhookChannel()
        assert c.send(3, "三级告警文本", event={"id": 42, "camera_id": 1, "type": "enter_no_ppe",
                                               "detail": "未穿实验服"}) is True
        r = _Receiver.last
        assert r["path"] == "/hook"
        payload = json.loads(r["body"])
        assert payload["version"] == 1 and payload["level"] == 3
        assert payload["source"] == "labsentinel" and payload["text"] == "三级告警文本"
        assert payload["title"] == "LabSentinel 级别3告警"
        assert payload["event_id"] == 42 and payload["type"] == "enter_no_ppe"
        assert payload["ts"] and "X-LS-Timestamp" in r["headers"]
        # 独立计算 HMAC 对原始字节验证签名
        want = hmac_mod.new(SECRET.encode(), r["body"], hashlib.sha256).hexdigest()
        assert r["headers"]["X-LS-Signature"] == f"sha256={want}"
        assert r["headers"]["X-LS-Event"] == "alert"
        assert r["headers"]["Content-Type"] == "application/json"
    finally:
        srv.shutdown()
        srv.server_close()


def test_5xx_and_no_secret_ok(monkeypatch):
    import httpx

    monkeypatch.setattr(settings, "webhook_url", "https://example.internal/hook")
    monkeypatch.setattr(settings, "webhook_secret", "")
    c = al.GenericWebhookChannel(transport=httpx.MockTransport(
        lambda request: httpx.Response(500)))
    assert c.send(2, "t") is False
    c2 = al.GenericWebhookChannel(transport=httpx.MockTransport(
        lambda request: httpx.Response(200, text="ok")))
    assert c2.send(2, "t") is True  # 无密钥也能发；2xx 即成功


def test_status_and_test_endpoint(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "db_path", tmp_path / "s.db")
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    monkeypatch.setattr(settings, "webhook_url", None)  # 未配置
    with TestClient(app) as c:
        names = [x["name"] for x in c.get("/api/alerts/status").json()["channels"]]
        assert "webhook" in names
        res = c.post("/api/alert-test").json()
        row = next(x for x in res if x["channel"] == "webhook")  # alert-test 返回键是 channel
        assert row["ok"] is False and row["error"] == "未配置"
        # 未配置不建投递行
        from app.store import Store
        import sqlite3
        con = sqlite3.connect(tmp_path / "s.db")
        n = con.execute("select count(*) from alert_deliveries where channel='webhook'").fetchone()[0]
        con.close()
        assert n == 0
