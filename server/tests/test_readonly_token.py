"""单元测试：任务88 只读令牌权限矩阵（admin/read/anonymous × 读/写/WS）。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402


@pytest.fixture()
def isolated(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "db_path", tmp_path / "s.db")
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    yield


def _matrix(client):
    """返回 (读 anonymous, 读 read, 读 admin, 写 read, 写 admin) 状态码元组。"""
    rr = client.get("/api/cameras")
    rr_read = client.get("/api/cameras", headers={"X-Admin-Token": "read-tok"})
    rr_admin = client.get("/api/cameras", headers={"X-Admin-Token": "adm-tok"})
    wr = client.post("/api/cameras", json={"name": "x", "type": "file", "source": "s.mp4"},
                     headers={"X-Admin-Token": "read-tok"})
    wa = client.post("/api/cameras", json={"name": "y", "type": "file", "source": "s.mp4"},
                     headers={"X-Admin-Token": "adm-tok"})
    return rr.status_code, rr_read.status_code, rr_admin.status_code, wr.status_code, wa.status_code


def test_matrix_admin_empty_all_open(isolated):
    """admin 空 → 鉴权整体不启用（read_token 被忽略）。"""
    settings.admin_token = ""
    settings.read_token = "read-tok"
    settings.auth_read_protected = True
    with TestClient(app) as c:
        rr, rr_read, rr_admin, wr, wa = _matrix(c)
        assert (rr, rr_read, rr_admin) == (200, 200, 200)
        assert wr == 200 and wa == 200  # 全开放（含"read"头写——与未启用语义一致）


def test_matrix_admin_only_read_protected(isolated):
    settings.admin_token = "adm-tok"
    settings.read_token = ""
    settings.auth_read_protected = True
    with TestClient(app) as c:
        rr, rr_read, rr_admin, wr, wa = _matrix(c)
        assert rr == 401                     # 匿名读 → 401
        assert rr_read == 401                # read 头=空读令牌不通过
        assert rr_admin == 200
        assert wr == 401 and wa == 200


def test_matrix_admin_plus_read(isolated):
    settings.admin_token = "adm-tok"
    settings.read_token = "read-tok"
    settings.auth_read_protected = True
    with TestClient(app) as c:
        rr, rr_read, rr_admin, wr, wa = _matrix(c)
        assert rr == 401                          # 匿名读 → 401
        assert rr_read == 200                     # 只读令牌可读
        assert rr_admin == 200
        assert wr == 401                          # 只读令牌写 → 401
        assert wa == 200                          # 管理令牌写 → 200
    # 恢复默认
    settings.admin_token = ""
    settings.read_token = ""
    settings.auth_read_protected = False


def test_matrix_read_unprotected_stays_public(isolated):
    """auth_read_protected=false 但设了 read_token → GET 仍公开（文档口径）。"""
    settings.admin_token = "adm-tok"
    settings.read_token = "read-tok"
    settings.auth_read_protected = False
    with TestClient(app) as c:
        assert c.get("/api/cameras").status_code == 200  # 公开读
        assert c.post("/api/cameras", json={"name": "x", "type": "file", "source": "s.mp4"},
                      ).status_code == 401  # 写仍需管理令牌


def test_read_token_write_401_message(isolated):
    settings.admin_token = "adm-tok"
    settings.read_token = "read-tok"
    with TestClient(app) as c:
        r = c.post("/api/cameras", json={"name": "x", "type": "file", "source": "s.mp4"},
                   headers={"X-Admin-Token": "read-tok"})
        assert r.status_code == 401
        assert "无写权限" in r.json()["detail"]


def test_ws_read_token(isolated, monkeypatch):
    monkeypatch.setattr(settings, "admin_token", "adm-tok")
    monkeypatch.setattr(settings, "read_token", "read-tok")
    monkeypatch.setattr(settings, "auth_read_protected", True)  # 显式声明，不依赖前序测试状态
    with TestClient(app) as c:
        import pytest as _p
        with _p.raises(Exception):
            # 错误令牌 → WS 拒绝（4401）；TestClient 会抛 WebSocketDisconnect
            with c.websocket_connect("/ws/events?token=wrong"):
                pass
    with TestClient(app) as c:
        with c.websocket_connect("/ws/events?token=read-tok") as ws:
            pass  # 只读令牌可连


def test_same_token_warning(capsys):
    """read_token 与 admin_token 相同 → 启动时 WARNING（弱隔离提示）。"""
    from app import api as api_mod
    settings.admin_token = "same-tok"
    settings.read_token = "same-tok"
    api_mod._warn_same_token()
    out = capsys.readouterr().out
    assert "弱隔离" in out
    settings.admin_token = ""
    settings.read_token = ""
