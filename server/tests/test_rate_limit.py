"""单元测试：任务90 写接口限流（滑动窗口/阈值/429/信任代理/关闭）。"""
import os
import sys
from collections import deque

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import main as main_mod
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402


@pytest.fixture(autouse=True)
def reset_windows():
    main_mod._rate_windows.clear()
    yield
    main_mod._rate_windows.clear()


@pytest.fixture()
def client(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "admin_token", "")
    monkeypatch.setattr(settings, "write_rate_limit", 2)  # 测试阈值 2/min
    monkeypatch.setattr(settings, "write_rate_window_s", 60)
    monkeypatch.setattr(settings, "trust_proxy", False)
    with TestClient(app) as c:
        yield c


def test_rate_limit_2_per_min(client):
    """limit=2：前 2 个 POST 正常，第 3 个 → 429 + Retry-After + request_id。"""
    r1 = client.post("/api/cameras", json={"name": "a", "type": "file", "source": "s.mp4"})
    r2 = client.post("/api/cameras", json={"name": "b", "type": "file", "source": "s.mp4"})
    assert r1.status_code == 200 and r2.status_code == 200
    r3 = client.post("/api/cameras", json={"name": "c", "type": "file", "source": "s.mp4"})
    assert r3.status_code == 429
    d = r3.json()
    assert d["code"] == "rate_limited" and "频繁" in d["detail"]
    assert r3.headers.get("retry-after") and r3.headers.get("x-request-id")
    # GET 不受限
    assert client.get("/api/cameras").status_code == 200


def test_window_expiry_allows_again(client, monkeypatch):
    """窗口过期后可再写（monkeypatch time.monotonic 模拟时间推进）。"""
    import time as time_mod
    real = time_mod.monotonic
    offset = [0.0]
    monkeypatch.setattr(main_mod.time, "monotonic", lambda: real() + offset[0])
    client.post("/api/cameras", json={"name": "a", "type": "file", "source": "s.mp4"})
    client.post("/api/cameras", json={"name": "b", "type": "file", "source": "s.mp4"})
    offset[0] += 61  # 越过 60s 窗口
    r = client.post("/api/cameras", json={"name": "c", "type": "file", "source": "s.mp4"})
    assert r.status_code == 200


def test_trust_proxy_toggle(client, monkeypatch):
    """trust_proxy=0 忽略 X-Forwarded-For；=1 时代理后不同客户端互不影响。"""
    monkeypatch.setattr(settings, "trust_proxy", False)
    h = {"X-Forwarded-For": "10.0.0.1, 10.0.0.2"}
    client.post("/api/cameras", json={"name": "a", "type": "file", "source": "s.mp4"}, headers=h)
    client.post("/api/cameras", json={"name": "b", "type": "file", "source": "s.mp4"}, headers=h)
    # trust_proxy=0：XFF 被忽略，仍算同一 IP → 429
    r = client.post("/api/cameras", json={"name": "c", "type": "file", "source": "s.mp4"}, headers=h)
    assert r.status_code == 429
    main_mod._rate_windows.clear()
    monkeypatch.setattr(settings, "trust_proxy", True)
    # trust_proxy=1：XFF 生效，不同客户端 IP 各自独立
    client.post("/api/cameras", json={"name": "d", "type": "file", "source": "s.mp4"},
                headers={"X-Forwarded-For": "10.0.0.1"})
    r = client.post("/api/cameras", json={"name": "e", "type": "file", "source": "s.mp4"},
                    headers={"X-Forwarded-For": "10.0.0.2"})
    assert r.status_code == 200  # 不同代理后 IP → 不限


def test_rate_limit_zero_disabled(client, monkeypatch):
    monkeypatch.setattr(settings, "write_rate_limit", 0)
    for i in range(10):
        assert client.post("/api/cameras",
                           json={"name": f"x{i}", "type": "file",
                                 "source": "s.mp4"}).status_code == 200


def test_429_has_request_id(client):
    client.post("/api/cameras", json={"name": "a", "type": "file", "source": "s.mp4"})
    client.post("/api/cameras", json={"name": "b", "type": "file", "source": "s.mp4"})
    r = client.post("/api/cameras", json={"name": "c", "type": "file", "source": "s.mp4"})
    assert r.status_code == 429
    assert r.headers.get("x-request-id")
    d = r.json()
    assert d["request_id"] == r.headers["x-request-id"]
