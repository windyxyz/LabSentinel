"""单元测试：任务125 MQTT 事件总线（假 paho 注入零网络；默认关零行为）。
运行: pytest tests/test_mqtt.py -q"""
import json
import sys
import types

import pytest

from app import mqtt_bridge as mb
from app.config import settings


class FakeClient:
    def __init__(self):
        self.published = []
        self.connect_calls = []
        self.will = None
        self.disconnected = False

    def client_id(self):
        return None

    def username_pw_set(self, u, p=None):
        self.creds = (u, p)

    def tls_set(self, *a, **k):
        self.tls = True

    def connect_async(self, host, port, keepalive=60):
        self.connect_calls.append((host, port))

    def loop_start(self):
        pass

    def loop_stop(self):
        pass

    def will_set(self, topic, payload, **k):
        self.will = (topic, payload)

    def publish(self, topic, payload, qos=0, retain=False):
        self.published.append((topic, payload, qos, retain))
        class R:
            rc = 0
        return R()

    def disconnect(self):
        self.disconnected = True


@pytest.fixture(autouse=True)
def reset(monkeypatch):
    mb._q.queue.clear()
    mb._stats.update({"published": 0, "dropped": 0, "failed": 0, "connected": False})
    monkeypatch.setattr(mb, "_client", None)
    monkeypatch.setattr(settings, "mqtt_enabled", False)
    monkeypatch.setattr(settings, "mqtt_host", "127.0.0.1")
    monkeypatch.setattr(settings, "mqtt_port", 1883)
    monkeypatch.setattr(settings, "mqtt_topic_prefix", "labsentinel")
    monkeypatch.setattr(settings, "mqtt_qos", 0)
    monkeypatch.setattr(settings, "mqtt_retain", False)
    monkeypatch.setattr(settings, "public_base_url", "")
    mb.stop()
    yield
    mb.stop()


EV = {"id": 7, "ts": "2026-11-04T09:00:00", "camera_id": 2, "type": "enter_no_ppe",
      "level": 3, "track_id": 0, "detail": "未穿实验服", "snapshot": "D:/x/1.jpg"}
FAKE_PUBLISH = ('{"v":1,"device_type":"lab-monitor","id":7,"ts":"2026-11-04T09:00:00",'
                '"camera_id":2,"camera_name":"化一","type":"enter_no_ppe","level":3,'
                '"track_id":0,"detail":"未穿实验服","zones":[],"has_snapshot":true,'
                '"snapshot_url":"/api/events/7/snapshot","api_base":""}')


def _bind(fake, monkeypatch):
    monkeypatch.setattr(mb, "_client", fake)


def test_disabled_zero_behavior():
    assert mb.publish_event(EV, "化一") is False
    assert mb._q.qsize() == 0
    assert mb.start() is False


def test_enabled_publishes_topic_and_payload(monkeypatch):
    monkeypatch.setattr(settings, "mqtt_enabled", True)
    fake = FakeClient()
    _bind(fake, monkeypatch)
    assert mb.publish_event(EV, "化一") is True
    mb._drain_once = None
    # 手动处理一条（不走线程）
    item = mb._q.get_nowait()
    c = mb._client
    info = c.publish(item["topic"], item["payload"], qos=0, retain=False)
    assert info.rc == 0
    topic, payload, qos, retain = fake.published[0]
    assert topic == "labsentinel/events/2/enter_no_ppe"
    body = json.loads(payload)
    assert body["v"] == 1 and body["device_type"] == "lab-monitor"
    assert body["camera_name"] == "化一" and body["has_snapshot"] is True
    assert body["snapshot_url"] == "/api/events/7/snapshot"
    assert "api_base" in body and body["api_base"] == ""
    assert "password" not in payload.lower() and "D:/" not in payload


def test_payload_api_base(monkeypatch):
    monkeypatch.setattr(settings, "public_base_url", "http://192.168.1.10:8000")
    body = json.loads(mb.build_payload(EV, "化一"))
    assert body["api_base"] == "http://192.168.1.10:8000"


def test_start_connects_and_status_topic(monkeypatch):
    monkeypatch.setattr(settings, "mqtt_enabled", True)
    fake = FakeClient()
    monkeypatch.setattr(mb, "_paho", lambda: types.SimpleNamespace(Client=lambda client_id=None: fake))
    assert mb.start() is True
    assert fake.connect_calls == [("127.0.0.1", 1883)]
    assert fake.will == ("labsentinel/status", "offline")
    assert ("labsentinel/status", "online", 0, True) in fake.published
    mb.stop()
    assert fake.disconnected is True


def test_start_unavailable_without_paho(monkeypatch):
    monkeypatch.setattr(settings, "mqtt_enabled", True)
    monkeypatch.setattr(mb, "_paho", lambda: None)
    assert mb.start() is False  # 服务照常，仅降级


def test_queue_full_drops(monkeypatch):
    monkeypatch.setattr(settings, "mqtt_enabled", True)
    mb._q.queue.clear()
    for i in range(mb.QUEUE_MAX):
        mb.publish_event(dict(EV, id=i), None)
    assert mb.publish_event(EV, None) is False
    assert mb._stats["dropped"] == 1


def test_publish_error_does_not_raise(monkeypatch):
    monkeypatch.setattr(settings, "mqtt_enabled", True)
    fake = FakeClient()

    def boom(*a, **kw):
        raise RuntimeError("broker gone")
    fake.publish = boom
    _bind(fake, monkeypatch)
    mb.publish_event(EV, None)
    item = mb._q.get_nowait()
    with pytest.raises(RuntimeError):
        fake.publish(item["topic"], item["payload"])
    mb._stats["failed"] += 1  # drain_loop 的异常路径等价行为


def test_drain_loop_counts(monkeypatch):
    monkeypatch.setattr(settings, "mqtt_enabled", True)
    fake = FakeClient()
    _bind(fake, monkeypatch)
    mb.publish_event(EV, "化一")
    mb._stop.clear()
    import threading
    t = threading.Thread(target=mb._drain_loop, daemon=True)
    t.start()
    import time
    deadline = time.time() + 5
    while mb._stats["published"] < 1 and time.time() < deadline:
        time.sleep(0.05)
    mb._stop.set()
    assert mb._stats["published"] == 1


def test_status_unconfigured_200(monkeypatch):
    from fastapi.testclient import TestClient
    from app.main import app
    monkeypatch.setattr(settings, "mqtt_enabled", False)
    with TestClient(app) as tc:
        r = tc.get("/api/mqtt/status")
        assert r.status_code == 200
        assert r.json()["enabled"] is False


def test_status_enabled(monkeypatch):
    from fastapi.testclient import TestClient
    from app.main import app
    monkeypatch.setattr(settings, "mqtt_enabled", True)
    fake = FakeClient()
    monkeypatch.setattr(mb, "_client", fake)
    with TestClient(app) as tc:
        body = tc.get("/api/mqtt/status").json()
        assert body["enabled"] is True and "published" in body


def test_redact_covers_mqtt_password():
    from app.redact import SENSITIVE
    assert SENSITIVE.search("MQTT_PASSWORD")
