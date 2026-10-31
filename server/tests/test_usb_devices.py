"""单元测试：任务60 USB 设备枚举（parse_dshow / list_devices 降级与缓存 / API 鉴权口径）
+ 任务75 ffmpeg 路径解析器（源码/便携双布局、LS_FFMPEG、全缺失报错）。"""
import os
import sys
from pathlib import Path

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


def make_ffmpeg(tmp_path: Path, layout: str) -> Path:
    """按布局造 ffmpeg.exe：source=<根>/tools/ffmpeg/，portable=<根>/tools/ 扁平。"""
    d = tmp_path / "tools" / ("ffmpeg" if layout == "source" else "")
    d.mkdir(parents=True, exist_ok=True)
    f = d / "ffmpeg.exe"
    f.write_bytes(b"x")
    return f


def test_resolve_ffmpeg_layouts(tmp_path, monkeypatch):
    """75.1：源码布局 / 便携扁平布局 / LS_FFMPEG 覆盖 / 全缺失。"""
    monkeypatch.delenv("LS_FFMPEG", raising=False)
    # 源码布局
    make_ffmpeg(tmp_path / "src", "source")
    hit, tried = usb.resolve_ffmpeg(tmp_path / "src")
    assert hit == tmp_path / "src" / "tools" / "ffmpeg" / "ffmpeg.exe"
    # 便携扁平布局
    make_ffmpeg(tmp_path / "pkg", "portable")
    hit, _ = usb.resolve_ffmpeg(tmp_path / "pkg")
    assert hit == tmp_path / "pkg" / "tools" / "ffmpeg.exe"
    # LS_FFMPEG 优先（存在才用）
    f = make_ffmpeg(tmp_path / "env", "portable")
    monkeypatch.setenv("LS_FFMPEG", str(f))
    hit, _ = usb.resolve_ffmpeg(tmp_path / "pkg")
    assert hit == f
    # 全缺失：返回第一个候选用于报错，且列出尝试路径
    monkeypatch.delenv("LS_FFMPEG")
    empty = tmp_path / "empty"
    empty.mkdir()
    hit, tried = usb.resolve_ffmpeg(empty)
    assert not hit.exists() and len(tried) >= 3


def test_detect_root_portable_layout(tmp_path):
    """75 复验抓到的 bug：便携布局 pkg/app/server/app/usb_devices.py 向上推导必须命中 pkg，
    不能误停在 pkg/app（它下面有 server/ 子目录）。"""
    import importlib.util
    app_dir = tmp_path / "pkg" / "app" / "server" / "app"
    app_dir.mkdir(parents=True)
    (tmp_path / "pkg" / "runtime").mkdir()
    (tmp_path / "pkg" / "runtime" / "python.exe").write_bytes(b"x")
    (app_dir / "usb_devices.py").write_bytes(
        (Path(__file__).resolve().parents[1] / "app" / "usb_devices.py").read_bytes())
    spec = importlib.util.spec_from_file_location("usb_pkg_probe", app_dir / "usb_devices.py")
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    assert m.detect_root() == tmp_path / "pkg"


def test_missing_ffmpeg_degrades(tmp_path, monkeypatch, fresh_cache):
    empty = tmp_path / "no-tools"
    empty.mkdir()
    monkeypatch.delenv("LS_FFMPEG", raising=False)
    r = usb.list_devices(root=empty)
    assert r["devices"] == [] and r["error"]  # 200 语义：空列表 + 可读原因
    assert "尝试过" in r["error"] and "LS_FFMPEG" in r["error"]  # 列出尝试路径


def test_cache_and_refresh(tmp_path, monkeypatch, fresh_cache):
    calls = {"n": 0}

    def fake_run(*a, **k):
        calls["n"] += 1

        class R:
            stderr = SAMPLE

        return R()

    f = make_ffmpeg(tmp_path, "portable")  # 需真实存在，才会走到 subprocess
    monkeypatch.setattr(usb.subprocess, "run", fake_run)
    r1 = usb.list_devices(root=tmp_path)
    r2 = usb.list_devices(root=tmp_path)
    assert calls["n"] == 1 and len(r1["devices"]) == 2 and r2 is r1  # 命中缓存同一对象
    r3 = usb.list_devices(refresh=True, root=tmp_path)
    assert calls["n"] == 2 and len(r3["devices"]) == 2
    assert r1["ffmpeg"] == str(f)


def test_timeout_degrades(tmp_path, monkeypatch, fresh_cache):
    import subprocess as sp

    make_ffmpeg(tmp_path, "portable")

    def fake_run(*a, **k):
        raise sp.TimeoutExpired(cmd="ffmpeg", timeout=10)
    monkeypatch.setattr(usb.subprocess, "run", fake_run)
    r = usb.list_devices(refresh=True, root=tmp_path)
    assert r["devices"] == [] and "超时" in r["error"]


def test_api_ok_and_auth(tmp_path, monkeypatch):
    monkeypatch.setattr(usb, "list_devices", lambda refresh=False, root=None: {
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
    # 读保护开 + 令牌启用 → 无令牌 401、带令牌/header/query 均 200
    monkeypatch.setattr(settings, "auth_read_protected", True)
    monkeypatch.setattr(settings, "admin_token", "tok60")
    with TestClient(app) as c:
        assert c.get("/api/cameras/usb-devices").status_code == 401
        assert c.get("/api/cameras/usb-devices", headers={"X-Admin-Token": "tok60"}).status_code == 200
        assert c.get("/api/cameras/usb-devices", params={"token": "tok60"}).status_code == 200
