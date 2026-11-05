"""单元测试：任务87 操作审计（自动记录/查询/actor 判定/敏感不落库/retention 清理）。"""
import json
import os
import sqlite3
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


def _audit_rows(client, token=None):
    headers = {"X-Admin-Token": "tok87"} if token else {}
    r = client.get("/api/audit", params={"limit": 200}, headers=headers)
    return r.json()["items"]


def test_write_request_audited(client):
    r = client.post("/api/cameras", json={"name": "cam87", "type": "file",
                                          "source": "x.mp4", "autostart": False})
    rid = r.headers["x-request-id"]
    cid = r.json()["id"]
    items = _audit_rows(client)
    row = next(x for x in items if x["action"] == "POST /api/cameras")
    assert row["request_id"] == rid  # 与响应头一致
    assert row["actor"] == "anonymous(auth-off)"  # T-113：未启用令牌的匿名写入如实标注
    assert row["status_code"] == 200
    assert row["target"] is None  # 创建端点路径无 ID → target 为空
    # 带 ID 的路径：target 提取（删除该摄像头，产生 DELETE /api/cameras/{cid}）
    client.delete(f"/api/cameras/{cid}")
    items2 = _audit_rows(client)
    row2 = next(x for x in items2 if x["action"] == f"DELETE /api/cameras/{cid}")
    assert row2["target"] == f"camera:{cid}"
    # GET 不产生审计
    client.get("/api/cameras")
    items3 = _audit_rows(client)
    assert not [x for x in items3 if x["action"].startswith("GET")]


def test_unauthorized_write_also_audited(client, monkeypatch):
    monkeypatch.setattr(settings, "admin_token", "tok87")
    with TestClient(app) as c:
        r = c.post("/api/cameras", json={"name": "x", "source": "y", "type": "file"})
        assert r.status_code == 401
        items = _audit_rows(c, token="tok87")
        row = next(x for x in items if x["action"] == "POST /api/cameras")
        assert row["status_code"] == 401 and row["actor"] == "rejected"  # T-113：未授权写如实标注


def test_body_secrets_not_in_audit(client):
    secret = "SUPER-SECRET-PAYLOAD-87"
    client.post("/api/cameras", json={"name": secret, "type": "file",
                                      "source": "x.mp4", "autostart": False})
    rows = client.get("/api/audit", params={"limit": 200}).json()["items"]
    for row in rows:
        blob = json.dumps(row, ensure_ascii=False)
        assert secret not in blob  # body 绝不入审计
    assert all(set(x) <= {"id", "ts", "request_id", "actor", "action", "target",
                          "status_code", "detail"} for x in rows)


import json  # noqa: E402


def test_audit_query_filters_and_auth(client, monkeypatch):
    monkeypatch.setattr(settings, "admin_token", "tok87")
    with TestClient(app) as c:
        c.post("/api/cameras", json={"name": "a", "type": "file", "source": "x.mp4"},
               headers={"X-Admin-Token": "tok87"})
        # 无令牌 401
        assert c.get("/api/audit").status_code == 401
        # 分页/total/actor 筛选
        r = c.get("/api/audit", params={"limit": 1, "offset": 0, "actor": "admin"},
                  headers={"X-Admin-Token": "tok87"}).json()
        assert len(r["items"]) <= 1 and r["total"] >= 1
        r2 = c.get("/api/audit", params={"actor": "nobody"},
                   headers={"X-Admin-Token": "tok87"}).json()
        assert r2["total"] == 0


def test_retention_cleans_expired_audit(tmp_path, monkeypatch):
    from app import retention
    from datetime import datetime, timedelta
    s = Store(tmp_path / "s.db")
    old = (datetime.now() - timedelta(days=settings.retention_events_days + 5)).isoformat(timespec="seconds")
    with s._lock, s._conn() as c:
        c.execute("INSERT INTO audit_log(ts,request_id,actor,action,target,status_code) "
                  "VALUES(?,?,?,?,?,?)", (old, "r1", "admin", "POST /api/x", None, 200))
        c.execute("INSERT INTO audit_log(ts,request_id,actor,action,target,status_code) "
                  "VALUES(?,?,?,?,?,?)",
                  (datetime.now().isoformat(timespec="seconds"), "r2", "admin",
                   "POST /api/y", None, 200))
    con = sqlite3.connect(tmp_path / "s.db")
    n0 = con.execute("select count(*) from audit_log").fetchone()[0]
    con.close()
    assert n0 == 2
    monkeypatch.setattr(retention, "_disk_free_gb", lambda: 100.0)
    stats = retention.run_cleanup(s)
    assert stats["deleted_audit"] == 1
    con = sqlite3.connect(tmp_path / "s.db")
    n1 = con.execute("select count(*) from audit_log").fetchone()[0]
    con.close()
    assert n1 == 1


def test_preview_counts_audit_readonly(tmp_path, monkeypatch):
    from app import retention
    from datetime import datetime, timedelta
    s = Store(tmp_path / "s.db")
    old = (datetime.now() - timedelta(days=settings.retention_events_days + 5)).isoformat(timespec="seconds")
    with s._lock, s._conn() as c:
        c.execute("INSERT INTO audit_log(ts,request_id,actor,action,target,status_code) "
                  "VALUES(?,?,?,?,?,?)", (old, "r", "admin", "POST /api/x", None, 200))
    monkeypatch.setattr(retention, "_disk_free_gb", lambda: 100.0)
    p = retention.preview_cleanup(s)
    assert p["audit"]["count"] == 1
    con = sqlite3.connect(tmp_path / "s.db")
    n = con.execute("select count(*) from audit_log").fetchone()[0]
    con.close()
    assert n == 1  # 预演只读
