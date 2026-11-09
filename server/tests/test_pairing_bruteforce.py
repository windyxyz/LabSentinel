"""R2-110/N-17 回归：配对码防爆破（限流 + 连续失败作废全部待配对码）。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import devices  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402


class FakeStore:
    def __init__(self):
        self.rows = []

    def add_device(self, name, platform, token_hash):
        self.rows.append((name, platform, token_hash))
        return len(self.rows)


def test_five_failures_invalidate_all_codes(monkeypatch):
    monkeypatch.setattr(devices, "_pending", {})
    monkeypatch.setattr(devices, "_claim_failures", 0)
    code1 = devices.new_pairing_code()
    code2 = devices.new_pairing_code()
    s = FakeStore()
    for _ in range(4):
        assert devices.claim(s, "000000", "x") is None
    # 第 5 次失败后所有待配对码作废
    assert devices.claim(s, "000000", "x") is None
    assert devices.claim(s, code1, "x") is None
    assert devices.claim(s, code2, "x") is None
    assert not devices._pending


def test_success_resets_failure_counter(monkeypatch):
    monkeypatch.setattr(devices, "_pending", {})
    monkeypatch.setattr(devices, "_claim_failures", 0)
    s = FakeStore()
    for _ in range(4):
        assert devices.claim(s, "000000", "x") is None
    assert devices._claim_failures == 4
    code = devices.new_pairing_code()
    assert devices.claim(s, code, "phone") is not None
    assert devices._claim_failures == 0


def test_claim_rate_limit_per_ip(monkeypatch):
    ip = "192.0.2.77"
    monkeypatch.setattr(devices, "_claim_rate", {})
    for _ in range(10):
        assert devices.claim_rate_ok(ip) is True
    assert devices.claim_rate_ok(ip) is False
    assert devices.claim_rate_ok("192.0.2.78") is True


def test_claim_endpoint_rate_limited(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "db_path", tmp_path / "rate.db")
    monkeypatch.setattr(settings, "admin_token", "adm")
    # 隔离全局限流状态，避免把后续用例的配对请求也 429
    monkeypatch.setattr(devices, "_claim_rate", {})
    monkeypatch.setattr(devices, "_pending", {})
    monkeypatch.setattr(devices, "_claim_failures", 0)
    with TestClient(app) as c:
        code = c.post("/api/pairing",
                      headers={"X-Admin-Token": "adm"}).json()["code"]
        # 前 10 次（含 1 次成功）放行，第 11 次 429
        statuses = []
        for i in range(11):
            body = {"code": code if i == 0 else "000000", "device_name": "x"}
            statuses.append(c.post("/api/pairing/claim", json=body).status_code)
        assert statuses[0] == 200
        assert 429 in statuses
