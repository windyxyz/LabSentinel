"""T-404/T-405 回归：版本化安装归档/回滚 + 安全卸载守卫。"""
import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "scripts"))

import pytest  # noqa: E402

import platform

# install_portable/portable_launcher 是 Windows 安装器/启动器（reg/taskkill/
# powershell/creationflags），Linux CI 上这些依赖不存在——整文件跳过。
pytestmark = pytest.mark.skipif(platform.system() != "Windows",
                                reason="Windows 安装器/启动器功能")

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

def test_upgrade_health_fail_auto_rollback_and_success(tmp_path, monkeypatch):
    """T-404 VM 演练等价：v1 安装 → 写数据 → v2 健康检查故意失败 → 自动回到 v1
    且数据完好 → 再升级正常 v2 → 成功；升级前 DB 备份存在。"""
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "LA"))
    (tmp_path / "LA").mkdir()
    src1 = _fake_package(tmp_path / "pkg1", "0.1.0")
    src2 = _fake_package(tmp_path / "pkg2", "0.2.0")
    install = tmp_path / "LA" / "Programs" / "LabSentinel"
    data_dir = tmp_path / "LA" / "LabSentinel" / "data"
    data_dir.mkdir(parents=True)
    (data_dir / "labsentinel.db").write_bytes(b"DB-BEFORE-UPGRADE")
    (data_dir / "backups").mkdir()

    assert ip.do_install(src1, autostart=False, install_dir=install,
                         shortcuts=False, silent=True) == 0
    # 故意失败的健康检查（真子进程，退出码 1）
    fail = [sys.executable, "-c", "import sys; sys.exit(1)"]
    rc = ip.do_upgrade(src2, install, shortcuts=False, silent=True, health_cmd=fail)
    assert rc == 4, "健康检查失败应自动回滚并返回 4"
    assert (install / "app" / "VERSION").read_text("ascii").strip() == "0.1.0"
    assert (install / ip.CURRENT_MARK).read_text("ascii").strip() == "0.1.0"
    assert (data_dir / "labsentinel.db").read_bytes() == b"DB-BEFORE-UPGRADE"
    assert list((data_dir / "backups").glob("pre-upgrade-*.zip")) or \
        (data_dir / "labsentinel.db.pre-upgrade.bak").exists(), "升级前 DB 备份"

    # 再升级正常 v2（健康检查通过）
    ok = [sys.executable, "-c", "import sys; sys.exit(0)"]
    assert ip.do_upgrade(src2, install, shortcuts=False, silent=True, health_cmd=ok) == 0
    assert (install / "app" / "VERSION").read_text("ascii").strip() == "0.2.0"
    assert (install / ip.CURRENT_MARK).read_text("ascii").strip() == "0.2.0"
    assert (data_dir / "labsentinel.db").read_bytes() == b"DB-BEFORE-UPGRADE"

    # 卸载只删受管文件：用户自放文件保留
    (install / "user.txt").write_text("mine", encoding="utf-8")
    assert ip.do_uninstall(install, purge=False) == 0
    assert (install / "user.txt").exists()

def test_upgrade_from_zip_with_path_traversal_guard(tmp_path, monkeypatch):
    """T-404：upgrade --from <zip> 解包（含一层包根），穿越条目被跳过。"""
    import zipfile as _zip
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "LA"))
    (tmp_path / "LA").mkdir()
    src1 = _fake_package(tmp_path / "pkg1", "0.1.0")
    src2 = _fake_package(tmp_path / "pkg2", "0.2.0")
    install = tmp_path / "LA" / "Programs" / "LabSentinel"
    assert ip.do_install(src1, autostart=False, install_dir=install,
                         shortcuts=False, silent=True) == 0
    zp = tmp_path / "upgrade.zip"
    with _zip.ZipFile(zp, "w") as z:
        for p in src2.rglob("*"):
            if p.is_file():
                z.write(p, f"LabSentinel/{p.relative_to(src2)}")
        z.writestr("../../evil.txt", "bad")
        z.writestr("LabSentinel/../escape.txt", "bad")
    ok = [sys.executable, "-c", "import sys; sys.exit(0)"]
    assert ip.do_upgrade(zp, install, shortcuts=False, silent=True, health_cmd=ok) == 0
    assert (install / "app" / "VERSION").read_text("ascii").strip() == "0.2.0"
    assert not (tmp_path / "evil.txt").exists()
    assert not (install.parent / "escape.txt").exists()

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
