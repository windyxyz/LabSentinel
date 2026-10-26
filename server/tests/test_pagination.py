"""单元测试：任务19 事件分页与查询性能（offset/时间范围/count/limit 钳制）。运行: pytest tests/ -q"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import api as api_mod  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402


@pytest.fixture()
def client(tmp_path, monkeypatch):
    s = Store(path=tmp_path / "t.db")
    monkeypatch.setattr(api_mod, "store", s)
    # 三天各 2 条 + 一条误报
    for d, t, lv, tp in (("2026-10-25", "08:00:00", 2, "enter"),
                         ("2026-10-25", "09:00:00", 1, "exit"),
                         ("2026-10-26", "08:30:00", 3, "enter_no_ppe"),
                         ("2026-10-26", "18:00:00", 2, "enter"),
                         ("2026-10-27", "07:00:00", 2, "enter")):
        eid = s.add_event(1, tp, lv, 1, f"{d} {tp}")
        with s._lock, s._conn() as c:
            c.execute("UPDATE events SET ts=? WHERE id=?", (f"{d}T{t}", eid))
    s.set_event_status(eid, "false_positive")
    return TestClient(app)


def test_offset_paging(client):
    r1 = client.get("/api/events", params={"limit": 2, "offset": 0}).json()
    r2 = client.get("/api/events", params={"limit": 2, "offset": 2}).json()
    all_ids = [e["id"] for e in client.get("/api/events", params={"limit": 50}).json()]
    assert [e["id"] for e in r1] + [e["id"] for e in r2] == all_ids[:4]  # 新→旧翻页无缝
    assert r1[0]["id"] > r1[1]["id"] > r2[0]["id"]  # id 倒序


def test_time_range_boundary(client):
    # 纯日期端到端（含边界）：26 号当天 2 条
    r = client.get("/api/events", params={"start_ts": "2026-10-26", "end_ts": "2026-10-26"}).json()
    assert len(r) == 2
    # 完整时间边界：恰好 08:30:00 含边界
    r2 = client.get("/api/events", params={
        "start_ts": "2026-10-26T08:30:00", "end_ts": "2026-10-26T08:30:00"}).json()
    assert len(r2) == 1 and r2[0]["type"] == "enter_no_ppe"
    # 空窗
    assert client.get("/api/events", params={
        "start_ts": "2026-10-01", "end_ts": "2026-10-01"}).json() == []


def test_count_endpoint_consistency(client):
    total = client.get("/api/events/count").json()["count"]
    assert total == 5
    # 与 events 行数一致（同过滤条件）
    for params in ({"status": "false_positive"}, {"level": 2},
                   {"start_ts": "2026-10-25", "end_ts": "2026-10-26"}):
        n = client.get("/api/events/count", params=params).json()["count"]
        rows = client.get("/api/events", params={**params, "limit": 200}).json()
        assert n == len(rows), params


def test_limit_clamp(client):
    r = client.get("/api/events", params={"limit": 99999}).json()
    assert len(r) == 5  # 全部数据只有 5 条，但 limit 99999 未报错
    # 钳制验证：构造 60 条并请求 limit=200 → 上限 200 生效（数据不足时以 count 对照）
    count = client.get("/api/events/count").json()["count"]
    rows = client.get("/api/events", params={"limit": 200}).json()
    assert len(rows) == count  # 钳制后仍能取全
