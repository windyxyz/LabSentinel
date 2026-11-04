"""单元测试：任务110 停止进程 PID 复用防护 + 冒烟残留收尾（全部 mock，不真启/杀进程）。
运行: pytest tests/test_stop_pid_safety.py -q"""
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "scripts"))

import pytest  # noqa: E402

import portable_launcher as pl  # noqa: E402
import smoke_portable as sp  # noqa: E402


# ---------- _pid_matches_launcher（monkeypatch PowerShell 输出） ----------

def _fake_ps(stdout="", returncode=0, raise_exc=None):
    def run(*a, **kw):
        if raise_exc is not None:
            raise raise_exc
        return subprocess.CompletedProcess(a[0] if a else [], returncode, stdout=stdout, stderr="")
    return run


def test_match_uvicorn_server_process(monkeypatch):
    monkeypatch.setattr(pl.subprocess, "run", _fake_ps(
        "python.exe|\"C:\\pkg\\runtime\\python.exe\" -m uvicorn app.main:app --port 8000\n"))
    assert pl._pid_matches_launcher(4242) is True


def test_match_launcher_pythonw(monkeypatch):
    monkeypatch.setattr(pl.subprocess, "run", _fake_ps(
        "pythonw.exe|\"C:\\pkg\\runtime\\pythonw.exe\" \"C:\\pkg\\scripts\\portable_launcher.py\" --stop"))
    assert pl._pid_matches_launcher(17) is True


def test_reject_foreign_process_name(monkeypatch):
    monkeypatch.setattr(pl.subprocess, "run", _fake_ps(
        "chrome.exe|\"C:\\...\\chrome.exe\" --type=renderer"))
    assert pl._pid_matches_launcher(999) is False


def test_reject_python_without_marker(monkeypatch):
    """PID 复用成另一个无关 python 进程：名字匹配但命令行无 LabSentinel 标记 → 拒绝。"""
    monkeypatch.setattr(pl.subprocess, "run", _fake_ps(
        "python.exe|\"C:\\Python312\\python.exe\" -m pip install requests"))
    assert pl._pid_matches_launcher(999) is False


def test_unknown_on_empty_output(monkeypatch):
    monkeypatch.setattr(pl.subprocess, "run", _fake_ps(""))
    assert pl._pid_matches_launcher(999) is None


def test_unknown_on_ps_failure(monkeypatch):
    monkeypatch.setattr(pl.subprocess, "run", _fake_ps(returncode=1))
    assert pl._pid_matches_launcher(999) is None


def test_unknown_on_timeout(monkeypatch):
    monkeypatch.setattr(pl.subprocess, "run",
                        _fake_ps(raise_exc=subprocess.TimeoutExpired(cmd="ps", timeout=5)))
    assert pl._pid_matches_launcher(999) is None


def test_unknown_on_oserror(monkeypatch):
    monkeypatch.setattr(pl.subprocess, "run", _fake_ps(raise_exc=OSError("no powershell")))
    assert pl._pid_matches_launcher(999) is None


# ---------- do_stop 分支 ----------

@pytest.fixture()
def stop_env(tmp_path, monkeypatch):
    lock = tmp_path / "launcher.lock"
    lock.write_text("4242 8180", encoding="ascii")
    monkeypatch.setattr(pl, "setup_logging", lambda: None)
    monkeypatch.setattr(pl, "LOCK", lock)
    calls: list[list[str]] = []

    def rec_run(cmd, *a, **kw):
        calls.append(list(cmd))
        return subprocess.CompletedProcess(cmd, 0)

    monkeypatch.setattr(pl.subprocess, "run", rec_run)
    return {"lock": lock, "calls": calls}


def test_do_stop_kills_only_on_match(stop_env, monkeypatch):
    monkeypatch.setattr(pl, "pid_alive", lambda pid: True)
    monkeypatch.setattr(pl, "_pid_matches_launcher", lambda pid: True)
    rc = pl.do_stop()
    assert rc == 0
    assert stop_env["calls"], "匹配时必须调用 taskkill"
    assert any("/PID" in c and "4242" in c for c in stop_env["calls"])
    assert not stop_env["lock"].exists()  # 停止后删锁


