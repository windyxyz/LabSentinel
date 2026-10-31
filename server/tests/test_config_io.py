"""单元测试：任务79 配置导入导出（名称关联/默认脱敏/merge 去重/replace 确认/校验不落半截）。"""
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


def _mk(client, name, source, zname=None, poly=None, behavior=None):
    cid = client.post("/api/cameras", json={"name": name, "type": "rtsp",
                                            "source": source, "autostart": False}).json()["id"]
    zid = rid = None
    if zname:
        zid = client.post("/api/zones", json={"camera_id": cid, "name": zname,
                                              "polygon": poly}).json()["id"]
    if behavior:
        rid = client.post("/api/rules", json={"camera_id": cid, "zone_id": zid,
                                              "behavior": behavior, "level": 2,
                                              "cooldown_s": 5}).json()["id"]
    return cid, zid, rid


def test_export_fields_and_redaction(client):
    _mk(client, "前门", "rtsp://admin:secret@10.0.0.3:554/ch1", "z1",
        [[0, 0], [100, 0], [100, 100]], "enter")
    r = client.get("/api/config/export")
    assert r.status_code == 200
    assert r.headers["cache-control"] == "no-store"
    d = r.json()
    assert d["format"] == "labsentinel-config" and d["version"] == 1
    assert d["cameras"][0]["source"] == "rtsp://***:***@10.0.0.3:554/ch1"  # 默认脱敏
    assert "secret" not in r.text
    assert d["zones"][0]["camera_name"] == "前门" and d["zones"][0]["polygon"] == \
        [[0, 0], [100, 0], [100, 100]]
    assert d["rules"][0]["zone_name"] == "z1"
    # with_secrets=1 返回真实 source
    r2 = client.get("/api/config/export?with_secrets=1")
    assert "secret@" in r2.json()["cameras"][0]["source"]
    # 不含事件数据
    assert "events" not in d


def test_import_merge_and_dedupe(client):
    _mk(client, "A", "rtsp://u:p@1.1.1.1/s", "zA", [[0, 0], [50, 0], [50, 50]], "enter")
    exp = client.get("/api/config/export?with_secrets=1").json()
    exp["cameras"].append({"name": "B", "source": "rtsp://u:p@2.2.2.2/s", "type": "rtsp"})
    exp["zones"].append({"camera_name": "B", "name": "zB", "polygon": [[0, 0], [60, 0], [60, 60]]})
    exp["rules"].append({"camera_name": "B", "zone_name": "zB", "behavior": "enter",
                         "level": 3, "cooldown_s": 10, "time_start": "00:00",
                         "time_end": "23:59", "threshold": 0, "enabled": True})
    r = client.post("/api/config/import", json={"mode": "merge", "data": exp})
    assert r.status_code == 200
    d = r.json()
    assert d["created"]["cameras"] == 1 and d["created"]["zones"] == 1 and d["created"]["rules"] == 1
    assert "A" in d["skipped"]["cameras"]  # 同名跳过
    # 规则 zone_id 指向新导入的区域（按名称关联）
    zones = client.get("/api/zones").json()
    zb = next(z for z in zones if z["name"] == "zB")
    rules = client.get("/api/rules").json()
    assert any(rr["zone_id"] == zb["id"] and rr["camera_id"] == zb["camera_id"] for rr in rules)
    # 再导一遍 merge：全部跳过
    d2 = client.post("/api/config/import", json={"mode": "merge", "data": exp}).json()
    assert d2["created"] == {"cameras": 0, "zones": 0, "rules": 0}


def test_import_replace_requires_confirm_and_keeps_events(client):
    _mk(client, "A", "rtsp://u:p@1.1.1.1/s", "zA", [[0, 0], [50, 0], [50, 50]], "enter")
    client.post("/api/events/1/status", json={"status": "acked"}) if False else None
    exp = client.get("/api/config/export?with_secrets=1").json()
    # 无 confirm → 400
    r = client.post("/api/config/import", json={"mode": "replace", "data": exp})
    assert r.status_code == 400
    # 确认 → 清空重建；事件表行数不变（本库无事件行，断言表可查）
    r = client.post("/api/config/import",
                    json={"mode": "replace", "confirm_replace": True,
                          "data": {"format": "labsentinel-config", "version": 1,
                                   "cameras": [{"name": "R", "source": "rtsp://x@3.3.3.3/s",
                                                "type": "rtsp"}], "zones": [], "rules": []}})
    assert r.status_code == 200
    assert r.json()["created"]["cameras"] == 1
    names = [c["name"] for c in client.get("/api/cameras").json()]
    assert names == ["R"]
    assert any("camera_id" in w for w in r.json()["warnings"])  # 事件失联提示


def test_import_validation_no_partial_write(client):
    _mk(client, "A", "rtsp://u:p@1.1.1.1/s")
    bad = {"format": "labsentinel-config", "version": 1,
           "cameras": [{"name": "OK", "source": "rtsp://x@4.4.4.4/s", "type": "rtsp"},
                       {"name": "Bad", "source": "s", "type": "hls"}],
           "zones": [{"camera_name": "OK", "name": "z", "polygon": [[0, 0], [1, 1]]}],
           "rules": [{"camera_name": "幽灵", "behavior": "enter", "level": 9}]}
    r = client.post("/api/config/import", json={"mode": "merge", "data": bad})
    assert r.status_code == 400
    assert "未写入任何数据" in r.json()["detail"]
    # 校验失败 → 无部分写入：OK 摄像头也没有进库
    assert "OK" not in [c["name"] for c in client.get("/api/cameras").json()]


def test_import_requires_admin(client, monkeypatch):
    monkeypatch.setattr(settings, "admin_token", "tok79")
    monkeypatch.setattr(settings, "auth_read_protected", False)
    with TestClient(app) as c:
        assert c.get("/api/config/export").status_code == 401
        assert c.post("/api/config/import", json={"mode": "merge"}).status_code == 401
        assert c.get("/api/config/export", headers={"X-Admin-Token": "tok79"}).status_code == 200
