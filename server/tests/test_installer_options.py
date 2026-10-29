"""单元测试：任务71 安装器参数化（--install-dir/--silent/--no-shortcuts/--dry-run/--force）。
全部隔离：LOCALAPPDATA/APPDATA 指向临时目录，绝不写真实 HKCU/桌面。"""
import importlib.util
import os
import sys
from pathlib import Path

import pytest

SPEC = importlib.util.spec_from_file_location(
    "install_portable", os.path.join(os.path.dirname(__file__), "..", "..", "scripts",
                                     "install_portable.py"))
ip = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ip)


def make_pkg(tmp_path: Path) -> Path:
    src = tmp_path / "pkg"
    (src / "runtime").mkdir(parents=True)
    (src / "runtime" / "python.exe").write_bytes(b"x")
    (src / "runtime" / "pythonw.exe").write_bytes(b"x")
    (src / "scripts").mkdir()
    (src / "scripts" / "portable_launcher.py").write_text("#", encoding="ascii")
    (src / "app").mkdir()
    (src / "app" / "VERSION").write_text("2099.01", encoding="ascii")
    (src / "启动.bat").write_text("@echo off\r\n", encoding="ascii")
    return src


@pytest.fixture()
def isolated(tmp_path, monkeypatch):
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "localappdata"))
    monkeypatch.setenv("APPDATA", str(tmp_path / "appdata"))
    (tmp_path / "localappdata").mkdir()
    (tmp_path / "appdata").mkdir()
    monkeypatch.setattr(ip, "desktop_dir", lambda: tmp_path / "desktop")
    yield tmp_path


def run(argv):
    old = sys.argv
    sys.argv = ["install_portable.py", *argv]
    try:
        return ip.main()
    finally:
        sys.argv = old


def test_dry_run_touches_nothing(isolated, capsys):
    src = make_pkg(isolated)
    target = isolated / "prog" / "LabSentinel"
    rc = run(["install", "--source", str(src), "--install-dir", str(target), "--dry-run"])
    assert rc == 0
    assert not target.exists()  # 不写任何文件
    out = capsys.readouterr().out
    assert "dry-run" in out and str(target) in out and "开机自启：不开" in out


def test_install_custom_dir_no_shortcuts(isolated, monkeypatch, capsys):
    src = make_pkg(isolated)
    called = {"ps": 0}
    monkeypatch.setattr(ip, "ps", lambda script: called.__setitem__("ps", called["ps"] + 1))
    target = isolated / "prog" / "LabSentinel"
    rc = run(["install", "--source", str(src), "--install-dir", str(target),
              "--no-shortcuts", "--no-autostart"])
    assert rc == 0
    assert (target / "runtime" / "python.exe").exists()
    assert (target / "app" / "VERSION").read_text(encoding="ascii").strip() == "2099.01"
    assert (target / "卸载.bat").exists()
    assert called["ps"] == 0  # 未生成快捷方式
    assert "lab" not in os.listdir(isolated / "localappdata" / "Programs") if \
        (isolated / "localappdata" / "Programs").exists() else True
    out = capsys.readouterr().out
    assert "2099.01" in out and "数据目录" in out  # 完成摘要（含版本/数据目录/卸载方式）


def test_upgrade_preserves_data(isolated):
    src = make_pkg(isolated)
    target = isolated / "prog" / "LabSentinel"
    assert run(["install", "--source", str(src), "--install-dir", str(target),
                "--no-shortcuts", "--no-autostart"]) == 0
    marker = isolated / "localappdata" / "LabSentinel" / "data" / "user-file.txt"
    marker.parent.mkdir(parents=True, exist_ok=True)
    marker.write_text("用户数据", encoding="utf-8")
    assert run(["install", "--source", str(src), "--install-dir", str(target),
                "--no-shortcuts", "--no-autostart"]) == 0  # 覆盖升级
    assert marker.read_text(encoding="utf-8") == "用户数据"  # 数据目录不动


def test_dangerous_and_foreign_dirs_rejected(isolated):
    src = make_pkg(isolated)
    win = Path(os.environ.get("WINDIR", r"C:\Windows"))
    assert run(["install", "--source", str(src), "--install-dir", str(win)]) == 2
    assert run(["install", "--source", str(src), "--install-dir", "relative/path"]) == 2
    foreign = isolated / "foreign"
    foreign.mkdir()
    (foreign / "somefile.txt").write_text("x", encoding="ascii")
    assert run(["install", "--source", str(src), "--install-dir", str(foreign)]) == 2  # 非安装目录拒绝
    assert run(["install", "--source", str(src), "--install-dir", str(foreign), "--force",
                "--no-shortcuts", "--no-autostart"]) == 0  # --force 放行
    assert (foreign / "runtime" / "python.exe").exists()


def test_shortcut_dirs_overridden(isolated, monkeypatch):
    src = make_pkg(isolated)
    called = {"lnk": []}
    orig = ip.ps

    def spy(script):
        called["lnk"].append(script)
        return orig(script)

    monkeypatch.setattr(ip, "ps", spy)
    desk = isolated / "mydesk"
    smenu = isolated / "mysmenu"
    desk.mkdir()
    smenu.mkdir()  # 真实桌面天然存在；覆盖目录需自备（安装器不代建桌面）
    rc = run(["install", "--source", str(src),
              "--install-dir", str(isolated / "prog" / "LabSentinel"),
              "--desktop-dir", str(desk), "--start-menu-dir", str(smenu)])
    assert rc == 0
    assert (desk / "LabSentinel.lnk").exists()
    assert (smenu / "LabSentinel.lnk").exists()


def test_mutual_exclusive_autostart(isolated, capsys):
    src = make_pkg(isolated)
    rc = run(["install", "--source", str(src), "--install-dir", str(isolated / "p2"),
              "--autostart", "--no-autostart"])
    assert rc == 2
    assert "互斥" in capsys.readouterr().out
