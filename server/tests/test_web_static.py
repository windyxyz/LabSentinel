"""单元测试：任务43 单端口模式（web/dist 托管 + SPA 回退 + 路由优先级 + 缺 dist 提示）
+ 任务72 缓存头与选择性 gzip。
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


# ---- 任务72：缓存头 + 选择性 gzip ----

def test_cache_headers(dist):
    with TestClient(app) as c:
        r = c.get("/assets/app.js")
        assert r.headers["cache-control"] == "public, max-age=31536000, immutable"
        assert c.get("/").headers["cache-control"] == "no-cache"
        assert c.get("/events").headers["cache-control"] == "no-cache"  # SPA 回退也 no-cache
        # 其它静态（无扩展名规则外）→ 1h
        (settings.web_dist / "favicon.ico").write_bytes(b"ico")
        assert c.get("/favicon.ico").headers["cache-control"] == "public, max-age=3600"


def test_304_keeps_cache_headers(dist):
    with TestClient(app) as c:
        r1 = c.get("/assets/app.js")
        etag = r1.headers["etag"]
        r2 = c.get("/assets/app.js", headers={"If-None-Match": etag})
        assert r2.status_code == 304
        assert "immutable" in r2.headers["cache-control"]


def test_gzip_assets_and_json(dist):
    big = "console.log('" + "x" * 4000 + "')"  # >1024 字节可压文本
    (settings.web_dist / "assets" / "big.js").write_text(big, encoding="utf-8")
    with TestClient(app) as c:
        r = c.get("/assets/big.js", headers={"Accept-Encoding": "gzip"})
        assert r.headers.get("content-encoding") == "gzip"
        assert r.headers.get("vary") == "Accept-Encoding"
        assert r.text == big  # httpx 自动解压后与原文一致
        # R2-005/N-38: httpx already decompressed the body -- calling
        # gzip.decompress() on r.content raises BadGzipFile. The old
        # `or True` also made the assertion vacuous. Assert the header instead.
        assert len(r.content) >= 1024
        rj = c.get("/api/system", headers={"Accept-Encoding": "gzip"})
        assert rj.status_code == 200  # JSON 可被压缩（不解也 200）
        assert rj.json()["version"]


def test_gzip_skips_zip_and_range(dist):
    (settings.web_dist / "assets" / "data.zip").write_bytes(b"PK" + b"0" * 4000)
    with TestClient(app) as c:
        r = c.get("/assets/data.zip", headers={"Accept-Encoding": "gzip"})
        assert r.headers.get("content-encoding") != "gzip"  # zip 不压
        rr = c.get("/assets/big.js", headers={"Accept-Encoding": "gzip", "Range": "bytes=0-100"})
        assert rr.headers.get("content-encoding") != "gzip" or rr.status_code == 200  # Range 优先


# ---- 任务77：大响应/流式保护 ----

def test_gzip_large_known_size_skipped(dist, monkeypatch):
    """Content-Length 已知但 > 阈值 → 不 gzip，body 完整。"""
    monkeypatch.setattr(settings, "gzip_max_bytes", 1000)
    big = "console.log('" + "x" * 3000 + "')"  # 3000B > 1000B 阈值
    (settings.web_dist / "assets" / "large.js").write_text(big, encoding="utf-8")
    with TestClient(app) as c:
        r = c.get("/assets/large.js", headers={"Accept-Encoding": "gzip"})
        assert r.headers.get("content-encoding") != "gzip"
        assert r.text == big  # body 完整
        assert r.headers.get("cache-control") == "public, max-age=31536000, immutable"  # 缓存头不受影响


def test_gzip_small_below_threshold_compressed(dist, monkeypatch):
    """阈值放宽时同尺寸文件会被压——证明跳过是阈值触发而非类型问题。"""
    monkeypatch.setattr(settings, "gzip_max_bytes", 10_000_000)
    big = "console.log('" + "x" * 3000 + "')"
    (settings.web_dist / "assets" / "large.js").write_text(big, encoding="utf-8")
    with TestClient(app) as c:
        r = c.get("/assets/large.js", headers={"Accept-Encoding": "gzip"})
        assert r.headers.get("content-encoding") == "gzip"
        assert r.text == big


def test_gzip_streaming_export_untouched(dist, monkeypatch):
    """流式 CSV 导出（无 Content-Length + attachment）不 gzip 且内容完整。"""
    from app.store import Store
    s = Store(settings.db_path)
    for i in range(3):
        s.add_event(1, "enter", 2, i, f"流式导出行 {i}")
    with TestClient(app) as c:
        r = c.get("/api/export/events.csv?days=365", headers={"Accept-Encoding": "gzip"})
        assert r.status_code == 200
        assert r.headers.get("content-encoding") != "gzip"
        assert "流式导出行 0" in r.text  # 内容完整
