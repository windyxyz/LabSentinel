"""R2-105/N-11 回归：文件夹系统目录判断改为"系统根包含判断"。

旧逻辑只检查 `real.parents` 的目录名：
- 绕过：C:\\Windows 本身（parents 只有 C:\\，name 为空）、/etc 直接通过；
- 误杀：任意父目录叫 library/bin/system 就拒绝，例如 D:\\data\\Library\\shots。
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402

from app import sources  # noqa: E402
from app.sources import SourceError, check_folder_root  # noqa: E402


@pytest.fixture
def no_system_roots(monkeypatch, tmp_path):
    """固定"系统根集合"为空，让"普通目录应放行"用例不受 TMP 位置影响。

    R2-105 会（正确地）拒绝落在 WINDIR / ProgramData / AppData 之下的目录。
    但 pytest 的 tmp_path 来自 TMP/TEMP：本机 TMP=c:\\windows\\temp 正好在
    WINDIR 下，CI 上 TEMP 又常落在 %LOCALAPPDATA%\\Temp 下——两种环境都会把
    测试自己的临时目录判成系统目录。下面两个用例只验证"父目录名黑名单已移除"
    与"allowed_roots 生效"，与具体系统根无关，故把系统根/AppData 固定成不可能
    包含 tmp_path 的取值，使断言在任何机器上都稳定。
    真正验证系统根拒绝的两个用例仍走真实环境（未应用本 fixture）。
    """
    monkeypatch.setattr(sources, "_system_roots", list)
    # 设为 tmp_path 的子目录（同级而非祖先），确保 check_folder_root 的 AppData
    # 分支不会把待放行的目录误判为配置目录。
    unrelated = str(tmp_path / "_not_appdata")
    monkeypatch.setenv("APPDATA", unrelated)
    monkeypatch.setenv("LOCALAPPDATA", unrelated)


def test_normal_user_folders_allowed(no_system_roots, tmp_path):
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


def test_allowed_roots_still_enforced(no_system_roots, tmp_path):
    root = tmp_path / "allowed"
    root.mkdir()
    inside = root / "in"
    inside.mkdir()
    outside = tmp_path / "elsewhere"
    outside.mkdir()
    assert check_folder_root(str(inside), allowed_roots=str(root)) == str(inside.resolve())
    with pytest.raises(SourceError, match="允许范围"):
        check_folder_root(str(outside), allowed_roots=str(root))
