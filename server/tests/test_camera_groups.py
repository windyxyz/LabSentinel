"""单元测试：任务111 摄像头分组与批量启停（迁移 v7 / 分组统计 / bulk / 鉴权）。
运行: pytest tests/test_camera_groups.py -q"""
import sqlite3

import pytest
from fastapi.testclient import TestClient

from app import api as api_mod
from app.main import app
from app.store import Store


def test_migration_v7_on_old_db(tmp_path):
    """旧库（无 group_name 列）迁移后：列在、默认空、旧数据保留。"""
    db = tmp_path / "old.db"
    con = sqlite3.connect(str(db))
    con.executescript("""
      CREATE TABLE cameras(id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, source TEXT,
        type TEXT, status TEXT DEFAULT 'stopped');
      INSERT INTO cameras(name, source, type) VALUES('旧摄像头', 'D:/a.mp4', 'file');
    """)
    con.commit()
    con.close()
    st = Store(db)
    assert st.schema_version() >= 7
    cam = st.cameras()[0]
    assert cam["name"] == "旧摄像头" and cam["group_name"] == ""  # 旧数据保留+默认未分组


def test_group_crud_and_stats(tmp_path):
    st = Store(tmp_path / "s.db")
    a = st.add_camera("A", "D:/a.mp4", "file", group_name="二楼")
    b = st.add_camera("B", "D:/b.mp4", "file", group_name="二楼")
    c = st.add_camera("C", "D:/c.mp4", "file")  # 未分组
    st.set_camera_status(a, "running")
    st.set_camera_status(b, "running")
    groups = {g["name"]: g for g in st.camera_groups()}
    assert groups["二楼"]["count"] == 2 and groups["二楼"]["running"] == 2
    assert groups[""]["count"] == 1 and groups[""]["running"] == 0
    # 改组/清组
    assert st.set_camera_group(c, "三楼") is True
    assert st.set_camera_group(999, "x") is False  # 不存在的摄像头
    groups = {g["name"]: g for g in st.camera_groups()}
    assert "三楼" in groups and groups[""]["count"] == 0 if "" in groups else True
    assert st.set_camera_group(c, "") is True
    assert {g["name"] for g in st.camera_groups()} == {"二楼", ""}


# ---- API ----

@pytest.fixture()
def client():
    with TestClient(app) as tc:
        yield tc


def _mkcam(client, name, group=""):
    body = {"name": name, "source": "D:/x.mp4", "type": "file", "autostart": False}
    if group:
        body["group_name"] = group
    return client.post("/api/cameras", json=body).json()


def test_add_camera_with_group_and_readback(client):
    cam = _mkcam(client, "g-cam", group="化学楼")
    assert cam["group_name"] == "化学楼"
    lst = client.get("/api/cameras").json()
    assert [c for c in lst if c["id"] == cam["id"]][0]["group_name"] == "化学楼"
    groups = client.get("/api/camera-groups").json()["groups"]
    g = [g for g in groups if g["name"] == "化学楼"][0]
    assert g["count"] == 1 and g["running"] == 0


def test_put_group_and_validation(client):
    cam = _mkcam(client, "g2")
    r = client.put(f"/api/cameras/{cam['id']}/group", json={"group_name": "新区"})
    assert r.status_code == 200 and r.json()["group_name"] == "新区"
    # 清空=未分组
    r = client.put(f"/api/cameras/{cam['id']}/group", json={"group_name": ""})
    assert r.json()["group_name"] == ""
    # 长度校验
    assert client.put(f"/api/cameras/{cam['id']}/group",
                      json={"group_name": "x" * 33}).status_code == 400
    assert client.put("/api/cameras/99999/group", json={"group_name": "x"}).status_code == 404


def test_bulk_start_stop_with_manager_mock(client, monkeypatch):
    c1 = _mkcam(client, "b1")["id"]
    c2 = _mkcam(client, "b2")["id"]
    calls = []

    class FakeMgr:
        def start_camera(self, cam):
            calls.append(("start", cam["id"]))

        def stop_camera(self, cid):
            calls.append(("stop", cid))

    monkeypatch.setattr(api_mod, "manager", FakeMgr())
    r = client.post("/api/cameras/bulk", json={"ids": [c1, c2], "action": "start"})
    assert r.status_code == 200
    assert r.json()["ok"] == [c1, c2] and r.json()["missing"] == [] and r.json()["failed"] == []
    assert calls == [("start", c1), ("start", c2)]
    r = client.post("/api/cameras/bulk", json={"ids": [c2, 99999], "action": "stop"})
    body = r.json()
    assert body["ok"] == [c2] and body["missing"] == [99999]
    assert calls[-1] == ("stop", c2)


def test_bulk_validation(client):
    assert client.post("/api/cameras/bulk", json={"ids": [1], "action": "x"}).status_code == 400
    assert client.post("/api/cameras/bulk", json={"ids": [], "action": "start"}).status_code == 400
    assert client.post("/api/cameras/bulk",
                       json={"ids": [1, 1], "action": "start"}).status_code == 400  # 去重
    assert client.post("/api/cameras/bulk",
                       json={"ids": list(range(101)), "action": "start"}).status_code == 400  # >100
    assert client.post("/api/cameras/bulk",
                       json={"ids": ["a"], "action": "start"}).status_code == 400  # 非整数
    assert client.post("/api/cameras/bulk",
                       json={"ids": [True], "action": "start"}).status_code == 400  # bool 冒充 int


def test_group_auth(client, monkeypatch):
    monkeypatch.setattr(api_mod.settings, "admin_token", "sec")
    monkeypatch.setattr(api_mod.settings, "auth_read_protected", True)
    cam = _mkcam(client, "auth")  # 无令牌时 POST /cameras 也要 401
    assert client.get("/api/camera-groups").status_code == 401
    assert client.get("/api/camera-groups", headers={"X-Admin-Token": "sec"}).status_code == 200
    assert client.put(f"/api/cameras/1/group", json={"group_name": "x"}).status_code == 401
    assert client.post("/api/cameras/bulk", json={"ids": [1], "action": "start"}).status_code == 401
    monkeypatch.setattr(api_mod.settings, "admin_token", "")
    monkeypatch.setattr(api_mod.settings, "auth_read_protected", False)
