"""单元测试：任务89 安全响应头（默认开/关闭/CSP/Frame-Options/不覆盖已有）。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402


def _headers(client, path="/"):
    return client.get(path).headers


def test_default_security_headers(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "security_headers", True)
    monkeypatch.setattr(settings, "csp", "")
    with TestClient(app) as c:
        for path in ("/", "/api/system", "/api/definitely-not-here"):
            h = c.get(path).headers
            assert h.get("x-content-type-options") == "nosniff"
            assert h.get("x-frame-options") == "DENY"
            assert h.get("referrer-policy") == "no-referrer"
            assert "camera=()" in h.get("permissions-policy", "")
            assert h.get("cross-origin-resource-policy") == "same-site"
            assert "content-security-policy" not in h  # 默认不发送 CSP


def test_security_headers_disabled(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "security_headers", False)
    with TestClient(app) as c:
        h = c.get("/api/system").headers
        assert "x-content-type-options" not in h
        assert "x-frame-options" not in h


def test_frame_options_sameorigin(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "security_headers", True)
    monkeypatch.setattr(settings, "frame_options", "SAMEORIGIN")
    with TestClient(app) as c:
        assert c.get("/api/system").headers["x-frame-options"] == "SAMEORIGIN"


def test_csp_and_report_only(tmp_path, monkeypatch):
    csp_val = "default-src 'self'; connect-src 'self' ws: wss: http://127.0.0.1:*"
    monkeypatch.setattr(settings, "security_headers", True)
    monkeypatch.setattr(settings, "csp", csp_val)
    monkeypatch.setattr(settings, "csp_report_only", False)
    with TestClient(app) as c:
        h = c.get("/api/system").headers
        assert h.get("content-security-policy") == csp_val
        assert "report-only" not in h.get("content-security-policy", "").lower()
    monkeypatch.setattr(settings, "csp_report_only", True)
    with TestClient(app) as c:
        h = c.get("/api/system").headers
        assert h.get("content-security-policy-report-only") == csp_val


def test_no_overwrite_existing_headers(tmp_path, monkeypatch):
    """安全头用 setdefault 添加——已有 x-request-id/content-type 等头不被覆盖。"""
    monkeypatch.setattr(settings, "security_headers", True)
    with TestClient(app) as c:
        h = c.get("/api/system").headers
        assert h.get("x-content-type-options") == "nosniff"
        assert h.get("x-request-id")  # 原有头仍在
        assert h.get("content-type", "").startswith("application/json")  # 原有头仍在
