"""单元测试：任务65 摄像头可观测性（manager.status 字段 / 单路端点 / 错误脱敏）。
运行: pytest tests/test_camera_status.py -q"""
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402
from app.pipeline import redact_creds  # noqa: E402


class FakeWorker:
    def __init__(self, last_frame_ts=None, last_error=None, reconnect=2, frames=123, alive=True):
        self.analysis_fps_now = 24.6
        self.last_frame_ts = last_frame_ts
        self.last_error = last_error
        self.last_error_ts = time.time() if last_error else None
        self.reconnect_count = reconnect
        self.opened_at = time.time() - 60
        self.frames_total = frames
        self._alive = alive

    def is_alive(self):
        return self._alive


def test_redact_creds():
    out = redact_creds("cannot open rtsp://admin:pw123@10.0.0.8:554/ch1 ... " + "x" * 300)
    assert "admin" not in out and "pw123" not in out
    assert "***:***@10.0.0.8:554" in out
    assert len(out) <= 200  # 截断
    assert redact_creds("普通错误") == "普通错误"


def test_manager_status_fields(tmp_path, monkeypatch):
    from app.pipeline import CameraManager
    import app.store as store_mod

    monkeypatch.setattr(store_mod.settings, "db_path", tmp_path / "s.db")
    monkeypatch.setattr(store_mod.settings, "data_dir", tmp_path)
    m = CameraManager(store_mod.Store(tmp_path / "s.db"))
    fixed = time.time() - 3.0
    m.workers[7] = FakeWorker(last_frame_ts=fixed, last_error="读帧中断，重连中")
    d = m.status()["cameras"][0]
    assert d["id"] == 7 and d["fps"] == 24.6
    assert abs(d["frame_age_s"] - 3.0) < 1.5
    assert d["last_error"] == "读帧中断，重连中"
    assert d["reconnect_count"] == 2 and d["frames_total"] == 123
    assert d["opened_at"] is not None and d["last_frame_ts"] == fixed


def test_status_endpoint_200_404_auth(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "db_path", tmp_path / "s.db")
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "admin_token", "")
    with TestClient(app) as c:
        cid = c.post("/api/cameras", json={"name": "cam65", "source": "x.mp4",
                                           "type": "file", "autostart": False}).json()["id"]
        r = c.get(f"/api/cameras/{cid}/status")
        assert r.status_code == 200
        d = r.json()
        assert d["name"] == "cam65" and d["type"] == "file" and d["status"] == "stopped"
        assert d["frames_total"] == 0 and d["frame_age_s"] is None  # 未启动过 worker → 仍有字段
        assert c.get("/api/cameras/9999/status").status_code == 404
    monkeypatch.setattr(settings, "auth_read_protected", True)
    monkeypatch.setattr(settings, "admin_token", "tok65")
    with TestClient(app) as c:
        assert c.get("/api/cameras/1/status").status_code == 401
        assert c.get("/api/cameras/1/status", headers={"X-Admin-Token": "tok65"}).status_code in (200, 404)


def test_system_route_has_new_fields(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "db_path", tmp_path / "s.db")
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    with TestClient(app) as c:
        import app.api as api_mod
        api_mod.manager.workers[3] = FakeWorker(last_frame_ts=time.time() - 0.5, frames=9)
        try:
            d = c.get("/api/system").json()
            cam = next(x for x in d["cameras"] if x["id"] == 3)
            for k in ("last_frame_ts", "frame_age_s", "last_error", "last_error_ts",
                      "reconnect_count", "opened_at", "frames_total"):
                assert k in cam
            assert cam["frames_total"] == 9 and cam["frame_age_s"] < 5
        finally:
            api_mod.manager.workers.pop(3, None)
