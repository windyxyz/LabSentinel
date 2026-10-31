"""单元测试：任务95 启动配置校验（validate_settings/strict/端点计数）。"""
import os
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402
from app.validate import validate_settings, fail_count, warn_count  # noqa: E402


@pytest.fixture()
def client():
    with TestClient(app) as c:
        yield c


def test_fresh_dirs_all_ok(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "data_dir", tmp_path / "data")
    monkeypatch.setattr(settings, "snapshot_dir", tmp_path / "data" / "snapshots")
    monkeypatch.setattr(settings, "video_dir", tmp_path / "data" / "videos")
    monkeypatch.setattr(settings, "annotation_pool_dir", tmp_path / "data" / "pool")
    monkeypatch.setattr(settings, "db_path", tmp_path / "data" / "s.db")
    monkeypatch.setattr(settings, "admin_token", "")
    monkeypatch.setattr(settings, "read_token", "")
    model = tmp_path / "yolo11n_ppe.onnx"
    model.write_bytes(b"0" * 2_000_000)
    monkeypatch.setattr(settings, "ppe_model", model)
    (model.parent / "yolo11n_ppe.classes.txt").write_text("person\nlab coat\n", encoding="utf-8")
    r = validate_settings()
    assert fail_count(r) == 0
    ok_keys = {x["key"] for x in r if x["level"] == "OK"}
    assert "data_dir" in ok_keys and "ppe_model" in ok_keys


def test_missing_model_is_fail(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "data_dir", tmp_path / "data")
    monkeypatch.setattr(settings, "snapshot_dir", tmp_path / "data" / "s")
    monkeypatch.setattr(settings, "video_dir", tmp_path / "data" / "v")
    monkeypatch.setattr(settings, "annotation_pool_dir", tmp_path / "data" / "p")
    monkeypatch.setattr(settings, "db_path", tmp_path / "data" / "s.db")
    monkeypatch.setattr(settings, "admin_token", "")
    monkeypatch.setattr(settings, "read_token", "")
    monkeypatch.setattr(settings, "ppe_model", tmp_path / "nope.onnx")
    r = validate_settings()
    fails = [x for x in r if x["level"] == "FAIL" and x["key"] == "ppe_model"]
    assert fails and "不存在" in fails[0]["message"]


def test_token_warnings(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "data_dir", tmp_path / "data")
    monkeypatch.setattr(settings, "snapshot_dir", tmp_path / "data" / "s")
    monkeypatch.setattr(settings, "video_dir", tmp_path / "data" / "v")
    monkeypatch.setattr(settings, "annotation_pool_dir", tmp_path / "data" / "p")
    monkeypatch.setattr(settings, "db_path", tmp_path / "data" / "s.db")
    monkeypatch.setattr(settings, "admin_token", "short")
    monkeypatch.setattr(settings, "read_token", "short")
    r = validate_settings()
    warns = [x for x in r if x["level"] == "WARN"]
    assert any("admin_token" in x["key"] and "长度" in x["message"] for x in warns)
    assert any("read_token" in x["key"] and "相同" in x["message"] for x in warns)


def test_config_validate_endpoint(client, monkeypatch):
    monkeypatch.setattr(settings, "admin_token", "tok95")
    with TestClient(app) as c:
        r = c.get("/api/config/validate", headers={"X-Admin-Token": "tok95"})
        assert r.status_code == 200
        d = r.json()
        assert "items" in d and "fail" in d and "warn" in d


def test_system_config_issues_count(client):
    with TestClient(app) as c:
        d = c.get("/api/system").json()
        assert "config_issues" in d
        assert "fail" in d["config_issues"] and "warn" in d["config_issues"]
