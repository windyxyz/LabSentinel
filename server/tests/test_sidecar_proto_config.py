"""R2-305：ApplyConfig 渠道快照单测（不依赖 ls-analyzer exe——纯 _proto_config 构造）。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app.analyzer_sidecar import SidecarAnalyzer  # noqa: E402
from conftest import rs_exe  # noqa: E402

EXE = rs_exe("ls-analyzer", "LS_ANALYZER_EXE")


def test_proto_config_includes_alert_channels(monkeypatch):
    """R2-305：ApplyConfig 快照必须携带已解密的渠道配置（网页配置 → rust 路径生效）。"""
    from app.config import settings

    monkeypatch.setattr(settings, "wecom_webhook",
                        "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=t")
    monkeypatch.setattr(settings, "webhook_url", "http://192.168.1.5/hook")
    monkeypatch.setattr(settings, "webhook_secret", "s3cret")
    monkeypatch.setattr(settings, "webhook_allow_http", True)
    monkeypatch.setattr(settings, "qmsg_key", "k1")
    monkeypatch.setattr(settings, "qmsg_qq", "10001")
    monkeypatch.setattr(settings, "smtp_host", "smtp.example.com")
    monkeypatch.setattr(settings, "smtp_port", 465)
    monkeypatch.setattr(settings, "alert_email_to", "ops@example.com")
    sa = SidecarAnalyzer(exe=EXE)
    cfg = sa._proto_config()
    names = [c.channel for c in cfg.alert_channels.channels]
    assert "wecom" in names
    assert "webhook" in names
    assert "qmsg" in names
    assert "email" in names
    by = {c.channel: c for c in cfg.alert_channels.channels}
    assert by["webhook"].secret == "s3cret"  # 快照是已解密明文（仅本机 gRPC）
    assert by["webhook"].allow_http is True
    assert by["qmsg"].qq == "10001"
    assert by["email"].email.host == "smtp.example.com"
    assert by["email"].email.to == "ops@example.com"
