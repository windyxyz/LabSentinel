"""单元测试：人脸模块（开关默认值/同意强制/降级路径/冷却/API 守卫）。运行: pytest tests/ -q"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import httpx  # noqa: E402
import numpy as np  # noqa: E402
import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import api as api_mod  # noqa: E402
from app.config import settings  # noqa: E402
from app.face import FaceClient, FaceGate  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402

FRAME = np.zeros((200, 200, 3), np.uint8)
BOX = (10, 10, 100, 100)


def _cf_transport(subject, sim):
    """模拟 CompreFace /recognition/faces 响应。subject=None 表示无人脸。"""
    def handler(request: httpx.Request) -> httpx.Response:
        faces = [{"subject": subject, "similarity": sim}] if subject else []
        return httpx.Response(200, json={"result": [{"faces": faces}]})
    return httpx.MockTransport(handler)


def _gate(monkeypatch, subject, sim, enabled=True, cool=0):
    monkeypatch.setattr(settings, "face_enabled", enabled)
    monkeypatch.setattr(settings, "face_cool_s", cool)
    client = FaceClient(api_key="k", transport=_cf_transport(subject, sim))
    return FaceGate(client)


def test_face_disabled_by_default():
    assert settings.face_enabled is False  # 合规红线：默认必须关闭
    gate = FaceGate(FaceClient(api_key="k"))  # 未 monkeypatch 全局 settings → 仍关闭
    assert gate.enabled is False
    assert gate.check(1, 1, FRAME, BOX) is None


def test_face_gate_known_unknown_cooldown(monkeypatch):
    # 陌生人（低于阈值）→ unknown
    gate = _gate(monkeypatch, subject="zzz", sim=0.3)
    assert gate.check(1, 7, FRAME, BOX) == "unknown"
    # 熟人 → known:<名字>
    gate2 = _gate(monkeypatch, subject="张三", sim=0.92)
    assert gate2.check(1, 7, FRAME, BOX) == "known:张三"
    # 同一 track 冷却期内不重复调（cool=30）→ None
    gate3 = _gate(monkeypatch, subject="张三", sim=0.92, cool=30)
    assert gate3.check(1, 7, FRAME, BOX) == "known:张三"
    assert gate3.check(1, 7, FRAME, BOX) is None
    # 不同 track 不受影响
    assert gate3.check(1, 8, FRAME, BOX) == "known:张三"


def test_face_gate_degrades_on_error(monkeypatch):
    """服务异常必须降级为 None（不得误报 stranger）。"""
    monkeypatch.setattr(settings, "face_enabled", True)

    def broken(request: httpx.Request) -> httpx.Response:
        raise httpx.ConnectError("compreface down")

    gate = FaceGate(FaceClient(api_key="k", transport=httpx.MockTransport(broken)))
    assert gate.check(1, 7, FRAME, BOX) is None
    # 未配置 key 同样降级
    gate2 = FaceGate(FaceClient(api_key=None))
    assert gate2.enabled is False
    assert gate2.check(1, 7, FRAME, BOX) is None


class FakeFaceClient:
    configured = True

    def __init__(self):
        self.persons = []

    def add_person(self, subject, data):
        self.persons.append(subject)

    def list_persons(self):
        return list(self.persons)

    def delete_person(self, subject):
        self.persons.remove(subject)


@pytest.fixture()
def client(monkeypatch, tmp_path):
    s = Store(path=tmp_path / "t.db")
    fake = FakeFaceClient()
    monkeypatch.setattr(api_mod, "store", s)
    monkeypatch.setattr(api_mod, "face_client", fake)
    c = TestClient(app)
    return c, s, fake, monkeypatch


def test_face_api_disabled_403(client):
    c, _, _, _ = client
    photo = {"photo": ("a.jpg", b"fakejpeg", "image/jpeg")}  # 过掉参数校验，只测功能守卫
    r = c.post("/api/face/persons", data={"subject": "x", "consent": "true"}, files=photo)
    assert r.status_code == 403
    r = c.get("/api/face/persons")
    assert r.status_code == 403
    r = c.delete("/api/face/persons/all")
    assert r.status_code == 403


def test_face_api_consent_enforced(client, monkeypatch):
    c, s, fake, monkeypatch = client
    monkeypatch.setattr(settings, "face_enabled", True)
    photo = {"photo": ("a.jpg", b"fakejpeg", "image/jpeg")}
    # 未勾选同意 → 400（合规红线）
    r = c.post("/api/face/persons", data={"subject": "张三", "consent": "false"}, files=photo)
    assert r.status_code == 400
    # 同意后注册成功，同意记录落库
    r = c.post("/api/face/persons", data={"subject": "张三", "consent": "true"},
               files={"photo": ("a.jpg", b"fakejpeg", "image/jpeg")})
    assert r.status_code == 200 and r.json()["subject"] == "张三"
    assert s.face_consent_ts("张三") is not None
    # 列表带同意时间
    r = c.get("/api/face/persons")
    assert r.json() == [{"subject": "张三", "consent_ts": s.face_consent_ts("张三")}]
    # 一键清除：人员 + 同意记录
    r = c.delete("/api/face/persons/all")
    assert r.status_code == 200
    assert fake.persons == [] and s.face_consents() == []
