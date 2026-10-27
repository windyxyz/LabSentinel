"""单元测试：任务43 单端口模式（web/dist 托管 + SPA 回退 + 路由优先级 + 缺 dist 提示）。
运行: pytest tests/test_web_static.py -q"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app.config import settings  # noqa: E402
from app.main import app, mount_web  # noqa: E402


@pytest.fixture()
def dist(tmp_path, monkeypatch):
    """造一个最小 dist：index.html + assets/app.js，重挂到 app。"""
    (tmp_path / "assets").mkdir()
    (tmp_path / "index.html").write_text("<html>INDEX-MARK-43</html>", encoding="utf-8")
    (tmp_path / "assets" / "app.js").write_text("console.log('app43')", encoding="utf-8")
    monkeypatch.setattr(settings, "web_dist", tmp_path)
    mount_web(app)
    yield tmp_path
    monkeypatch.undo()
    mount_web(app)  # 恢复默认挂载，避免影响其他用例


def test_root_serves_index(dist):
    with TestClient(app) as c:
        r = c.get("/")
        assert r.status_code == 200
        assert "INDEX-MARK-43" in r.text


def test_spa_fallback_events(dist):
    """无扩展名的非资源路径 404 时回 index.html（前端路由刷新不 404）。"""
    with TestClient(app) as c:
        r = c.get("/events")
        assert r.status_code == 200
        assert "INDEX-MARK-43" in r.text


def test_static_asset_served(dist):
    with TestClient(app) as c:
        r = c.get("/assets/app.js")
        assert r.status_code == 200
        assert "app43" in r.text


def test_missing_asset_still_404(dist):
    """带扩展名的缺失资源必须 404，不能被 SPA 回退伪装成 html。"""
    with TestClient(app) as c:
        r = c.get("/assets/missing.js")
        assert r.status_code == 404


def test_unknown_api_still_404(dist):
    """未知 /api/* 必须 404，不能被 SPA 回退成 index.html。"""
    with TestClient(app) as c:
        r = c.get("/api/definitely-not-a-route")
        assert r.status_code == 404
        assert "INDEX-MARK-43" not in r.text


def test_api_and_docs_take_priority(dist):
    """/api/* 与 /docs 必须优先于静态挂载（挂载在路由注册之后）。"""
    with TestClient(app) as c:
        r = c.get("/api/system")
        assert r.status_code == 200
        assert r.headers["content-type"].startswith("application/json")
        assert "INDEX-MARK-43" not in r.text
        assert c.get("/docs").status_code == 200


def test_missing_dist_hint(tmp_path, monkeypatch):
    """dist 不存在：GET / 返回提示 JSON，不影响 API。"""
    monkeypatch.setattr(settings, "web_dist", tmp_path / "nope")
    mount_web(app)
    try:
        with TestClient(app) as c:
            r = c.get("/")
            assert r.status_code == 200
            assert "not built" in r.json().get("message", "")
            assert c.get("/api/system").status_code == 200
    finally:
        monkeypatch.undo()
        mount_web(app)
