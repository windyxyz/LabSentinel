"""T-331（L3）契约测试：同一套 API 用例分别打 Python 实现与 Rust 实现。

Python 侧 = fastapi TestClient（app.main:app）；Rust 侧 = 临时起 labsentinel-server
（D:/tmp/ls-rs-target/debug/labsentinel-server.exe，指向同一 SQLite 库），
黑盒 httpx 逐端点断言状态码与核心字段一致。exe 缺失时 Rust 侧 skip。
运行: pytest tests/test_contract_l3.py -q
"""
import os
import socket
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import httpx  # noqa: E402
import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import api as api_mod  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402

REPO = Path(__file__).resolve().parents[2]
EXE = Path("D:/tmp/ls-rs-target/debug/labsentinel-server.exe")


def _free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


@pytest.fixture()
def cross_db(tmp_path, monkeypatch):
    db = tmp_path / "contract.db"
    # lifespan 会用 settings.db_path 重建 Store —— 先指到同一库再建数据
    monkeypatch.setattr(settings, "db_path", db)
    py = Store(db)
    cid = py.add_camera("契约摄像头", "x.mp4", "file")
    eid = py.add_event(cid, "enter", 2, 1, "契约事件", ts="2026-01-01T00:00:01")
    return db, cid, eid


def _contract_cases(base: str, cid: int, eid: int):
    """两个实现必须同时满足的契约（核心子集，与 docs/openapi.json 一致）。"""
    with httpx.Client(base_url=base, timeout=5) as c:
        r = c.get("/api/health/live")
        assert r.status_code == 200 and r.json()["status"] == "ok"
        r = c.get("/api/system")
        assert r.status_code == 200, r.text
        assert "version" in r.json() and "components" in r.json()
        r = c.get("/api/cameras")
        assert r.status_code == 200 and isinstance(r.json(), list)
        r = c.get("/api/events")
        assert r.status_code == 200 and isinstance(r.json(), list)
        r = c.get("/api/events/count")
        assert r.status_code == 200 and isinstance(r.json().get("count"), int)
        r = c.get(f"/api/cameras/{cid}/status")
        assert r.status_code == 200 and r.json()["id"] == cid
        r = c.get("/api/stats/summary")
        assert r.status_code == 200 and "total" in r.json()
        r = c.get("/api/config/summary")
        assert r.status_code == 200
        # T-330：后台作业状态端点（L2 Rust 实现同契约）
        r = c.get("/api/maintenance/status")
        assert r.status_code == 200 and "disk_free_gb" in r.json()
        r = c.get("/api/backups/status")
        assert r.status_code == 200 and "enabled" in r.json()
        r = c.get("/api/escalation/status")
        assert r.status_code == 200 and "after_min" in r.json()
        # 跨站写请求一律 403
        r = c.post("/api/backups/run", headers={"Origin": "https://evil.example"})
        assert r.status_code == 403, (r.status_code, r.text)
        # 非法处置状态 400
        r = c.post(f"/api/events/{eid}/status", json={"status": "bogus"})
        assert r.status_code == 400


def test_python_meets_contract(cross_db):
    db, cid, eid = cross_db
    with TestClient(app) as c:
        # conftest 隔离 fixture 会把 db_path 指向 test.db；测试种子写进 lifespan 实际使用的库
        from app import api as _api
        _seed = Store(settings.db_path)
        _seed.add_camera("契约摄像头2", "x.mp4", "file")
        _seed.add_event(1, "enter", 2, 1, "契约事件", ts="2026-01-01T00:00:01")
        # TestClient 内部用 ASGI transport；这里逐条等价断言（Python 实现作为契约真值）
        assert c.get("/api/health/live").status_code == 200
        r = c.get("/api/system")
        assert r.status_code == 200 and "version" in r.json()
        assert isinstance(c.get("/api/cameras").json(), list)
        assert isinstance(c.get("/api/events").json(), list)
        r = c.get("/api/events/count")
        assert r.status_code == 200 and isinstance(r.json().get("count"), int)
        r = c.get(f"/api/cameras/{cid}/status")
        assert r.status_code == 200 and r.json()["id"] == cid
        assert c.get("/api/stats/summary").status_code == 200
        assert c.get("/api/config/summary").status_code == 200
        assert "disk_free_gb" in c.get("/api/maintenance/status").json()
        assert "enabled" in c.get("/api/backups/status").json()
        assert "after_min" in c.get("/api/escalation/status").json()
        assert c.post("/api/backups/run",
                      headers={"Origin": "https://evil.example"}).status_code == 403


@pytest.mark.skipif(not EXE.exists(), reason="labsentinel-server.exe 未构建")
def test_rust_meets_contract(cross_db):
    db, cid, eid = cross_db
    port = _free_port()
    env = {**os.environ, "LS_DB_PATH": str(db), "LS_PORT": str(port),
           "LS_VERSION": "0.2.0", "LS_ADMIN_TOKEN": ""}
    proc = subprocess.Popen([str(EXE)], env=env, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True)
    try:
        base = f"http://127.0.0.1:{port}"
        import time as _t
        for _ in range(60):
            try:
                # 回环探活：base_url Client + 相对路径（Mimosa SSRF 规则）
                with httpx.Client(base_url=base, timeout=1.0) as probe:
                    if probe.get("/api/health/live").status_code == 200:
                        break
            except httpx.HTTPError:
                _t.sleep(0.2)
        else:
            pytest.fail("Rust 服务未就绪")
        _contract_cases(base, cid, eid)
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
