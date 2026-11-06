"""T-601/T-602 平台抽象层回归：Windows 行为等价 + POSIX 分支可达。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app import platform as plat  # noqa: E402
from app.platform import binaries, devices, fonts, paths, power, process  # noqa: E402


def test_platform_identity():
    assert plat.platform_name() in ("windows", "macos", "linux")
    assert plat.arch() in ("x64", "arm64")
    assert plat.is_windows() == (os.name == "nt")


def test_find_binary_known():
    ff = binaries.find_binary("ffmpeg")  # 本机 tools/ffmpeg 或 PATH
    assert ff is None or os.path.isfile(ff)


def test_fonts_prefers_builtin_then_platform():
    p = fonts.cjk_font_path(None)
    if p:
        assert os.path.isfile(p)
    # 显式配置优先
    here = os.path.abspath(__file__)
    assert fonts.cjk_font_path(here) == here


def test_paths_env_override(monkeypatch, tmp_path):
    monkeypatch.setenv("LS_DATA_DIR", str(tmp_path / "d"))
    assert paths.data_dir() == tmp_path / "d"
    monkeypatch.delenv("LS_DATA_DIR")
    assert "labsentinel" in str(paths.data_dir()).lower() or "LabSentinel" in str(paths.data_dir())


def test_process_spawn_hidden_and_identity():
    p = process.spawn_hidden([sys.executable, "-c", "import time; time.sleep(3)"],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        assert p.poll() is None
        # 命令行含 python -c 不匹配默认 markers → False（防误杀语义）
        assert process.is_our_process(p.pid) is False
    finally:
        process.kill_tree(p.pid)
        p.wait(timeout=5)


def test_power_status_shape():
    s = power.status()
    assert "active" in s


def test_devices_returns_list():
    r = devices.list_cameras()
    if isinstance(r, dict):  # Windows 委托 usb_devices：{devices, error, ffmpeg}
        assert isinstance(r.get("devices"), list)
    else:  # POSIX：[{index, name}]
        assert isinstance(r, list)


import subprocess  # noqa: E402  (test helper 用)
