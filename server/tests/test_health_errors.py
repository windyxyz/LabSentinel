"""单元测试：任务63 健康探针 + 请求 ID + 统一错误响应。
运行: pytest tests/test_health_errors.py -q
路径隔离由 tests/conftest.py 的 autouse fixture 统一提供（105 补丁 P1-2）。"""
import logging
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402


def test_live():
    with TestClient(app) as c:
        r = c.get("/api/health/live")
        assert r.status_code == 200 and r.json() == {"status": "ok"}


def test_ready_ok_then_model_missing(tmp_path, monkeypatch):
    with TestClient(app) as c:
        r = c.get("/api/health/ready")
        assert r.status_code == 200
        d = r.json()
        assert d["status"] == "ready"
        assert all(v["ok"] for v in d["checks"].values()), d
        # 删模型引用 → 503 且 model.ok=false
        monkeypatch.setattr(settings, "ppe_model", tmp_path / "no-such.onnx")
        r2 = c.get("/api/health/ready")
        assert r2.status_code == 503
        assert r2.json()["status"] == "degraded"
        assert r2.json()["checks"]["model"]["ok"] is False


def test_request_id_echo_and_generate():
    with TestClient(app) as c:
        r = c.get("/api/health/live", headers={"X-Request-ID": "my-trace-123"})
        assert r.headers["X-Request-ID"] == "my-trace-123"
        r2 = c.get("/api/health/live")
        rid = r2.headers["X-Request-ID"]
        assert len(rid) == 32 and all(ch in "0123456789abcdef" for ch in rid)
        r3 = c.get("/api/health/live", headers={"X-Request-ID": "x" * 200})  # 超长忽略并重新生成
        assert r3.headers["X-Request-ID"] != "x" * 200 and len(r3.headers["X-Request-ID"]) == 32


def test_error_bodies():
    with TestClient(app) as c:
        r404 = c.get("/api/definitely-not-here")
        assert r404.status_code == 404
        b = r404.json()
        assert b["code"] == "http_404" and "detail" in b and b["request_id"] == r404.headers["X-Request-ID"]
        r422 = c.post("/api/cameras", json={"type": "bogus"})  # 缺 name/source
        assert r422.status_code == 422
        b422 = r422.json()
        assert b422["code"] == "validation_error" and b422["request_id"]
        assert "bogus" not in b422["detail"]  # 可读摘要而非整段 pydantic


def test_internal_error_shape(monkeypatch, caplog):
    class Boom:
        def cameras(self):
            raise RuntimeError("boom-61-secret-detail")

    import app.api as api_mod
    old = api_mod.store
    try:
        with TestClient(app, raise_server_exceptions=False) as c:
            api_mod.store = Boom()  # lifespan 会重置 store，必须在进入上下文后再注入
            with caplog.at_level(logging.ERROR, logger="app.errors"):
                r = c.get("/api/cameras", headers={"X-Request-ID": "trace-500"})
            assert r.status_code == 500
            d = r.json()
            assert d["code"] == "internal_error" and d["request_id"] == "trace-500"
            assert d["detail"] == "服务器内部错误"
            assert "boom-61-secret-detail" not in r.text  # 堆栈/内部信息不回传
            # 日志含 request_id 与路径（server.log 按 [req=...] 检索的依据）
            assert any("trace-500" in rec.message and "/api/cameras" in rec.message
                       for rec in caplog.records)
    finally:
        api_mod.store = old
