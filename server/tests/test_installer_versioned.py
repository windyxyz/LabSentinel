"""T-404/T-405 回归：版本化安装归档/回滚 + 安全卸载守卫。"""
import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "scripts"))

import pytest  # noqa: E402

import install_portable as ip  # noqa: E402


def _fake_package(root: Path, version: str) -> Path:
    """最小便携包：runtime/python.exe + app/VERSION + scripts/portable_launcher.py。"""
    (root / "runtime").mkdir(parents=True)
    (root / "runtime" / "python.exe").write_bytes(b"x")
    (root / "app").mkdir()
    (root / "app" / "VERSION").write_text(version, encoding="ascii")
    (root / "scripts").mkdir()
    (root / "scripts" / "portable_launcher.py").write_text("# launcher", encoding="utf-8")
    return root


@pytest.mark.skipif(ip is None, reason="install_portable 不可导入")
def test_install_archive_and_rollback(tmp_path, monkeypatch):
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "LA"))
    (tmp_path / "LA").mkdir()
    src1 = _fake_package(tmp_path / "pkg1", "0.1.0")
    src2 = _fake_package(tmp_path / "pkg2", "0.2.0")
    install = tmp_path / "LA" / "Programs" / "LabSentinel"

    assert ip.do_install(src1, autostart=False, install_dir=install,
                         shortcuts=False, silent=True) == 0
    mark = ip._read_install_mark(install)
    assert mark and mark["version"] == "0.1.0"
    assert (install / ip.CURRENT_MARK).read_text("ascii").strip() == "0.1.0"

    # 升级：旧版归档 + current 切到新版
    assert ip.do_install(src2, autostart=False, install_dir=install,
                         shortcuts=False, silent=True) == 0
    assert (install / "versions" / "0.1.0" / "app" / "VERSION").exists()
    assert (install / ip.CURRENT_MARK).read_text("ascii").strip() == "0.2.0"

    # 回滚：版本树复制回安装根
    assert ip.rollback(install) == 0
    assert (install / "app" / "VERSION").read_text("ascii").strip() == "0.1.0"
    assert (install / ip.CURRENT_MARK).read_text("ascii").strip() == "0.1.0"


def test_uninstall_refuses_foreign_dir(tmp_path, monkeypatch):
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "LA"))
    (tmp_path / "LA").mkdir()
    foreign = tmp_path / "LA" / "Programs" / "LabSentinel"
    foreign.mkdir(parents=True)
    (foreign / "keep.txt").write_text("user data", encoding="utf-8")
    assert ip.do_uninstall(foreign, purge=False) == 2
    assert (foreign / "keep.txt").exists()  # 未删任何文件


def test_uninstall_accepts_marked_dir(tmp_path, monkeypatch):
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "LA"))
    (tmp_path / "LA").mkdir()
    src = _fake_package(tmp_path / "pkg", "0.2.0")
    install = tmp_path / "LA" / "Programs" / "LabSentinel"
    assert ip.do_install(src, autostart=False, install_dir=install,
                         shortcuts=False, silent=True) == 0
    (install / "user.txt").write_text("mine", encoding="utf-8")
    assert ip.do_uninstall(install, purge=False) == 0
    # 受管文件已清；用户自放文件保留（T-405 语义）
    assert (install / "user.txt").exists()
