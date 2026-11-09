"""T-103 回归：source 白名单 + YAML 注入防护 + 允许根。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402

from app.sources import (SourceError, check_folder_root, startup_issues,  # noqa: E402
                         validate_source, yaml_stream_entry)


def test_rtsp_valid_and_injections_rejected():
    assert validate_source("rtsp", "rtsp://u:p@1.2.3.4/stream")
    assert validate_source("rtsp", "rtsps://1.2.3.4/stream")
    for bad in ("exec:calc", "ffmpeg:x#raw=-f", "rtsp://a#raw", 'rtsp://a" \n  "x": "exec:...',
                "rtsp://a b", "http://1.2.3.4/x"):
        with pytest.raises(SourceError):
            validate_source("rtsp", bad)


def test_file_and_usb_validation(tmp_path):
    f = tmp_path / "a.mp4"
    f.write_bytes(b"x")
    assert validate_source("file", str(f)).endswith("a.mp4")
    for bad in (str(tmp_path / "a.exe"), "x.mp4#raw", 'x.mp4"', "'quoted.mp4'"):
        with pytest.raises(SourceError):
            validate_source("file", bad)
    with pytest.raises(SourceError):
        validate_source("file", "x.mp4\n")  # 控制字符在 strip 前检查
    assert validate_source("usb", "7") == "7"
    for bad in ("abc", "100", "-1"):
        with pytest.raises(SourceError):
            validate_source("usb", bad)


def test_yaml_entry_escapes_injection():
    entry = yaml_stream_entry(1, 'rtsp://a/"b')
    assert entry.startswith("  ")
    # 双引号/换行被 JSON 转义，无法闭合 YAML 结构
    assert '\\"' in entry and "\n" not in entry
    import json
    assert json.loads("{" + entry.strip().rstrip(",") + "}") == {"1": 'rtsp://a/"b'}


def test_folder_system_roots_rejected(tmp_path, monkeypatch):
    # 本机 TEMP 落在 C:\Windows\Temp（被系统目录黑名单覆盖，产品规则正确）；
    # 测试把环境变量指向 D:/tmp 旁路位置，隔离产品规则与本用例意图
    import secrets
    from pathlib import Path as _P
    base = _P("D:/tmp") / ("ls-folder-" + secrets.token_hex(4))
    base.mkdir(parents=True, exist_ok=True)
    monkeypatch.setenv("APPDATA", str(base / "appdata"))
    monkeypatch.setenv("LOCALAPPDATA", str(base / "localappdata"))
    good = base / "shots"
    good.mkdir()
    assert check_folder_root(str(good)) == str(good.resolve())
    # 允许根模式
    assert check_folder_root(str(good), str(base)) == str(good.resolve())
    with pytest.raises(SourceError):
        check_folder_root(str(good), str(base / "other"))
    # R2-105：系统根判断基于真实系统路径，不再按"任意父目录名"误杀。
    # 用户目录下叫 Windows/Web 的普通目录应当通过；真实系统根由
    # test_folder_system_roots.py 单独验证。
    sysdir = base / "Windows" / "Web"
    sysdir.mkdir(parents=True)
    assert check_folder_root(str(sysdir)) == str(sysdir.resolve())
    import shutil
    shutil.rmtree(base, ignore_errors=True)


def test_startup_issues_reports_bad_source():
    issues = startup_issues([
        {"id": 1, "name": "ok", "type": "usb", "source": "0"},
        {"id": 2, "name": "bad", "type": "rtsp", "source": "exec:calc"},
    ])
    assert len(issues) == 1 and "bad" in issues[0]