def test_do_stop_refuses_and_deletes_lock_on_mismatch(stop_env, monkeypatch):
    monkeypatch.setattr(pl, "pid_alive", lambda pid: True)
    monkeypatch.setattr(pl, "_pid_matches_launcher", lambda pid: False)
    rc = pl.do_stop()
    assert rc == 0
    assert stop_env["calls"] == [], "不匹配时绝不 taskkill"
    assert not stop_env["lock"].exists()  # 陈旧锁清除


def test_do_stop_refuses_and_keeps_lock_on_unknown(stop_env, monkeypatch):
    """查询失败：锁保留（下次重试仍可正常停止），打印锁路径，不杀。"""
    monkeypatch.setattr(pl, "pid_alive", lambda pid: True)
    monkeypatch.setattr(pl, "_pid_matches_launcher", lambda pid: None)
    rc = pl.do_stop()
    assert rc == 0
    assert stop_env["calls"] == []
    assert stop_env["lock"].exists()  # 身份不明时锁不能删——否则正常服务变孤儿


def test_do_stop_stale_lock_without_ps_query(stop_env, monkeypatch):
    """PID 已死：纯陈旧锁，直接清理，不应发起 PowerShell 查询。"""
    monkeypatch.setattr(pl, "pid_alive", lambda pid: False)

    def boom(pid):
        raise AssertionError("进程已死不应查询身份")

    monkeypatch.setattr(pl, "_pid_matches_launcher", boom)
    rc = pl.do_stop()
    assert rc == 0
    assert stop_env["calls"] == []
    assert not stop_env["lock"].exists()


def test_do_stop_no_lock(stop_env):
    stop_env["lock"].unlink()
    assert pl.do_stop() == 0
    assert stop_env["calls"] == []


# ---------- smoke reap_leftover_pids（假 PID/假 alive/假 killer，不真杀） ----------

def test_reap_already_dead_no_kill():
    killer_calls = []
    out = sp.reap_leftover_pids([4242], wait_s=0, alive=lambda p: False,
                                killer=lambda p: killer_calls.append(p))
    assert out == [] and killer_calls == []


def test_reap_dies_during_wait():
    state = {"n": 0}

    def alive(p):
        state["n"] += 1
        return state["n"] <= 2  # 第一次 True，之后死亡

    killer_calls = []
    out = sp.reap_leftover_pids([4242], wait_s=5, alive=alive,
                                killer=lambda p: killer_calls.append(p))
    assert out == [] and killer_calls == []  # 等待窗内自行退出，无需强杀


def test_reap_kills_confirmed_ours():
    killer_calls = []
    state = {"killed": False}

    def alive(p):
        return not state["killed"]  # 强杀后进程消失

    def killer(p):
        killer_calls.append(p)
        state["killed"] = True

    out = sp.reap_leftover_pids([4242], wait_s=0, alive=alive,
                                matcher=lambda p: True, killer=killer)
    assert out == [] and killer_calls == [4242]


def test_reap_reports_survivor():
    out = sp.reap_leftover_pids(
        [4242], wait_s=0, alive=lambda p: True,
        matcher=lambda p: True,
        killer=lambda p: None)  # 杀了也没死
    assert out == [4242]


def test_reap_skips_reused_pid():
    """PID 已被复用成无关进程：绝不能杀（宁可漏杀不可误杀）。"""
    killer_calls = []
    out = sp.reap_leftover_pids(
        [4242], wait_s=0, alive=lambda p: True,
        matcher=lambda p: False,
        killer=lambda p: killer_calls.append(p))
    assert out == [] and killer_calls == []


def test_reap_skips_unknown_identity():
    killer_calls = []
    out = sp.reap_leftover_pids(
        [4242], wait_s=0, alive=lambda p: True,
        matcher=lambda p: None,
        killer=lambda p: killer_calls.append(p))
    assert out == [] and killer_calls == []
