"""单元测试：任务116 规则模板库（迁移 v8 / CRUD 校验 / 套用跳重 / 鉴权）。
运行: pytest tests/test_rule_templates.py -q"""
import sqlite3

import pytest
from fastapi.testclient import TestClient

from app import api as api_mod
from app.main import app
from app.store import Store


def test_migration_v8_empty_table(tmp_path):
    """旧库迁移后模板表为空、规则不受影响。"""
    db = tmp_path / "old.db"
    con = sqlite3.connect(str(db))
    con.executescript("""
      CREATE TABLE rules(id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER,
        zone_id INTEGER, behavior TEXT, level INTEGER, cooldown_s INTEGER,
        time_start TEXT, time_end TEXT, threshold INTEGER, enabled INTEGER);
      INSERT INTO rules(camera_id,zone_id,behavior,level,cooldown_s,time_start,time_end,threshold,enabled)
        VALUES(1,NULL,'enter',2,60,'00:00','23:59',0,1);
    """)
    con.commit()
    con.close()
    st = Store(db)
    assert st.schema_version() >= 8
    assert st.rule_templates() == []
    assert len(st.rules(1)) == 1  # 旧规则完好


GOOD_RULES = [
    {"behavior": "enter_no_ppe", "level": 3, "cooldown_s": 60,
     "time_start": "08:00", "time_end": "18:00", "threshold": 0,
     "days_of_week": "1,2,3,4,5", "holiday_only": False},
    {"behavior": "occupancy_over", "level": 2, "cooldown_s": 30,
     "time_start": "00:00", "time_end": "23:59", "threshold": 5,
     "days_of_week": "1,2,3,4,5,6,7", "holiday_only": False},
]


@pytest.fixture()
def client():
    with TestClient(app) as tc:
        yield tc


def _mkcam(client, name):
    return client.post("/api/cameras", json={"name": name, "source": "D:/x.mp4",
                                             "type": "file", "autostart": False}).json()["id"]


def test_template_crud_and_validation(client):
    r = client.post("/api/rule-templates", json={"name": "实验室常规", "rules": GOOD_RULES})
    assert r.status_code == 200
    tid = r.json()["id"]
    assert r.json()["rules"][0]["behavior"] == "enter_no_ppe"  # 清洗后回显
    # 重名 409（定义：无 upsert，重名必须 409）
    assert client.post("/api/rule-templates",
                       json={"name": "实验室常规", "rules": GOOD_RULES}).status_code == 409
    # 列表
    lst = client.get("/api/rule-templates").json()["templates"]
    assert lst[0]["name"] == "实验室常规" and lst[0]["rules_count"] == 2
    # 校验矩阵
    bad = [
        {"name": "x1", "rules": []},                                   # 空
        {"name": "x2", "rules": [{"behavior": "foo"}]},                # behavior 非法
        {"name": "x3", "rules": [{"behavior": "enter", "level": 9}]},  # level 越界
        {"name": "x4", "rules": [{"behavior": "enter", "time_start": "8:0"}]},  # 时间格式
        {"name": "x5", "rules": [{"behavior": "enter", "days_of_week": "9"}]},  # 星期非法
        {"name": "x6", "rules": "nope"},                               # 非数组
    ]
    for b in bad:
        assert client.post("/api/rule-templates", json=b).status_code == 400, b
    # PUT 改名/冲突
    assert client.put(f"/api/rule-templates/{tid}",
                      json={"name": "常规 v2", "rules": GOOD_RULES[:1]}).json()["name"] == "常规 v2"
    t2 = client.post("/api/rule-templates", json={"name": "夜间", "rules": GOOD_RULES[:1]}).json()["id"]
    assert client.put(f"/api/rule-templates/{t2}",
                      json={"name": "常规 v2", "rules": GOOD_RULES[:1]}).status_code == 409
    assert client.put("/api/rule-templates/99999",
                      json={"name": "y", "rules": GOOD_RULES}).status_code == 404
    # DELETE
    assert client.delete(f"/api/rule-templates/{t2}").json() == {"ok": True}
    assert client.delete(f"/api/rule-templates/{t2}").status_code == 404


def test_apply_to_two_cameras(client):
    tid = client.post("/api/rule-templates",
                      json={"name": "双路套用", "rules": GOOD_RULES}).json()["id"]
    c1, c2 = _mkcam(client, "ap1"), _mkcam(client, "ap2")
    r = client.post(f"/api/rule-templates/{tid}/apply", json={"camera_ids": [c1, c2]})
    assert r.status_code == 200
    assert r.json() == {"created": 4, "skipped": 0, "cameras": 2, "rules_per_camera": 2}
    for cid in (c1, c2):
        rules = client.get(f"/api/rules?camera_id={cid}").json()
        assert len(rules) == 2
        assert all(x["camera_id"] == cid for x in rules)  # 模板不绑摄像头：按目标生成
        assert rules[0]["days_of_week"] == "1,2,3,4,5"    # 112 字段随模板带过
    # 再套一次：同 behavior+zone+时间已存在 → 全部跳过
    r2 = client.post(f"/api/rule-templates/{tid}/apply", json={"camera_ids": [c1, c2]})
    assert r2.json()["created"] == 0 and r2.json()["skipped"] == 4


def test_apply_zone_validation(client):
    tid = client.post("/api/rule-templates",
                      json={"name": "区域套用", "rules": GOOD_RULES[:1]}).json()["id"]
    c1, c2 = _mkcam(client, "z1"), _mkcam(client, "z2")
    z1 = client.post("/api/zones", json={"camera_id": c1, "name": "门",
                                         "polygon": [[0, 0], [100, 0], [100, 100]]}).json()
    # 区域属于别的摄像头 → 400
    r = client.post(f"/api/rule-templates/{tid}/apply",
                    json={"camera_ids": [c2], "zone_id": z1["id"]})
    assert r.status_code == 400 and "不属于" in r.json()["detail"]
    # 正确区域 → 规则挂到该区域
    r2 = client.post(f"/api/rule-templates/{tid}/apply",
                     json={"camera_ids": [c1], "zone_id": z1["id"]})
    assert r2.json()["created"] == 1
    rules = client.get(f"/api/rules?camera_id={c1}").json()
    assert rules[0]["zone_id"] == z1["id"]
    # 摄像头不存在 → 404
    assert client.post(f"/api/rule-templates/{tid}/apply",
                       json={"camera_ids": [99999]}).status_code == 404
    # 模板删除后套用 → 404
    client.delete(f"/api/rule-templates/{tid}")
    assert client.post(f"/api/rule-templates/{tid}/apply",
                       json={"camera_ids": [c1]}).status_code == 404


def test_template_auth(client, monkeypatch):
    monkeypatch.setattr(api_mod.settings, "admin_token", "sec")
    monkeypatch.setattr(api_mod.settings, "auth_read_protected", True)
    assert client.get("/api/rule-templates").status_code == 401
    assert client.get("/api/rule-templates",
                      headers={"X-Admin-Token": "sec"}).status_code == 200
    assert client.post("/api/rule-templates",
                       json={"name": "x", "rules": GOOD_RULES}).status_code == 401
    assert client.delete("/api/rule-templates/1").status_code == 401
    monkeypatch.setattr(api_mod.settings, "admin_token", "")
    monkeypatch.setattr(api_mod.settings, "auth_read_protected", False)
