"""单元测试：任务84 事件关键词搜索（LIKE 转义/组合过滤/分页/400 边界）。"""
import os
import sys

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


@pytest.fixture()
def seeded(client):
    s = Store(settings.db_path)
    s.add_event(1, "enter_no_ppe", 3, 1, "有人未穿实验服进入试剂区")
    s.add_event(1, "enter", 2, 2, "正常进入试剂区")
    s.add_event(2, "exit", 1, 3, "离开 reagent 区")
    return s


def test_q_matches_detail_and_type(client, seeded):
    r = client.get("/api/events", params={"q": "实验服"})
    assert [e["detail"] for e in r.json()] == ["有人未穿实验服进入试剂区"]
    r2 = client.get("/api/events", params={"q": "enter"})  # type 匹配
    assert {e["type"] for e in r2.json()} == {"enter", "enter_no_ppe"}
    # count 一致
    c1 = client.get("/api/events/count", params={"q": "实验服"}).json()["count"]
    assert c1 == 1


def test_q_wildcards_literal(client, seeded):
    """q=% / _ 按字面匹配：% 不再是通配符；_ 字面匹配 type enter_no_ppe 的下划线。"""
    assert client.get("/api/events", params={"q": "%"}).json() == []
    r = client.get("/api/events", params={"q": "_"}).json()
    assert [e["type"] for e in r] == ["enter_no_ppe"]  # 字面下划线命中 type
    assert client.get("/api/events/count", params={"q": "%"}).json()["count"] == 0


def test_q_blank_is_noop(client, seeded):
    assert len(client.get("/api/events", params={"q": "   "}).json()) == 3
    assert client.get("/api/events/count", params={"q": ""}).json()["count"] == 3


def test_q_too_long_400(client, seeded):
    r = client.get("/api/events", params={"q": "x" * 101})
    assert r.status_code == 400
    assert client.get("/api/events/count", params={"q": "x" * 101}).status_code == 400


def test_q_combines_with_filters_and_paging(client, seeded):
    s = Store(settings.db_path)
    s.add_event(1, "enter", 2, 9, "又一次正常进入试剂区")
    # q=试剂 + camera=1 → 3 条（两条原 试剂 + 测试自加的一条；exit 在 camera 2 且 detail 无"试剂"）
    r = client.get("/api/events", params={"q": "试剂", "camera_id": 1}).json()
    assert len(r) == 3
    # 组合 + 分页：limit=1 offset 递进
    r1 = client.get("/api/events", params={"q": "试剂", "camera_id": 1, "limit": 1, "offset": 0}).json()
    r2 = client.get("/api/events", params={"q": "试剂", "camera_id": 1, "limit": 1, "offset": 1}).json()
    assert r1[0]["id"] != r2[0]["id"]
    total = client.get("/api/events/count", params={"q": "试剂", "camera_id": 1}).json()["count"]
    assert total == 3
