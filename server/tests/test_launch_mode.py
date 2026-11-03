"""单元测试：任务120 安装/便携双形态驱动逻辑 + 注册表卸载项。

运行: pytest tests/test_launch_mode.py -q
"""
import importlib.util
import os
import subprocess
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "portable_launcher", Path(__file__).resolve().parents[2] / "scripts" / "portable_launcher.py")
launcher = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(launcher)

import install_portable  # noqa: E402  随 launcher 一起加载（同目录）


def make_install(tmp_path: Path) -> Path:
    inst = tmp_path / "Programs" / "LabSentinel"
    (inst / "runtime").mkdir(parents=True)
    (inst / "LabSentinel.exe").write_bytes(b"x")
    return inst


# ---- decide_launch_mode（纯函数）----

def test_unattended_always_portable(tmp_path):
    """无人值守（冒烟/自动化）绝不转启安装副本——否则包冒烟会被真机残留安装劫持。"""
    inst = make_install(tmp_path)
    mode = tmp_path / "launch.mode"
    assert launcher.decide_launch_mode(inst, mode, unattended=True) == "portable"


def test_interactive_installed_redirects(tmp_path):
    inst = make_install(tmp_path)
    mode = tmp_path / "launch.mode"
    assert launcher.decide_launch_mode(inst, mode, unattended=False) == "installed"


def test_installed_copy_of_itself_never_relaunches(tmp_path):
    """2026-10-31 VM 实测 fork 炸弹回归：安装副本双击时 installed==自己 → 必须按便携启动，
    否则每世代转启自己 → 无限接力 + 每代闪一个 reg.exe 黑窗。"""
    inst = make_install(tmp_path)
    mode = tmp_path / "launch.mode"
    assert launcher.decide_launch_mode(inst, mode, unattended=False, app_root=inst) == "portable"


def test_installed_redirects_other_root(tmp_path):
    inst = make_install(tmp_path)
    other = tmp_path / "pkg-other"
    other.mkdir()
    assert launcher.decide_launch_mode(inst, tmp_path / "m", unattended=False,
                                       app_root=other) == "installed"


def test_interactive_mode_file_portable(tmp_path):
    mode = tmp_path / "launch.mode"
    mode.write_text("portable", encoding="ascii")
    assert launcher.decide_launch_mode(None, mode, unattended=False) == "portable"


def test_interactive_no_install_no_mode_asks(tmp_path):
    assert launcher.decide_launch_mode(None, tmp_path / "none.mode", unattended=False) == "ask"


# ---- installed_location / 注册表 ----

def test_installed_location_valid(tmp_path, monkeypatch):
    inst = make_install(tmp_path)

    def fake_run(cmd, **kw):
        class R:
            returncode = 0
            stdout = f"    InstallLocation    REG_SZ    {inst}\\"
        return R()

    monkeypatch.setattr(install_portable.subprocess, "run", fake_run)
    assert install_portable.installed_location() == inst


def test_installed_location_rejects_missing_exe(tmp_path, monkeypatch):
    """注册表指向的目录没有 LabSentinel.exe（半删残留）→ 视为未安装。"""
    inst = tmp_path / "Programs" / "LabSentinel"  # 目录不存在
    monkeypatch.setattr(
        install_portable.subprocess, "run",
        lambda cmd, **kw: type("R", (), {"returncode": 0,
                                         "stdout": f"    InstallLocation    REG_SZ    {inst}\\"})())
    assert install_portable.installed_location() is None


def test_installed_location_reg_failure(tmp_path, monkeypatch):
    monkeypatch.setattr(install_portable.subprocess, "run",
                        lambda cmd, **kw: (_ for _ in ()).throw(OSError("no reg")))
    assert install_portable.installed_location() is None


def test_set_uninstall_registry_all_keys(tmp_path, monkeypatch):
    inst = make_install(tmp_path)
    calls = []

    def fake_run(cmd, **kw):
        calls.append(cmd)
        return type("R", (), {"returncode": 0, "stdout": "", "stderr": ""})()

    monkeypatch.setattr(install_portable.subprocess, "run", fake_run)
    ok = install_portable.set_uninstall_registry(inst, "2099.01")
    assert ok is True
    names = [c[c.index("/v") + 1] for c in calls]
    for expected in ("DisplayName", "DisplayVersion", "InstallLocation",
                     "UninstallString", "DisplayIcon", "NoModify", "NoRepair"):
        assert expected in names
    assert install_portable.UNINSTALL_KEY in calls[0]
    ver = next(c for c in calls if c[c.index("/v") + 1] == "DisplayVersion")
    assert ver[-2] == "2099.01"  # [/d, 2099.01, /f]


