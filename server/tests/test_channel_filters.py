"""单元测试：任务86 告警渠道按级别/类型过滤（解析/派发过滤/status 回显/alert-test 不受影响）。"""
import json
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import alerts as al  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402


def test_parse_filters():
    monkeyfilters = json.dumps({"wecom": {"min_level": 3, "types": ["enter_no_ppe"]},
                                "email": {"min_level": 2}})
    settings.alert_filters = monkeyfilters
    f = al.alert_filters()
    assert f["wecom"] == {"min_level": 3, "types": ["enter_no_ppe"]}
    assert f["email"] == {"min_level": 2}
    # 非法 JSON → 空
    settings.alert_filters = "{bad json"
    assert al.alert_filters() == {}
    # 未知渠道忽略、非法级别忽略字段、空串=无
    settings.alert_filters = json.dumps({"nope": {"min_level": 3},
                                         "wecom": {"min_level": 9},
                                         "email": {"types": "not-a-list"}})
    assert al.alert_filters() == {}
    settings.alert_filters = ""
    assert al.alert_filters() == {}


def test_channel_filtered_pure():
    f = {"wecom": {"min_level": 3}, "email": {"types": ["enter_no_ppe"]}}
    ev = {"level": 2, "type": "enter"}
    assert al.channel_filtered("wecom", ev, f) is not None  # L2 < 3
    assert al.channel_filtered("wecom", {"level": 3, "type": "enter"}, f) is None
    assert al.channel_filtered("email", {"level": 3, "type": "exit"}, f) is not None
    assert al.channel_filtered("email", {"level": 3, "type": "enter_no_ppe"}, f) is None
    assert al.channel_filtered("dingtalk", ev, f) is None  # 未配置过滤的渠道不拦


class SpyCh:
    name = "wecom"

    def __init__(self):
        self.calls = []

    def configured(self):
        return True

    def send(self, level, text):
        self.calls.append((level, text))
        return True


def test_dispatch_filtered_no_delivery(tmp_path, monkeypatch):
    s = Store(tmp_path / "s.db")
    monkeypatch.setattr(settings, "alert_filters",
                        json.dumps({"wecom": {"min_level": 3}}))
    d = al.AlertDispatcher(store=s)
    spy = SpyCh()
    d.channels = [spy]
    ev_l2 = {"id": 1, "ts": "2026-10-31T12:00:00", "camera_id": 1, "type": "enter",
             "level": 2, "track_id": 1, "detail": "L2 事件"}
    ev_l3 = {"id": 2, "ts": "2026-10-31T12:01:00", "camera_id": 1, "type": "enter_no_ppe",
             "level": 3, "track_id": 2, "detail": "L3 事件"}
    d.dispatch(ev_l2)
    time.sleep(0.3)
    assert spy.calls == []  # L2 被过滤：零调用
    assert s.delivery_counts().get("wecom", {"ok": 0})["ok"] == 0  # 无投递行
    d.dispatch(ev_l3)
    time.sleep(0.3)
    assert len(spy.calls) == 1 and spy.calls[0][0] == 3  # L3 正常发送


def test_status_shows_filters(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "admin_token", "")
    settings.alert_filters = json.dumps({"wecom": {"min_level": 3}})
    with TestClient(app) as c:
        chs = c.get("/api/alerts/status").json()["channels"]
        w = next(x for x in chs if x["name"] == "wecom")
        assert w["filters"] == {"min_level": 3}


def test_alert_test_not_filtered(tmp_path, monkeypatch):
    """alert-test 不受过滤影响：配置了过滤也照发测试消息。"""
    s = Store(tmp_path / "s.db")
    monkeypatch.setattr(settings, "alert_filters", json.dumps({"wecom": {"min_level": 3}}))
    d = al.AlertDispatcher(store=s)
    spy = SpyCh()
    d.channels = [spy]
    d.send_test()  # L1 测试消息——min_level=3 会拦普通事件，但测试不受影响
    assert len(spy.calls) == 1
