"""单元测试：任务13 推送可靠性（重试/死信/errcode 判定/手动重发/自检）。运行: pytest tests/ -q"""
import json
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import httpx  # noqa: E402
import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import api as api_mod  # noqa: E402
from app.alerts import AlertDispatcher, DingTalkChannel, RETRY_BACKOFF  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402


def _row(eid=1):
    return {"id": eid, "ts": "2026-10-26T10:00:00", "camera_id": 1, "type": "enter",
            "level": 2, "track_id": 1, "detail": "测试事件"}


def test_retry_then_deadline(tmp_path, monkeypatch):
    """失败渠道重试 3 次后进死信；deliveries 记录 attempts=4、status=failed。"""
    monkeypatch.setattr("app.alerts.RETRY_BACKOFF", (0.0, 0.0, 0.0))  # 测试不真睡
    s = Store(path=tmp_path / "t.db")
    d = AlertDispatcher(store=s)
    d.channels = [DingTalkChannel(transport=_fail_transport())]
    d._deliver_with_retry(d.channels[0], 1, 2, "测试")
    rec = s.deliveries()[0]
    assert rec["status"] == "failed" and rec["attempts"] == 4  # 1 首次 + 3 重试
    assert rec["last_error"]


def _fail_transport(fail_times=None):
    """始终失败的 MockTransport（HTTP 200 + errcode!=0，模拟钉钉业务失败）。"""
    calls = []

    def handler(request: httpx.Request) -> httpx.Response:
        calls.append(1)
        return httpx.Response(200, json={"errcode": 310000, "errmsg": "invalid"})
    handler.calls = calls
    return httpx.MockTransport(handler)


def test_is_ok_errcode():
    """渠道成功判定：HTTP 2xx 且业务码为 0/200；errcode!=0 或非 2xx 都算失败。"""
    ok200 = httpx.Response(200, json={"errcode": 0})
    ok_code200 = httpx.Response(200, json={"code": 200})
    bad_err = httpx.Response(200, json={"errcode": 310000})
    bad_http = httpx.Response(500, json={"errcode": 0})
    from app.alerts import _HttpPushChannel
    assert _HttpPushChannel._is_ok(ok200) is True
    assert _HttpPushChannel._is_ok(ok_code200) is True
    assert _HttpPushChannel._is_ok(bad_err) is False
    assert _HttpPushChannel._is_ok(bad_http) is False
    assert _HttpPushChannel._is_ok(None) is False


def test_retry_success_on_third(monkeypatch, tmp_path):
    """前两次失败第三次成功 → status=ok、attempts=3。"""
    monkeypatch.setattr("app.alerts.RETRY_BACKOFF", (0.0, 0.0, 0.0))
    monkeypatch.setattr(settings, "dingtalk_webhook",
                        "https://oapi.dingtalk.com/robot/send?access_token=x")  # 显式配置，免受其他用例污染
    s = Store(path=tmp_path / "t.db")
    state = {"n": 0}

    def handler(request: httpx.Request) -> httpx.Response:
        state["n"] += 1
        return httpx.Response(200, json={"errcode": 0 if state["n"] >= 3 else 310000})

    d = AlertDispatcher(store=s)
    d.channels = [DingTalkChannel(transport=httpx.MockTransport(handler))]
    d._deliver_with_retry(d.channels[0], 7, 2, "x")
    rec = s.deliveries()[0]
    assert rec["status"] == "ok" and rec["attempts"] == 3


def test_dispatch_async_and_counts(tmp_path, monkeypatch):
    """dispatch 走线程池立即返回；死信计数正确。"""
    monkeypatch.setattr("app.alerts.RETRY_BACKOFF", (0.0, 0.0, 0.0))
    s = Store(path=tmp_path / "t.db")
    d = AlertDispatcher(store=s)

    class SlowFail:
        name = "slowfail"

        def send(self, level, text):
            raise RuntimeError("boom")

    d.channels = [SlowFail()]
    t0 = time.monotonic()
    d.dispatch(_row())  # 异步提交，立即返回
    assert time.monotonic() - t0 < 1.0
    d._pool.shutdown(wait=True)
    rec = s.deliveries()[0]
    assert rec["event_id"] == 1 and rec["channel"] == "slowfail"
    assert s.delivery_counts()["failed"] == 1


def test_delivery_api_and_retry_and_test(client, monkeypatch):
    c = client
    # 自检端点：全部渠道未配置 → 每个 ok=False，但 API 200
    r = c.post("/api/alert-test")
    assert r.status_code == 200
    results = r.json()
    assert len(results) == 8 and all(item["ok"] is False for item in results)  # 任务67 +webhook
    # 列表 + 计数
    r = c.get("/api/alert-deliveries", params={"status": "failed"})
    assert r.status_code == 200
    # 手动重发：不存在的投递 → 404
    assert c.post("/api/alert-deliveries/9999/retry").status_code == 404
    # system 计数字段
    sysr = c.get("/api/system").json()
    assert set(sysr["alerts"]) == {"pending", "failed", "ok"}


@pytest.fixture()
def client(tmp_path, monkeypatch):
    s = Store(path=tmp_path / "t.db")
    monkeypatch.setattr(api_mod, "store", s)
    d = AlertDispatcher(store=s)
    monkeypatch.setattr(api_mod.dispatcher, "store", s)
    c = TestClient(app)
    return c
