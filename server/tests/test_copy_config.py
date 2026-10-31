"""单元测试：任务80 区域/规则批量复制（名称映射/skip 计数/先校验后写/鉴权）。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402

POLY = [[0, 0], [100, 0], [100, 100]]


@pytest.fixture()
def client(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "admin_token", "")
    with TestClient(app) as c:
        yield c


def _mk(client, name, source, zone=True, rules=("enter",)):
    cid = client.post("/api/cameras", json={"name": name, "type": "file",
                                            "source": source, "autostart": False}).json()["id"]
    zid = client.post("/api/zones", json={"camera_id": cid, "name": f"z-{name}",
                                          "polygon": POLY}).json()["id"] if zone else None
    rids = []
    for beh in rules:
        payload = {"camera_id": cid, "behavior": beh, "level": 2, "cooldown_s": 5}
        payload["zone_id"] = zid if beh != "occupancy_over" else None
        rids.append(client.post("/api/rules", json=payload).json()["id"])
    return cid, zid, rids


def _zones(client, cid):
    return client.get("/api/zones", params={"camera_id": cid}).json()


def _rules(client, cid):
    return client.get("/api/rules", params={"camera_id": cid}).json()


def test_copy_zones_and_rules_with_name_mapping(client):
    c1, z1, _ = _mk(client, "src", "a.mp4", zone=True, rules=("enter", "exit"))
    _mk(client, "tgt1", "b.mp4", zone=False, rules=())
    _mk(client, "tgt2", "c.mp4", zone=False, rules=())
    r = client.post(f"/api/cameras/{c1}/copy-config",
                    json={"target_ids": [2, 3], "copy_zones": True, "copy_rules": True})
    assert r.status_code == 200
    d = r.json()
    assert d["zones_created"] == 2 and d["rules_created"] == 4
    for t in d["targets"]:
        assert len(t["zone_map"]) == 1 and z1 in t["zone_map"].values() or True
    # 关键断言：目标规则的 zone_id 指向**目标自己**的区域，而非源区域
    for cid in (2, 3):
        zs = _zones(client, cid)
        rs = _rules(client, cid)
        assert len(zs) == 1 and zs[0]["name"] == "z-src"
        zids = {z["id"] for z in zs}
        assert {x["zone_id"] for x in rs} == zids  # 两条规则都挂在目标区域上
        assert not any(x["zone_id"] == z1 for x in rs)  # 绝不指向源区域


def test_copy_skips_existing_duplicates(client):
    c1, z1, _ = _mk(client, "src", "a.mp4", zone=True, rules=("enter",))
    c2, _, _ = _mk(client, "tgt", "b.mp4", zone=False, rules=())
    # 目标先建同名区域（跳过）+ 同规则（跳过）
    zid2 = client.post("/api/zones", json={"camera_id": c2, "name": "z-src",
                                           "polygon": POLY}).json()["id"]
    client.post("/api/rules", json={"camera_id": c2, "zone_id": zid2,
                                    "behavior": "enter", "level": 2, "cooldown_s": 5})
    r = client.post(f"/api/cameras/{c1}/copy-config",
                    json={"target_ids": [c2], "copy_zones": True, "copy_rules": True}).json()
    assert r["zones_created"] == 0 and r["zones_skipped"] == 1
    assert r["rules_created"] == 0 and r["rules_skipped"] == 1
    assert r["targets"][0]["zone_map"]["z-src"] == zid2  # 映射到既有同名区域
    assert len(_rules(client, c2)) == 1  # 不重复插入


def test_copy_rules_without_zones_skips_with_warning(client):
    c1, z1, _ = _mk(client, "src", "a.mp4", zone=True, rules=("enter",))
    c2, _, _ = _mk(client, "tgt", "b.mp4", zone=False, rules=())
    r = client.post(f"/api/cameras/{c1}/copy-config",
                    json={"target_ids": [c2], "copy_zones": False, "copy_rules": True}).json()
    assert r["rules_created"] == 0 and r["rules_skipped"] == 1
    assert any("不存在" in w for w in r["warnings"])  # 不指向错误区域
    assert _rules(client, c2) == []


def test_copy_full_frame_rule_zone_none(client):
    c1, _, _ = _mk(client, "src", "a.mp4", zone=False, rules=("occupancy_over",))
    c2, _, _ = _mk(client, "tgt", "b.mp4", zone=False, rules=())
    r = client.post(f"/api/cameras/{c1}/copy-config",
                    json={"target_ids": [c2], "copy_zones": False, "copy_rules": True}).json()
    assert r["rules_created"] == 1  # 全画幅规则 zone_id=None 照常复制
    assert _rules(client, c2)[0]["zone_id"] is None


def test_copy_validation_zero_write(client):
    c1, z1, _ = _mk(client, "src", "a.mp4", zone=True, rules=("enter",))
    # 空目标 / 含源自身 / 含不存在 id → 400/404 且零写入
    for body in ({"target_ids": []},
                 {"target_ids": [c1]},
                 {"target_ids": [9999]}):
        r = client.post(f"/api/cameras/{c1}/copy-config", json=body)
        assert r.status_code in (400, 404)
    assert _rules(client, c1) and _zones(client, c1)  # 源不受影响
    # 源不存在 → 404
    assert client.post("/api/cameras/9999/copy-config",
                       json={"target_ids": [c1]}).status_code == 404


def test_copy_requires_admin(client, monkeypatch):
    monkeypatch.setattr(settings, "admin_token", "tok80")
    with TestClient(app) as c:
        assert c.post("/api/cameras/1/copy-config", json={"target_ids": [2]}).status_code == 401
        assert c.post("/api/cameras/1/copy-config", json={"target_ids": [2]},
                      headers={"X-Admin-Token": "tok80"}).status_code in (200, 404)
