"""Q-08 / T-406 回归：PPE 模型按需下载（许可确认 + SHA256）+ 构建链不随包。"""
import hashlib
import json
import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app.main import app  # noqa: E402
from app.routers import models as ppe  # noqa: E402


@pytest.fixture()
def client():
    with TestClient(app) as c:
        yield c


def _lock():
    return json.loads((ppe._LOCK).read_text(encoding="utf-8"))["assets"]


def test_asset_lock_has_ppe_entries():
    assets = _lock()
    for n in ("yolo11n_ppe.onnx", "yolo11n_ppe.classes.txt"):
        spec = assets[n]
        assert len(spec["sha256"]) == 64
        assert spec["size"] > 0
        assert all(u.startswith("https://") for u in spec["urls"])
    # 与本机训练产物哈希一致（许可确认前不随包，仅登记校验值）
    onnx = Path(__file__).resolve().parents[2] / "m0-spike" / "yolo11n_ppe.onnx"
    if onnx.is_file():
        assert hashlib.sha256(onnx.read_bytes()).hexdigest() == \
            assets["yolo11n_ppe.onnx"]["sha256"]


def test_status_person_only_without_install(tmp_path, monkeypatch):
    from app.config import settings
    monkeypatch.setattr(settings, "data_dir", tmp_path / "data")
    monkeypatch.setattr(settings, "ppe_model", None)
    st = ppe._status(tmp_path / "models" / "yolo11n_ppe.onnx",
                     tmp_path / "models" / "yolo11n_ppe.classes.txt")
    assert st["installed"] is False
    assert st["mode"] == "person_only"
    assert st["license_confirmed_required"] is True
    assert "Roboflow" in st["license_notice"]


def test_sha256_ok_rejects_tampered(tmp_path):
    spec = _lock()["yolo11n_ppe.classes.txt"]
    p = tmp_path / "yolo11n_ppe.classes.txt"
    p.write_bytes(spec["size"] * b"x")
    assert ppe._sha256_ok(p, spec) is False  # 大小相同但哈希不符


def test_api_requires_license_confirm(client):
    r = client.post("/api/models/ppe/download", json={"confirm_license": False})
    assert r.status_code == 400
    r = client.post("/api/models/ppe/download", json={})
    assert r.status_code == 422  # Pydantic extra=forbid/必填


def test_build_portable_does_not_bundle_ppe():
    src = (Path(__file__).resolve().parents[2] / "scripts" /
           "build_portable.py").read_text(encoding="utf-8")
    assert "yolo11n_ppe.onnx" not in src.split("MODELS =")[1].split("\n")[0]
    assert "LS_PPE_MODEL=../../models/yolo11n_ppe.onnx" not in src


def test_sources_md_template_exists():
    p = Path(__file__).resolve().parents[2] / "Resources" / "效果演示" / "SOURCES.md"
    assert p.is_file()
    assert "CC0" in p.read_text(encoding="utf-8")


def test_fetch_assets_list():
    import subprocess
    r = subprocess.run([sys.executable, str(Path(__file__).resolve().parents[2] /
                                            "scripts" / "fetch_assets.py"), "--list"],
                       capture_output=True, text=True)
    assert r.returncode == 0
    assert "yolo11n_ppe.onnx" in r.stdout


def test_weekly_audit_workflow_exists():
    wf = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "security-weekly.yml"
    text = wf.read_text(encoding="utf-8")
    assert "cron:" in text and "issues: write" in text
    assert "SECURITY-EXCEPTIONS.md" in text


def test_auth_disabled_loopback_only():
    """Q-02 逃生口：仅回环放行；非回环即便 auth_disabled 也拒绝。"""
    from app.api import _auth_off
    from app.config import settings
    old_flag, old_host = settings.auth_disabled, os.environ.get("LS_HOST")
    try:
        settings.auth_disabled = True
        os.environ["LS_HOST"] = "127.0.0.1"
        assert _auth_off() is True
        os.environ["LS_HOST"] = "0.0.0.0"
        assert _auth_off() is False
        settings.auth_disabled = False
        assert _auth_off() is False
    finally:
        settings.auth_disabled = old_flag
        if old_host is None:
            os.environ.pop("LS_HOST", None)
        else:
            os.environ["LS_HOST"] = old_host


def test_validate_forces_read_protection_on_lan(monkeypatch):
    """Q-03：非回环绑定强制开启读保护（启动校验写入 settings）。"""
    from app.config import settings
    from app import validate as v
    monkeypatch.setenv("LS_HOST", "192.168.1.10")
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "admin_token", "test-token")
    res = v.validate_settings()
    assert settings.auth_read_protected is True
    assert any(r["key"] == "auth_read_protected" for r in res)
