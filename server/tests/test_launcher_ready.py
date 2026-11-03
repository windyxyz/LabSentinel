"""单元测试：任务119 便携启动器就绪等待/无人值守静默。
运行: pytest tests/test_launcher_ready.py -q"""
import importlib.util
import os
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "portable_launcher", Path(__file__).resolve().parents[2] / "scripts" / "portable_launcher.py")
launcher = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(launcher)


def test_ready_timeout_default_60(monkeypatch):
    monkeypatch.delenv("LS_READY_TIMEOUT_S", raising=False)
    assert launcher.ready_timeout_s() == 60


def test_ready_timeout_env_override(monkeypatch):
    monkeypatch.setenv("LS_READY_TIMEOUT_S", "90")
    assert launcher.ready_timeout_s() == 90


def test_ready_timeout_invalid_falls_back(monkeypatch):
    monkeypatch.setenv("LS_READY_TIMEOUT_S", "abc")
    assert launcher.ready_timeout_s() == 60


def test_ready_timeout_min_clamp(monkeypatch):
    monkeypatch.setenv("LS_READY_TIMEOUT_S", "3")  # 过小无意义，钳到 10
    assert launcher.ready_timeout_s() == 10


def test_stop_process_dead_pid_no_crash():
    """stop_process 对已死 pid 不应抛异常（taskkill 非零返回码被忽略）。"""
    launcher.stop_process(999999)  # 不存在的高位 pid
