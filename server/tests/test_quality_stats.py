"""单元测试：任务102 处置质量统计（/api/stats/quality：误报率口径/分组/过滤/边界）。
运行: pytest tests/test_quality_stats.py -q"""
from datetime import datetime, timedelta

import pytest
from fastapi.testclient import TestClient

from app import api as api_mod
from app.main import app


@pytest.fixture()
def client():
    # conftest 已把 settings 路径隔离到 tmp；lifespan init() 会把 api_mod.store 指向临时库，
    # 因此摄像头/事件一律经 api_mod.store 造（与端点同一连接源）。
    with TestClient(app) as tc:
        yield tc


def _cam(name):
    return api_mod.store.add_camera(name, "D:/x.mp4", "file")


def _add(st, cid, status, level=2, type_="enter", days_ago=0):
    # 不强制小时：now-days_ago 直接落窗口内（避免墙钟时点脆弱——凌晨跑测试 hour=10 会在未来）
    ts = (datetime.now() - timedelta(days=days_ago)).strftime("%Y-%m-%d %H:%M:%S")
    with st._conn() as c:
        cur = c.execute(
            "INSERT INTO events(camera_id,ts,type,level,detail,status,handled_by,handled_ts)"
            " VALUES(?,?,?,?,?,?,?,?)",
            (cid, ts, type_, level, "", status,
             "" if status == "new" else "admin",
             None if status == "new" else ts))
        return cur.lastrowid


def test_quality_aggregates(client):
    st = api_mod.store
    c1, c2 = _cam("化学一"), _cam("化学二")
    # cam1: 6 acked + 2 fp + 1 new；cam2: 1 fp + 1 ignored（跨 2 天）
    for i in range(6):
        _add(st, c1, "acked", days_ago=i % 2)
    _add(st, c1, "false_positive", days_ago=0)
    _add(st, c1, "false_positive", level=3, type_="occupancy_over", days_ago=1)
    _add(st, c1, "new", days_ago=0)
    _add(st, c2, "false_positive", days_ago=1)
    _add(st, c2, "ignored", days_ago=0)
    r = client.get("/api/stats/quality?days=7")
    assert r.status_code == 200
    q = r.json()
    assert q["total"] == 11
    assert q["by_status"] == {"acked": 6, "false_positive": 3, "new": 1, "ignored": 1}
    assert q["by_level"]["2"] == 10 and q["by_level"]["3"] == 1
    assert q["by_type"]["enter"] == 10 and q["by_type"]["occupancy_over"] == 1
    # 误报率 = 3 / (6+3+1) = 0.3（分母=已处置，不含 new）
    assert q["false_positive_rate"] == pytest.approx(0.3)
    assert q["unhandled"] == 1 and q["l3_total"] == 1
    by_cam = {c["camera_id"]: c for c in q["by_camera"]}
    assert by_cam[c1]["total"] == 9 and by_cam[c1]["false_positive"] == 2
    assert by_cam[c1]["rate"] == pytest.approx(round(2 / 9, 3))
    assert by_cam[c2]["total"] == 2 and by_cam[c2]["false_positive"] == 1
    assert by_cam[c1]["name"] == "化学一"
    assert sum(d["total"] for d in q["daily"]) == 11
    assert sum(d["false_positive"] for d in q["daily"]) == 3
    assert len(q["daily"]) == 2  # 覆盖两天
    assert q["days"] == 7 and q["start_ts"] < q["end_ts"]


def test_rate_new_only_and_empty(client):
    st = api_mod.store
    c1 = _cam("只有未处置")
    _add(st, c1, "new")  # 只有未处置：已处置分母 0 → rate=0
    q = client.get("/api/stats/quality?days=7").json()
    assert q["total"] == 1 and q["unhandled"] == 1
    assert q["false_positive_rate"] == 0.0
    # 完全无事件：零值/空数组
    q3 = client.get(f"/api/stats/quality?days=7&camera_id={c1 + 99}").json()
    assert q3["total"] == 0 and q3["by_camera"] == [] and q3["daily"] == []
    assert q3["false_positive_rate"] == 0.0 and q3["unhandled"] == 0


def test_days_validation_and_camera_filter(client):
    assert client.get("/api/stats/quality?days=0").status_code == 400
    assert client.get("/api/stats/quality?days=366").status_code == 400
    assert client.get("/api/stats/quality?days=365").status_code == 200
    st = api_mod.store
    c1, c2 = _cam("A"), _cam("B")
    _add(st, c1, "acked")
    _add(st, c2, "acked")
    q = client.get(f"/api/stats/quality?days=7&camera_id={c2}").json()
    assert q["total"] == 1 and len(q["by_camera"]) == 1
    assert q["by_camera"][0]["camera_id"] == c2


def test_auth_read_protected(client, monkeypatch):
    """读保护口径与其它 GET 一致：开启读保护后无令牌 401。"""
    monkeypatch.setattr(api_mod.settings, "admin_token", "sec")
    monkeypatch.setattr(api_mod.settings, "auth_read_protected", True)
    monkeypatch.setattr(api_mod.settings, "read_token", "ro")
    assert client.get("/api/stats/quality").status_code == 401
    assert client.get("/api/stats/quality",
                      headers={"X-Admin-Token": "sec"}).status_code == 200
    monkeypatch.setattr(api_mod.settings, "admin_token", "")
    monkeypatch.setattr(api_mod.settings, "auth_read_protected", False)


def test_existing_stats_endpoints_unaffected(client):
    st = api_mod.store
    c1 = _cam("回归")
    _add(st, c1, "acked", level=1)
    assert client.get("/api/stats/daily?days=7").status_code == 200
    assert client.get("/api/stats/summary?days=30").status_code == 200
    assert client.get("/api/stats/hourly",
                      params={"date": datetime.now().strftime("%Y-%m-%d")}).status_code == 200
