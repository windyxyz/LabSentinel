"""单元测试：任务94 Prometheus 指标端点（字段/无敏感标签/鉴权）。"""
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


def test_metrics_200_and_content_type(client):
    r = client.get("/api/metrics")
    assert r.status_code == 200
    assert "text/plain" in r.headers["content-type"]


def test_metrics_core_lines(client):
    text = client.get("/api/metrics").text
    for key in ("labsentinel_up 1", "labsentinel_uptime_seconds",
                "labsentinel_cameras_total", "labsentinel_disk_free_bytes",
                "labsentinel_schema_version"):
        assert key in text, key


def test_metrics_no_sensitive_labels(client):
    text = client.get("/api/metrics").text
    assert "rtsp://" not in text
    assert "password" not in text.lower()


def test_metrics_with_camera(client, monkeypatch):
    import app.api as api_mod
    from app.store import Store
    s = Store(settings.db_path)
    cid = s.add_camera("m94", "x.mp4", "file")

    class FW:
        analysis_fps_now = 25.0
        last_frame_ts = None
        frame_age_s = None
        last_error = None
        last_error_ts = None
        reconnect_count = 0
        opened_at = None
        frames_total = 0

        def is_alive(self):
            return True

        def stop(self):
            pass  # lifespan teardown 调 stop_all() 时用

    try:
        with TestClient(app) as c:
            # lifespan 后设 FW（lifespan 会新建 manager）
            import app.api as api_mod
            api_mod.manager.workers[cid] = FW()
            text = c.get("/api/metrics").text
            assert f'labsentinel_camera_fps{{camera_id="{cid}"}}' in text
            assert "labsentinel_cameras_running 1" in text
    finally:
        api_mod.manager.workers.pop(cid, None)


def test_metrics_auth(client, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", True)
    monkeypatch.setattr(settings, "admin_token", "tok94")
    with TestClient(app) as c:
        assert c.get("/api/metrics").status_code == 401
        assert c.get("/api/metrics", headers={"X-Admin-Token": "tok94"}).status_code == 200
