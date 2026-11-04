"""任务140：网页端告警配置（掩码/校验/持久化/回退/单渠道测试）。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402

from app import alert_config  # noqa: E402
from app.alerts import AlertDispatcher  # noqa: E402
from app.config import settings  # noqa: E402
from app.store import Store  # noqa: E402
from fastapi import HTTPException  # noqa: E402

WECOM = "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=verysecret123"


def store(tmp_path):
    return Store(tmp_path / "alert.db")


def test_status_masks_secrets_and_urls(tmp_path, monkeypatch):
    s = store(tmp_path)
    monkeypatch.setattr(settings, "wecom_webhook", WECOM)
    monkeypatch.setattr(settings, "pushplus_token", "pp-secret")
    monkeypatch.setattr(settings, "smtp_host", "smtp.qq.com")
    monkeypatch.setattr(settings, "smtp_port", 465)
    monkeypatch.setattr(settings, "smtp_pass", "mail-pass")
    monkeypatch.setattr(settings, "alert_email_to", "to@example.com")
    st = alert_config.status(s)
    wecom = st["channels"]["wecom"]
    assert wecom["configured"] and wecom["has_webhook"]
    assert "verysecret" not in wecom["webhook"] and "key=***" in wecom["webhook"]
    pp = st["channels"]["pushplus"]
    assert pp["has_token"] and pp["token"] == ""
    mail = st["channels"]["email"]
    assert mail["configured"] and mail["has_smtp_pass"] and mail["smtp_pass"] == ""
    assert mail["to"] == "to@example.com" and mail["smtp_port"] == 465


def test_save_applies_persists_and_blank_keeps(tmp_path, monkeypatch):
    s = store(tmp_path)
    monkeypatch.setattr(settings, "wecom_webhook", "")
    monkeypatch.setattr(settings, "pushplus_token", "orig-token")
    st = alert_config.save(s, {"wecom": {"webhook": WECOM}})
    assert settings.wecom_webhook == WECOM
    assert st["channels"]["wecom"]["source"] == "db"
    assert any(k.endswith("wecom_webhook") for k in s.app_settings("alert."))

    # 空 secret = 保持原值，不落 DB 覆盖
    alert_config.save(s, {"pushplus": {"token": ""}})
    assert settings.pushplus_token == "orig-token"
    assert "alert.pushplus_token" not in s.app_settings("alert.")

    # 重启：新 Store 加载 DB 覆盖
    s2 = Store(s.path)
    monkeypatch.setattr(settings, "wecom_webhook", "")
    alert_config.load(s2)
    assert settings.wecom_webhook == WECOM


def test_validation_rejects_bad_values(tmp_path, monkeypatch):
    s = store(tmp_path)
    for field in ("wecom_webhook", "webhook_url"):
        monkeypatch.setattr(settings, field, "")
    cases = [
        {"wecom": {"webhook": "http://evil.example/x"}},
        {"wecom": {"webhook": "https://example.com/x"}},
        {"nope": {"x": "1"}},
        {"wecom": {"unknown": "x"}},
        {"email": {"smtp_host": "smtp.qq.com"}},  # 缺收件人
        {"email": {"smtp_port": "abc"}},
        {"qmsg": {"key": "k"}},  # 缺 QQ
        {"webhook": {"url": "http://127.0.0.1/hook"}},  # 未允许 http
        {"webhook": {"url": "ftp://127.0.0.1/hook"}},
        {"webhook": {"url": "https://user:pass@example.com/hook"}},
    ]
    for body in cases:
        with pytest.raises(alert_config.AlertConfigError):
            alert_config.save(s, body)


def test_reset_restores_defaults(tmp_path, monkeypatch):
    s = store(tmp_path)
    monkeypatch.setitem(alert_config._DEFAULTS, "wecom_webhook", "https://env.example/hook")
    monkeypatch.setattr(settings, "wecom_webhook", "")
    alert_config.save(s, {"wecom": {"webhook": WECOM}})
    assert settings.wecom_webhook == WECOM
    st = alert_config.reset(s, "wecom")
    assert settings.wecom_webhook == "https://env.example/hook"
    assert st["channels"]["wecom"]["source"] in ("env", "none")
    with pytest.raises(alert_config.AlertConfigError):
        alert_config.reset(s, "nope")


class _Ch:
    def __init__(self, name):
        self.name = name

    def configured(self):
        return True

    def send(self, level, text):
        return True


def test_send_test_channel_filter():
    d = AlertDispatcher(store=None)
    d.channels = [_Ch("a"), _Ch("b")]
    assert [r["channel"] for r in d.send_test("a")] == ["a"]
    assert d.send_test("nope")[0]["error"] == "未知渠道"


def test_schema_v10_table(tmp_path):
    s = Store(tmp_path / "v10.db")
    assert s.schema_version() >= 10
    assert s.app_settings("alert.") == {}


def test_api_endpoints_direct(tmp_path, monkeypatch):
    from app import api
    s = Store(tmp_path / "api.db")
    monkeypatch.setattr(api, "store", s)
    monkeypatch.setattr(settings, "wecom_webhook", "")
    out = api.alerts_config_put({"wecom": {"webhook": WECOM}})
    assert out["channels"]["wecom"]["configured"] is True
    with pytest.raises(HTTPException) as ei:
        api.alerts_config_put({"wecom": {"webhook": "http://evil/x"}})
    assert ei.value.status_code == 400
    api.alerts_config_reset("wecom")
    assert settings.wecom_webhook == alert_config._DEFAULTS["wecom_webhook"]