def test_set_uninstall_registry_partial_failure(tmp_path, monkeypatch):
    inst = make_install(tmp_path)
    n = {"i": 0}

    def fake_run(cmd, **kw):
        n["i"] += 1
        rc = 1 if n["i"] == 2 else 0  # 第二个键写失败
        return type("R", (), {"returncode": rc, "stdout": "", "stderr": "access denied"})()

    monkeypatch.setattr(install_portable.subprocess, "run", fake_run)
    assert install_portable.set_uninstall_registry(inst, "2099.01") is False


def test_remove_uninstall_registry(tmp_path, monkeypatch):
    seen = []
    monkeypatch.setattr(install_portable.subprocess, "run",
                        lambda cmd, **kw: seen.append(cmd) or type("R", (), {"returncode": 0})())
    install_portable.remove_uninstall_registry()
    assert seen and seen[0][0] == "reg" and "/f" in seen[0]


# ---- 卸载停服三级降级（热修回归）----

def test_uninstall_stop_fallback_taskkill(tmp_path, monkeypatch):
    """exe 缺失 + runtime python spawn 抛 PermissionError → taskkill 兜底，卸载不中断。"""
    src = tmp_path / "pkg"
    (src / "runtime").mkdir(parents=True)
    (src / "runtime" / "python.exe").write_bytes(b"x")
    (src / "runtime" / "pythonw.exe").write_bytes(b"x")
    (src / "scripts").mkdir()
    (src / "scripts" / "portable_launcher.py").write_text("#", encoding="ascii")
    (src / "app").mkdir()
    (src / "app" / "VERSION").write_text("2099.01", encoding="ascii")

    la = tmp_path / "localappdata"
    (la / "LabSentinel" / "logs").mkdir(parents=True)
    (la / "LabSentinel" / "logs" / "launcher.lock").write_text("123 8000", encoding="ascii")
    monkeypatch.setenv("LOCALAPPDATA", str(la))
    monkeypatch.setenv("APPDATA", str(tmp_path / "appdata"))
    monkeypatch.setattr(install_portable, "desktop_dir", lambda: tmp_path / "desktop")
    monkeypatch.setattr(install_portable, "set_autostart", lambda i, on: None)
    monkeypatch.setattr(install_portable, "set_uninstall_registry", lambda i, v: True)
    monkeypatch.setattr(install_portable, "remove_uninstall_registry", lambda: None)

    real_run = install_portable.subprocess.run
    killed = []

    def fake_run(cmd, **kw):
        c = [str(x) for x in cmd]
        # LabSentinel.exe 不存在 → 走 python 分支：spawn 即 PermissionError
        if c[0].endswith("python.exe") and c[1].endswith("portable_launcher.py"):
            raise PermissionError(5, "拒绝访问。")
        if c[:2] == ["taskkill", "/PID"] and "/F" in c:
            killed.append(c)
        return type("R", (), {"returncode": 0, "stdout": "", "stderr": ""})()

    monkeypatch.setattr(install_portable.subprocess, "run", fake_run)
    monkeypatch.setattr(install_portable.time, "sleep", lambda s: None)
    rc = install_portable.do_uninstall(src, purge=False)
    assert rc == 0
    assert killed, "兜底 taskkill 未执行"


def test_relaunch_installed_success(tmp_path, monkeypatch):
    inst = make_install(tmp_path)
    opened = []
    monkeypatch.setattr(launcher.subprocess, "Popen",
                        lambda cmd, **kw: opened.append(cmd) or type("R", (), {"pid": 1})())
    assert launcher.relaunch_installed(inst) == 0
    assert opened and str(opened[0][0]) == str(inst / "LabSentinel.exe")


def test_relaunch_installed_falls_back(tmp_path, monkeypatch):
    """转启失败 → 返回 -1（调用方继续便携启动，不留死路）。"""
    inst = make_install(tmp_path)

    def boom(*a, **kw):
        raise OSError("target gone")

    monkeypatch.setattr(launcher.subprocess, "Popen", boom)
    assert launcher.relaunch_installed(inst) == -1
