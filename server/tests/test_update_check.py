"""单元测试：任务117 版本更新检查（mock httpx 不访问外网；只 GET manifest 绝不下载）。
运行: pytest tests/test_update_check.py -q"""
import httpx
import pytest

from app import update_check as uc
from app import api as api_mod
from app.config import settings
from fastapi.testclient import TestClient
from app.main import app


@pytest.fixture(autouse=True)
def reset_cache():
    uc._cache.update({"ts": 0.0, "result": None})
    yield
    uc._cache.update({"ts": 0.0, "result": None})


def _fake_httpx(monkeypatch, calls, body=b'{"version":"2026.11"}', status=200,
                raise_exc=None):
    """mock 生产实现的新请求接缝（httpx.Client base_url+相对路径，语义同旧 httpx.get：
    单次 GET、不跟随重定向）；同时 mock DNS 解析保证单测不触网。"""
    class _FakeClient:
        def __init__(self, base_url=None, timeout=None, follow_redirects=None, **kw):
            self._base = str(base_url or "")
            self._follow = follow_redirects

        def __enter__(self):
            return self

        def __exit__(self, *exc):
            return False

        def get(self, path):
            url = self._base + str(path)
            calls.append({"url": url, "follow_redirects": self._follow})
            if raise_exc is not None:
                raise raise_exc
            return httpx.Response(status, content=body,
                                  request=httpx.Request("GET", url))

    monkeypatch.setattr(uc.httpx, "Client", _FakeClient)

    def fake_getaddrinfo(host, port, *a, **kw):
        return [(2, 1, 6, "", ("93.184.216.34", 0))]  # 公网 IP，公网校验放行
    monkeypatch.setattr(uc.socket, "getaddrinfo", fake_getaddrinfo)


MANIFEST = ('{"version":"2026.11","url":"https://example.com/LabSentinel-portable.zip",'
            '"sha256":"abc","notes":"修复若干","published_at":"2026-11-01","channel":"stable"}'
            ).encode("utf-8")


def test_not_configured_returns_enabled_false():
    r = uc.check_update("", "0.1.0-m1")
    assert r == {"enabled": False, "current": "0.1.0-m1", "update_available": False,
                 "latest": None, "url": None, "sha256": None, "notes": None,
                 "published_at": None, "channel": None, "error": ""}


def test_https_manifest_and_version_compare(monkeypatch):
    calls = []
    _fake_httpx(monkeypatch, calls, body=MANIFEST)
    r = uc.check_update("https://example.com/manifest.json", "2026.10")
    assert r["enabled"] is True and r["update_available"] is True
    assert r["latest"] == "2026.11" and r["channel"] == "stable" and r["sha256"] == "abc"
    # 相同版本 → 不提示
    uc._cache.update({"ts": 0.0, "result": None})
    r2 = uc.check_update("https://example.com/manifest.json", "2026.11")
    assert r2["update_available"] is False


def test_http_rejected_by_default_and_allowed_explicitly(monkeypatch):
    calls = []
    _fake_httpx(monkeypatch, calls, body=MANIFEST)
    r = uc.check_update("http://192.168.1.10/manifest.json", "2026.10")
    assert "https" in r["error"] and r["latest"] is None
    assert calls == []  # 协议检查在发请求前
    r2 = uc.check_update("http://192.168.1.10/manifest.json", "2026.10", allow_http=True)
    assert r2["latest"] == "2026.11" and r2["error"] == ""


def test_no_redirect_followed(monkeypatch):
    """follow_redirects 必须为 False（重定向到任意地址是信息面）。"""
    calls = []
    _fake_httpx(monkeypatch, calls, body=MANIFEST)
    uc.check_update("https://example.com/manifest.json", "2026.10")
    assert calls[0]["follow_redirects"] is False


def test_failures_become_error_not_exception(monkeypatch):
    calls = []
    _fake_httpx(monkeypatch, calls, raise_exc=httpx.ConnectTimeout("t"))
    r = uc.check_update("https://example.com/m.json", "x")
    assert "清单请求失败" in r["error"] and r["latest"] is None
    uc._cache.update({"ts": 0.0, "result": None})
    _fake_httpx(monkeypatch, calls, body=b"not json", status=200)
    assert "JSON" in uc.check_update("https://example.com/m.json", "x")["error"]
    uc._cache.update({"ts": 0.0, "result": None})
    _fake_httpx(monkeypatch, calls, body=b'{"nope":1}', status=200)
    assert "version" in uc.check_update("https://example.com/m.json", "x")["error"]
    uc._cache.update({"ts": 0.0, "result": None})
    _fake_httpx(monkeypatch, calls, body=b"x", status=404)
    assert "404" in uc.check_update("https://example.com/m.json", "x")["error"]


def test_cache_one_hour_and_refresh(monkeypatch):
    calls = []
    _fake_httpx(monkeypatch, calls, body=MANIFEST)
    uc.check_update("https://example.com/m.json", "2026.10")
    uc.check_update("https://example.com/m.json", "2026.10")  # 命中缓存
    assert len(calls) == 1
    uc.check_update("https://example.com/m.json", "2026.10", refresh=True)  # 强制
    assert len(calls) == 2


def test_only_get_manifest_never_downloads(monkeypatch):
    """断言只对 manifest URL 发了一次 GET（url 字段只是回显，不会被访问）。"""
    calls = []
    _fake_httpx(monkeypatch, calls, body=MANIFEST)
    uc.check_update("https://example.com/m.json", "2026.10")
    assert [c["url"] for c in calls] == ["https://example.com/m.json"]


# ---- API ----

@pytest.fixture()
def client():
    with TestClient(app) as tc:
        yield tc


def test_api_unconfigured_and_flow(client, monkeypatch):
    monkeypatch.setattr(settings, "update_manifest_url", "")
    r = client.get("/api/update/check").json()
    assert r["enabled"] is False
    monkeypatch.setattr(settings, "update_manifest_url", "https://example.com/m.json")
    calls = []
    _fake_httpx(monkeypatch, calls, body=MANIFEST)
    r2 = client.get("/api/update/check?refresh=1").json()
    assert r2["enabled"] is True and r2["update_available"] is True


def test_api_auth(client, monkeypatch):
    monkeypatch.setattr(api_mod.settings, "admin_token", "sec")
    monkeypatch.setattr(api_mod.settings, "auth_read_protected", True)
    assert client.get("/api/update/check").status_code == 401
    assert client.get("/api/update/check",
                      headers={"X-Admin-Token": "sec"}).status_code == 200
    monkeypatch.setattr(api_mod.settings, "admin_token", "")
    monkeypatch.setattr(api_mod.settings, "auth_read_protected", False)
