"""单元测试：任务82 未处置升级提醒（find_unhandled 选行/发送注入/batch/关闭/状态端点）。"""
import os
import sys
from datetime import datetime, timedelta

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import escalation  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402


@pytest.fixture()
def store(tmp_path, monkeypatch):
    s = Store(tmp_path / "s.db")
    old_ts = (datetime.now() - timedelta(minutes=30)).isoformat(timespec="seconds")
    new_ts = datetime.now().isoformat(timespec="seconds")
    # 1 条 L3 超时未处置 / 1 条 L2 超时 / 1 条 L3 已处置 / 1 条 L3 新
    s.add_event(1, "enter_no_ppe", 3, 1, "l3-old")
    s.add_event(1, "enter", 2, 2, "l2-old")
    acked = s.add_event(1, "enter_no_ppe", 3, 3, "acked")
    s.add_event(1, "enter", 3, 4, "l3-fresh")
    con = s._conn()
    con.execute("UPDATE events SET ts=?, status='new' WHERE detail IN ('l3-old','l2-old')", (old_ts,))
    con.execute("UPDATE events SET ts=?, status='acked' WHERE detail='acked'", (old_ts,))
    con.execute("UPDATE events SET ts=?, status='new' WHERE detail='l3-fresh'", (new_ts,))
    con.commit()
    con.close()
    yield s


def test_find_unhandled_selection(store, monkeypatch):
    monkeypatch.setattr(settings, "escalate_after_min", 15)
    rows = escalation.find_unhandled(store, datetime.now(), 15, 3, 20)
    assert [r["detail"] for r in rows] == ["l3-old"]  # 只选 L3 超时未处置未提醒


def test_escalate_once_send_injection_and_mark(store, monkeypatch):
    monkeypatch.setattr(settings, "escalate_after_min", 15)
    sends = []
    r = escalation.escalate_once(store, datetime.now(), send=lambda ev, text: sends.append((ev["id"], text)))
    assert r == {"checked": 1, "escalated": 1}
    assert len(sends) == 1 and "⏰ 未处置提醒" in sends[0][1] and "l3-old" in sends[0][1]
    # escalated_ts 已写入；再跑不重复（105 空帧教训：状态必须清）
    assert escalation.escalate_once(store, datetime.now())["checked"] == 0
    con = store._conn()
    row = con.execute("select escalated_ts from events where detail='l3-old'").fetchone()
    con.close()
    assert row[0]


def test_escalate_disabled_and_batch(store, monkeypatch):
    monkeypatch.setattr(settings, "escalate_after_min", 0)
    assert escalation.escalate_once(store, datetime.now())["disabled"] is True
    # batch 上限
    monkeypatch.setattr(settings, "escalate_after_min", 15)
    monkeypatch.setattr(settings, "escalate_batch", 1)
    monkeypatch.setattr(settings, "escalate_min_level", 2)
    r = escalation.escalate_once(store, datetime.now(), send=lambda ev, t: True)
    assert r["checked"] == 1  # 多条命中也只处理 batch 条


def test_channels_unconfigured_no_delivery(store, monkeypatch):
    monkeypatch.setattr(settings, "escalate_after_min", 15)
    r = escalation.escalate_once(store, datetime.now())  # dispatcher 渠道全未配置
    assert r["escalated"] == 1  # 仍标记（防重试风暴）


def test_status_endpoint(store, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "admin_token", "")
    monkeypatch.setattr(settings, "escalate_after_min", 15)
    with TestClient(app) as c:
        d = c.get("/api/escalation/status").json()
        for k in ("enabled", "after_min", "min_level", "pending", "escalated_today", "last"):
            assert k in d
        assert d["enabled"] is True and d["min_level"] == 3
