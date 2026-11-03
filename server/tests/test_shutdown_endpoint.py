"""单元测试：任务120 网页端停止服务（POST /api/system/shutdown）。

运行: pytest tests/test_shutdown_endpoint.py -q
"""
import os
from pathlib import Path

import pytest

from app import api
from app.config import settings
from fastapi.testclient import TestClient


@pytest.fixture()
def client(tmp_path, monkeypatch):
    data = tmp_path / "data"
    data.mkdir(exist_ok=True)  # conftest autouse 可能已建
    monkeypatch.setattr(settings, "data_dir", data)
    from app.main import app
    return TestClient(app), data


def test_shutdown_endpoint_schedules_exit(client, monkeypatch):
    c, _data = client
    called = []
    monkeypatch.setattr(api, "_do_shutdown", lambda: called.append(1))
    r = c.post("/api/system/shutdown")
    assert r.status_code == 200
    assert r.json()["ok"] is True
    assert called, "退出线程未被调度"


def test_do_shutdown_removes_launcher_lock(client, monkeypatch):
    """退出前清启动器锁（便携布局：data 上一级 logs/launcher.lock）。"""
    c, data = client
    lock = data.parent / "logs" / "launcher.lock"
    lock.parent.mkdir(parents=True)
    lock.write_text("123 8000", encoding="ascii")

    exited = []
    monkeypatch.setattr(api, "SHUTDOWN_DELAY_S", 0)
    monkeypatch.setattr(os, "_exit", lambda code: exited.append(code))
    api._do_shutdown()
    assert exited == [0]
    assert not lock.exists()


def test_do_shutdown_no_lock_no_crash(client, monkeypatch):
    """源码模式锁路径不存在 → 静默退出不抛异常。"""
    c, data = client
    exited = []
    monkeypatch.setattr(api, "SHUTDOWN_DELAY_S", 0)
    monkeypatch.setattr(os, "_exit", lambda code: exited.append(code))
    api._do_shutdown()
    assert exited == [0]
