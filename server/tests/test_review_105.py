"""单元测试：任务105 补丁 P0-2（读保护 ?token= 回退）+ P1-1（跨摄像头区域规则 400）。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402


@pytest.fixture()
def protected_client(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", True)
    monkeypatch.setattr(settings, "admin_token", "tok-105")
    with TestClient(app) as c:
        yield c


def test_read_protected_header_and_query(protected_client):
    """header 与 ?token= 两种方式均 200；错误/缺失 token 401（105 补丁 P0-2）。"""
    r = protected_client.get("/api/cameras", headers={"X-Admin-Token": "tok-105"})
    assert r.status_code == 200
    r2 = protected_client.get("/api/cameras", params={"token": "tok-105"})
    assert r2.status_code == 200  # <img>/<a download> 裸 URL 的回退通道
    assert protected_client.get("/api/cameras").status_code == 401
    assert protected_client.get("/api/cameras", params={"token": "wrong"}).status_code == 401


def test_media_endpoints_query_token(protected_client):
    """媒体端点（快照/导出）在 query token 下可用——前端 mediaUrl 的目标路径。"""
    cid = protected_client.post(
        "/api/cameras", headers={"X-Admin-Token": "tok-105"},
        json={"name": "c105", "type": "file", "source": "x.mp4", "autostart": False}).json()["id"]
    assert protected_client.get(f"/api/cameras/{cid}/snapshot",
                                params={"token": "tok-105"}).status_code in (200, 404)
    r = protected_client.get("/api/export/events.csv", params={"token": "tok-105", "days": 1})
    assert r.status_code == 200


def test_cross_camera_zone_rule_rejected(protected_client):
    """P1-1：规则不能挂别的摄像头的区域——400 而非静默失活。"""
    h = {"X-Admin-Token": "tok-105"}
    c1 = protected_client.post("/api/cameras", headers=h,
                               json={"name": "c1", "type": "file", "source": "a.mp4",
                                     "autostart": False}).json()["id"]
    c2 = protected_client.post("/api/cameras", headers=h,
                               json={"name": "c2", "type": "file", "source": "b.mp4",
                                     "autostart": False}).json()["id"]
    z1 = protected_client.post("/api/zones", headers=h,
                               json={"camera_id": c1, "name": "z",
                                     "polygon": [[0, 0], [100, 0], [100, 100]]}).json()["id"]
    r = protected_client.post("/api/rules", headers=h,
                              json={"camera_id": c2, "zone_id": z1, "behavior": "enter"})
    assert r.status_code == 400
    assert "不能挂到摄像头" in r.json()["detail"]
    # 同摄像头正常
    ok = protected_client.post("/api/rules", headers=h,
                               json={"camera_id": c1, "zone_id": z1, "behavior": "enter"})
    assert ok.status_code == 200
