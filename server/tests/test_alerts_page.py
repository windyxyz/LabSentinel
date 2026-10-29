"""单元测试：任务23 告警渠道状态页（渠道遍历/configured 判定/不泄露敏感值）。运行: pytest tests/ -q"""
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
def env(tmp_path, monkeypatch):
    s = Store(path=tmp_path / "t.db")
    monkeypatch.setattr(api_mod, "store", s)
    monkeypatch.setattr(api_mod.dispatcher, "store", s)  # alerts/status 读 dispatcher.store
    return s, TestClient(app)


def test_alerts_status_channels_traversal(env, monkeypatch):
    """七渠道全遍历、configured 判定按各自字段组合。"""
    s, c = env
    monkeypatch.setattr(settings, "wecom_webhook", "https://qyapi.weixin.qq.com/x")
    monkeypatch.setattr(settings, "dingtalk_webhook", None)
    monkeypatch.setattr(settings, "qmsg_key", "k")          # qmsg 需要 key+qq 同时非空
    monkeypatch.setattr(settings, "qmsg_qq", "")
    monkeypatch.setattr(settings, "smtp_host", "smtp.qq.com")
    monkeypatch.setattr(settings, "alert_email_to", None)   # email 需要 host+to 同时非空
    body = c.get("/api/alerts/status").json()
    names = [ch["name"] for ch in body["channels"]]
    assert names == ["wecom", "dingtalk", "feishu", "serverchan", "pushplus", "qmsg", "email", "webhook"]  # 任务67 +通用Webhook
    cfg = {ch["name"]: ch["configured"] for ch in body["channels"]}
    assert cfg["wecom"] is True and cfg["dingtalk"] is False
    assert cfg["qmsg"] is False        # 缺 qq
    assert cfg["email"] is False       # 缺 to
    assert body["test_supported"] is True
    # deliveries 汇总结构
    for ch in body["channels"]:
        assert set(ch["deliveries"]) == {"ok", "failed", "pending"}


def test_alerts_status_no_secret_leak(env, monkeypatch):
    """响应体不泄露 token/webhook/key 原值。"""
    s, c = env
    monkeypatch.setattr(settings, "wecom_webhook", "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=SECRETKEY")
    monkeypatch.setattr(settings, "serverchan_sendkey", "SECRETKEY2")
    monkeypatch.setattr(settings, "smtp_pass", "SECRETKEY3")
    raw = c.get("/api/alerts/status").text
    for secret in ("SECRETKEY", "SECRETKEY2", "SECRETKEY3"):
        assert secret not in raw


def test_alerts_status_delivery_counts(env, monkeypatch):
    s, c = env
    did = s.add_delivery(1, "dingtalk")
    s.update_delivery(did, "failed", 4, "boom")
    did2 = s.add_delivery(2, "dingtalk")
    s.update_delivery(did2, "ok", 1)
    body = c.get("/api/alerts/status").json()
    dt = [ch for ch in body["channels"] if ch["name"] == "dingtalk"][0]["deliveries"]
    assert dt == {"ok": 1, "failed": 1, "pending": 0}
