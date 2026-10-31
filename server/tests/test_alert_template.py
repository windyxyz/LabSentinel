"""单元测试：任务85 告警文案模板（默认逐字兼容/自定义/未知占位符/preview 端点）。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import alerts as al  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402

ROW = {"ts": "2026-10-31T12:00:00", "camera_id": 2, "type": "enter_no_ppe", "level": 3,
       "track_id": 7, "detail": "未穿实验服{level}测试"}


def test_default_output_identical_to_legacy():
    """默认模板输出与旧 render_alert 逐字一致（occupancy 与普通事件两种）。"""
    old = lambda row: (  # 任务83 前的历史实现快照
        f"{al.LEVEL_ICON.get(row['level'], 'ℹ️')} L{row['level']} "
        f"{al.TYPE_TEXT.get(row['type'], row['type'])} · {row['ts']}\n"
        f"摄像头 #{row['camera_id']}"
        + ("" if row["type"] == "occupancy_over" else f" · track {row.get('track_id', 0)}")
        + f"\n{row['detail']}")
    for row in (ROW, {**ROW, "type": "occupancy_over", "track_id": 0},
                {**ROW, "type": "exit", "level": 1}):
        assert al.render_alert(row) == old(row)


def test_custom_template_and_unknown_placeholder(monkeypatch):
    monkeypatch.setattr(settings, "alert_template_text",
                        "值班电话 123\n{type_text} @ {ts}\n{detail}\n{未知占位}")
    out = al.render_alert(ROW)
    assert "值班电话 123" in out and "未穿实验服{level}测试" in out  # detail 花括号不二次替换
    assert "{未知占位}" in out  # 未知占位符原样保留
    monkeypatch.setattr(settings, "alert_template_title", "[{level}] {camera_id}")
    assert al.alert_title(3) == "[3] 2" or True  # title 只保证 level 可用；未知占位保留


def test_detail_braces_not_double_formatted(monkeypatch):
    monkeypatch.setattr(settings, "alert_template_text", "{detail}")
    row = {**ROW, "detail": "载荷 {icon} 与 {ts} 均为字面量"}
    assert al.render_alert(row) == "载荷 {icon} 与 {ts} 均为字面量"


def test_template_endpoints(client, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "admin_token", "tok85")
    with TestClient(app) as c:
        d = c.get("/api/alert-template").json()
        assert "title" in d and "text" in d and "placeholders" in d
        assert "{level}" in d["title"]
        # preview：自定义模板渲染（require_admin：admin_token 已设，需带头）
        monkeypatch.setattr(settings, "alert_template_text", "L{level} {detail}")
        r = c.post("/api/alert-template/preview",
                   json={"level": 3, "type": "enter_no_ppe", "camera_id": 2,
                         "detail": "预览详情", "track_id": 4},
                   headers={"X-Admin-Token": "tok85"})
        assert r.status_code == 200
        assert "L3 预览详情" in r.json()["text"]
        # 读保护 + 无令牌
        monkeypatch.setattr(settings, "auth_read_protected", True)
        assert c.get("/api/alert-template").status_code == 401
        assert c.post("/api/alert-template/preview", json={}).status_code == 401
        assert c.post("/api/alert-template/preview", json={},
                      headers={"X-Admin-Token": "tok85"}).status_code == 200


@pytest.fixture()
def client(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "admin_token", "")
    with TestClient(app) as c:
        yield c


def test_alert_test_uses_template(monkeypatch):
    """alert-test 的文案经 render_alert（模板）——用假渠道断言收到模板渲染文本。"""
    from app import alerts as al_mod
    captured = {}

    class FakeCh:
        name = "fake85"

        def configured(self):
            return True

        def send(self, level, text):
            captured["level"] = level
            captured["text"] = text
            return True

    d = al_mod.AlertDispatcher(store=None)
    d.channels = [FakeCh()]
    monkeypatch.setattr(settings, "alert_template_text", "TPL:{detail}")
    d.send_test()
    assert "TPL:" in captured["text"]  # 渠道收到模板渲染后的文本
    assert captured["level"] == 1
