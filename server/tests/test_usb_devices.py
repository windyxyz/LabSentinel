"""单元测试：任务60 USB 设备枚举（parse_dshow / list_devices 降级与缓存 / API 鉴权口径）。
运行: pytest tests/test_usb_devices.py -q"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import usb_devices as usb  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402

SAMPLE = """[dshow @ 0000025a1f4a7ac0] "HD Webcam" (video)
[dshow @ 0000025a1f4a7ac0]  (vcodec=mjpeg  size=1280x720  fps=30)
[dshow @ 0000025a1f4a7ac0] "麦克风 (Realtek Audio)" (audio)
[dshow @ 0000025a1f4a7ac0] "OBS Virtual Camera" (video)
"""


def test_parse_two_videos_ignore_audio():
    d = usb.parse_dshow(SAMPLE)
    assert [x["name"] for x in d] == ["HD Webcam", "OBS Virtual Camera"]
    assert [x["index"] for x in d] == [0, 1]
    assert d[1]["source"] == "1"


def test_parse_chinese_quotes_spaces():
    txt = '[dshow @ abc123] "实验室 前门 \"4K\" 摄像头" (video)\n'
    d = usb.parse_dshow(txt)
    assert len(d) == 1 and d[0]["name"] == '实验室 前门 "4K" 摄像头'


def test_parse_empty():
    assert usb.parse_dshow("") == []
    assert usb.parse_dshow("no dshow lines here") == []


@pytest.fixture()
def fresh_cache():
    usb._cache.update(ts=0.0, data=None)
    yield
    usb._cache.update(ts=0.0, data=None)


def test_missing_ffmpeg_degrades(tmp_path, monkeypatch, fresh_cache):
    monkeypatch.setattr(usb, "FFMPEG", tmp_path / "nope" / "ffmpeg.exe")
    r = usb.list_devices()
    assert r["devices"] == [] and r["error"]  # 200 语义：空列表 + 可读原因


def test_cache_and_refresh(monkeypatch, fresh_cache):
    calls = {"n": 0}

    def fake_run(*a, **k):
        calls["n"] += 1

        class R:
            stderr = SAMPLE

        return R()

    monkeypatch.setattr(usb, "FFMPEG", __import__("pathlib").Path(__file__).resolve())  # 需真实存在，才会走到 subprocess
    monkeypatch.setattr(usb.subprocess, "run", fake_run)
    r1 = usb.list_devices()
    r2 = usb.list_devices()
    assert calls["n"] == 1 and len(r1["devices"]) == 2 and r2 is r1  # 命中缓存同一对象
    r3 = usb.list_devices(refresh=True)
    assert calls["n"] == 2 and len(r3["devices"]) == 2


def test_timeout_degrades(monkeypatch, fresh_cache):
    import subprocess as sp

    monkeypatch.setattr(usb, "FFMPEG", __import__("pathlib").Path(__file__).resolve())
    def fake_run(*a, **k):
        raise sp.TimeoutExpired(cmd="ffmpeg", timeout=10)
    monkeypatch.setattr(usb.subprocess, "run", fake_run)
    r = usb.list_devices(refresh=True)
    assert r["devices"] == [] and "超时" in r["error"]


def test_api_ok_and_auth(tmp_path, monkeypatch):
    monkeypatch.setattr(usb, "list_devices", lambda refresh=False: {
        "devices": [{"index": 0, "name": "HD Webcam", "source": "0"}], "error": None, "ffmpeg": "x"})
    import app.api as api_mod  # noqa: F401  （路由内 from . import usb_devices，monkeypatch 生效）
    # settings 打回默认（不启用读保护）
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "admin_token", "")
    with TestClient(app) as c:
        r = c.get("/api/cameras/usb-devices")
        assert r.status_code == 200
        assert r.json()["devices"][0]["name"] == "HD Webcam"
        r2 = c.get("/api/cameras/usb-devices?refresh=1")
        assert r2.status_code == 200
    # 读保护开 + 令牌启用 → 无令牌 401、带令牌 200
    monkeypatch.setattr(settings, "auth_read_protected", True)
    monkeypatch.setattr(settings, "admin_token", "tok60")
    with TestClient(app) as c:
        assert c.get("/api/cameras/usb-devices").status_code == 401
        assert c.get("/api/cameras/usb-devices", headers={"X-Admin-Token": "tok60"}).status_code == 200
