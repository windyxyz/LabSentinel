"""单元测试：任务91 模型/配置只读状态（不加载 ONNX；脱敏摘要；鉴权口径）。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402


@pytest.fixture()
def client(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "admin_token", "")
    with TestClient(app) as c:
        yield c


def test_model_status_fields(client, tmp_path, monkeypatch):
    model = tmp_path / "yolo11n_ppe.onnx"
    model.write_bytes(b"0" * 2000)
    classes = tmp_path / "yolo11n_ppe.classes.txt"
    classes.write_text("person\nlab coat\nno lab coat\n", encoding="utf-8")
    monkeypatch.setattr(settings, "ppe_model", model)
    monkeypatch.setattr(settings, "model_fp32", tmp_path / "yolo11n.onnx")
    (tmp_path / "yolo11n.onnx").write_bytes(b"0" * 1000)
    r = client.get("/api/model/status")
    assert r.status_code == 200
    d = r.json()
    assert d["models"]["ppe"]["exists"] is True
    assert d["models"]["ppe"]["size_bytes"] == 2000
    assert d["classes"]["count"] == 3
    assert "lab coat" in d["classes"]["names"]
    assert d["classes"]["file"].endswith(".classes.txt")
    assert d["face_enabled"] is False
    assert "backend" in d and "imgsz" in d and "conf" in d


def test_model_status_missing_files(client, tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "ppe_model", tmp_path / "nope.onnx")
    monkeypatch.setattr(settings, "model_fp32", tmp_path / "also-nope.onnx")
    r = client.get("/api/model/status")
    assert r.status_code == 200
    d = r.json()
    assert d["models"]["ppe"]["exists"] is False
    assert d["classes"]["exists"] is False


def test_config_summary_redaction(client, monkeypatch):
    monkeypatch.setattr(settings, "admin_token", "real-admin-tok")
    monkeypatch.setattr(settings, "read_token", "real-read-tok")
    monkeypatch.setattr(settings, "smtp_pass", "real-smtp-pass")
    monkeypatch.setattr(settings, "webhook_url", "https://hook.example/x?token=tok123")
    monkeypatch.setattr(settings, "smtp_host", "smtp.qq.com")
    monkeypatch.setattr(settings, "ppe_conf", 0.30)
    r = client.get("/api/config/summary", headers={"X-Admin-Token": "real-admin-tok"})
    assert r.status_code == 200
    text = "\n".join(r.json()["keys"])
    for sensitive in ("real-admin-tok", "real-read-tok", "real-smtp-pass", "tok123"):
        assert sensitive not in text, sensitive
    assert "LS_ADMIN_TOKEN=***" in text
    assert "LS_SMTP_PASS=***" in text
    assert "LS_SMTP_HOST=smtp.qq.com" in text  # 普通键保留
    assert "LS_PPE_CONF=0.3" in text


def test_config_summary_requires_admin(client, monkeypatch):
    monkeypatch.setattr(settings, "admin_token", "tok91")
    r = client.get("/api/config/summary")
    assert r.status_code == 401
    r2 = client.get("/api/config/summary", headers={"X-Admin-Token": "tok91"})
    assert r2.status_code == 200


def test_model_status_requires_read(client, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", True)
    monkeypatch.setattr(settings, "admin_token", "tok91")
    with TestClient(app) as c:
        assert c.get("/api/model/status").status_code == 401
        assert c.get("/api/model/status",
                     headers={"X-Admin-Token": "tok91"}).status_code == 200
