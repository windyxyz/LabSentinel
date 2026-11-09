"""R2-105/N-11 回归：文件夹系统目录判断改为"系统根包含判断"。

旧逻辑只检查 `real.parents` 的目录名：
- 绕过：C:\\Windows 本身（parents 只有 C:\\，name 为空）、/etc 直接通过；
- 误杀：任意父目录叫 library/bin/system 就拒绝，例如 D:\\data\\Library\\shots。
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402

from app.sources import SourceError, check_folder_root  # noqa: E402


def test_normal_user_folders_allowed(tmp_path):
    d = tmp_path / "Library" / "shots"
    d.mkdir(parents=True)
    assert check_folder_root(str(d)) == str(d.resolve())

    d2 = tmp_path / "data" / "bin" / "system" / "pics"
    d2.mkdir(parents=True)
    assert check_folder_root(str(d2)) == str(d2.resolve())


def test_system_root_itself_rejected():
    """系统根目录本身必须被拒（旧逻辑只看 parents 会漏）。"""
    if os.name == "nt":
        windir = os.environ.get("WINDIR") or r"C:\Windows"
        if os.path.isdir(windir):
            with pytest.raises(SourceError, match="系统目录"):
                check_folder_root(windir)
        programdata = os.environ.get("ProgramData")
        if programdata and os.path.isdir(programdata):
            with pytest.raises(SourceError, match="系统目录"):
                check_folder_root(programdata)
    else:
        for p in ("/etc", "/usr", "/bin"):
            if os.path.isdir(p):
                with pytest.raises(SourceError, match="系统目录"):
                    check_folder_root(p)


def test_child_of_system_root_rejected():
    if os.name == "nt":
        windir = os.environ.get("WINDIR") or r"C:\Windows"
        child = os.path.join(windir, "Web")
        if os.path.isdir(child):
            with pytest.raises(SourceError, match="系统目录"):
                check_folder_root(child)
    else:
        if os.path.isdir("/etc/ssl"):
            with pytest.raises(SourceError, match="系统目录"):
                check_folder_root("/etc/ssl")


def test_allowed_roots_still_enforced(tmp_path):
    root = tmp_path / "allowed"
    root.mkdir()
    inside = root / "in"
    inside.mkdir()
    outside = tmp_path / "elsewhere"
    outside.mkdir()
    assert check_folder_root(str(inside), allowed_roots=str(root)) == str(inside.resolve())
    with pytest.raises(SourceError, match="允许范围"):
        check_folder_root(str(outside), allowed_roots=str(root))
