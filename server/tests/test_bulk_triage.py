"""单元测试：任务83 事件批量处置（updated/skipped/missing/failed、误报入池、校验、鉴权）。"""
import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402


@pytest.fixture()
def client(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "admin_token", "")
    with TestClient(app) as c:
        yield c


def _mk_events(n=3, with_snapshot=False):
    s = Store(settings.db_path)
    ids = []
    for i in range(n):
        snap = None
        if with_snapshot:
            settings.snapshot_dir.mkdir(parents=True, exist_ok=True)
            p = settings.snapshot_dir / f"bulk{i}.jpg"
            p.write_bytes(b"\xff\xd8fake")
            snap = str(p)
        ids.append(s.add_event(1, "enter", 2, i + 1, f"bulk{i}", snapshot=snap))
    return ids


def test_bulk_acked_updates_all(client):
    ids = _mk_events(3)
    r = client.post("/api/events/bulk-status",
                    json={"ids": ids, "status": "acked", "handled_by": "admin"})
    assert r.status_code == 200
    d = r.json()
    assert d["updated"] == 3 and d["skipped"] == 0 and d["missing"] == [] and d["failed"] == []
    s = Store(settings.db_path)
    for eid in ids:
        assert s.event_by_id(eid)["status"] == "acked"
        assert s.event_by_id(eid)["handled_by"] == "admin"
        assert s.event_by_id(eid)["handled_ts"]


def test_bulk_mixed_updated_skipped_missing(client):
    ids = _mk_events(2)
    client.post("/api/events/bulk-status", json={"ids": [ids[0]], "status": "acked"})
    # 单条端点语义=允许处置状态间再流转（acked→ignored 也会更新）——与单条保持一致
    r = client.post("/api/events/bulk-status",
                    json={"ids": [ids[0], ids[1], 99999], "status": "ignored"})
    d = r.json()
    assert d["updated"] == 2 and d["skipped"] == 0 and 99999 in d["missing"]
    s = Store(settings.db_path)
    assert s.event_by_id(ids[0])["status"] == "ignored"


def test_bulk_false_positive_pool_once(client, tmp_path, monkeypatch):
    pool = tmp_path / "pool"
    monkeypatch.setattr(settings, "annotation_pool_dir", pool)
    ids = _mk_events(1, with_snapshot=True)
    r1 = client.post("/api/events/bulk-status",
                     json={"ids": ids, "status": "false_positive"}).json()
    assert r1["updated"] == 1
    files = list(pool.glob("*.jpg"))
    assert len(files) == 1  # 截图入池
    r2 = client.post("/api/events/bulk-status",
                     json={"ids": ids, "status": "false_positive"}).json()
    assert r2["skipped"] == 1  # 已是目标状态 → skipped，不重复复制
    assert len(list(pool.glob("*.jpg"))) == 1


def test_bulk_validation(client):
    _mk_events(1)
    assert client.post("/api/events/bulk-status", json={"ids": [], "status": "acked"}).status_code == 400
    assert client.post("/api/events/bulk-status",
                       json={"ids": list(range(201)), "status": "acked"}).status_code == 400
    assert client.post("/api/events/bulk-status",
                       json={"ids": [1], "status": "new"}).status_code == 400
    assert client.post("/api/events/bulk-status",
                       json={"ids": [1], "status": "bogus"}).status_code == 400


def test_bulk_requires_admin(client, monkeypatch):
    monkeypatch.setattr(settings, "admin_token", "tok83")
    with TestClient(app) as c:
        assert c.post("/api/events/bulk-status",
                      json={"ids": [1], "status": "acked"}).status_code == 401
        assert c.post("/api/events/bulk-status", json={"ids": [1], "status": "acked"},
                      headers={"X-Admin-Token": "tok83"}).status_code in (200, 404)
