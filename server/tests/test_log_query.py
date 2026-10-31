"""单元测试：任务93 日志查询（尾部/级别/关键词/脱敏/白名单/文件缺失）。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402


@pytest.fixture()
def client(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "admin_token", "")
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    logs = tmp_path / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    with TestClient(app) as c:
        yield c


def _mk_log(tmp_path, name="server.log", lines=300):
    log = tmp_path / "logs" / name
    lines_out = []
    for i in range(lines):
        lv = "ERROR" if i % 50 == 0 else "WARNING" if i % 10 == 0 else "INFO"
        cred = f"rtsp://admin:pass{i}@10.0.0.1/x" if i == 5 else ""
        lines_out.append(f"2026-10-31 00:00:{i:02d} {lv} app.test [req=x] line {i} {cred}")
    log.write_text("\n".join(lines_out), encoding="utf-8")


def test_tail_and_level(client, tmp_path):
    _mk_log(tmp_path)
    r = client.get("/api/logs", params={"tail": 50}).json()
    assert len(r["lines"]) == 50 and r["scanned"] == 300
    r_err = client.get("/api/logs", params={"tail": 100, "level": "ERROR"}).json()
    assert all(" ERROR " in ln for ln in r_err["lines"])
    assert r_err["matched"] == 6  # 300 行 / 50 一错


def test_q_substring_and_redaction(client, tmp_path):
    _mk_log(tmp_path)
    r = client.get("/api/logs", params={"q": "pass5"}).json()
    assert r["matched"] == 0  # 已脱敏
    r2 = client.get("/api/logs", params={"q": "pass"})
    assert r2.status_code == 200  # 搜"pass"可命中脱敏后的行


def test_file_missing_returns_empty(client):
    r = client.get("/api/logs", params={"file": "server.log"}).json()
    assert r["lines"] == [] and "note" in r


def test_file_whitelist(client):
    assert client.get("/api/logs", params={"file": "../etc/passwd"}).status_code == 400
    assert client.get("/api/logs", params={"file": "server.log.5"}).status_code == 400
    assert client.get("/api/logs", params={"file": "server.log.1"}).status_code in (200, 404)


def test_tail_clamped(client, tmp_path):
    _mk_log(tmp_path, lines=10)
    r = client.get("/api/logs", params={"tail": 5000}).json()
    assert len(r["lines"]) == 10  # 夹取到文件实际行数


def test_requires_read_protected(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", True)
    monkeypatch.setattr(settings, "admin_token", "tok93")
    with TestClient(app) as c:
        assert c.get("/api/logs").status_code == 401
        assert c.get("/api/logs", headers={"X-Admin-Token": "tok93"}).status_code == 200
