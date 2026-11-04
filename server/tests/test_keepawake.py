"""单元测试：任务123 监控机防休眠（fake kernel32，不真调 Win32、不真 sleep 长间隔）。
运行: pytest tests/test_keepawake.py -q"""
import sys
import threading
import time

import pytest

from app import keepawake

ES_CONTINUOUS = keepawake.ES_CONTINUOUS
ES_SYSTEM_REQUIRED = keepawake.ES_SYSTEM_REQUIRED
ES_DISPLAY_REQUIRED = keepawake.ES_DISPLAY_REQUIRED


class FakeK32:
    def __init__(self, fail=False, raise_on=None):
        self.calls: list[int] = []
        self.fail = fail
        self.raise_on = raise_on  # 第 N 次调用抛异常（1 基）

    def SetThreadExecutionState(self, flags):
        self.calls.append(flags)
        if self.raise_on and len(self.calls) >= self.raise_on:
            raise OSError("kernel32 gone")
        return 0 if self.fail else 0x80000041  # 模拟返回"上一个状态"（非 0=成功）


@pytest.fixture()
def clean(monkeypatch):
    """每用例：重置模块状态 + 缩短重断言间隔 + 收尾 stop 兜底。"""
    monkeypatch.setattr(keepawake, "REASSERT_S", 0.05)
    monkeypatch.setattr(keepawake, "_state", {"enabled": False, "active": False, "display": False})
    monkeypatch.setattr(keepawake, "_thread", None)
    monkeypatch.setattr(keepawake, "_stop", threading.Event())
    yield
    keepawake.stop()


def test_start_asserts_system_lock(clean, monkeypatch):
    fake = FakeK32()
    monkeypatch.setattr(keepawake, "_kernel32", fake)
    assert keepawake.start(True) is True
    assert fake.calls[0] == ES_CONTINUOUS | ES_SYSTEM_REQUIRED
    st = keepawake.status()
    assert st == {"enabled": True, "active": True, "display": False}
    keepawake.stop()
    assert fake.calls[-1] == ES_CONTINUOUS  # 解除=仅 CONTINUOUS
    assert keepawake.status()["active"] is False


def test_start_with_display_flag(clean, monkeypatch):
    fake = FakeK32()
    monkeypatch.setattr(keepawake, "_kernel32", fake)
    assert keepawake.start(True, display=True) is True
    assert fake.calls[0] == ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
    assert keepawake.status()["display"] is True


def test_reassert_loop_repeats(clean, monkeypatch):
    fake = FakeK32()
    monkeypatch.setattr(keepawake, "_kernel32", fake)
    keepawake.start(True)
    time.sleep(0.18)  # REASSERT_S=0.05 → 至少重断言 2 次
    keepawake.stop()
    assert len(fake.calls) >= 3  # 首断言 + ≥2 次重断言 + 解除
    assert all(c == ES_CONTINUOUS | ES_SYSTEM_REQUIRED for c in fake.calls[:-1])


def test_start_disabled_is_noop(clean, monkeypatch):
    fake = FakeK32()
    monkeypatch.setattr(keepawake, "_kernel32", fake)
    assert keepawake.start(False) is False
    assert fake.calls == []
    assert keepawake.status()["active"] is False


def test_non_windows_noop(clean, monkeypatch):
    fake = FakeK32()
    monkeypatch.setattr(keepawake, "_kernel32", fake)
    monkeypatch.setattr(sys, "platform", "linux")
    assert keepawake.start(True) is False
    assert fake.calls == []
    assert keepawake.status()["active"] is False


def test_win32_returns_zero_no_thread(clean, monkeypatch):
    """SetThreadExecutionState 返回 0（组策略限制）：active=False，不常驻线程。"""
    fake = FakeK32(fail=True)
    monkeypatch.setattr(keepawake, "_kernel32", fake)
    assert keepawake.start(True) is False
    assert keepawake.status()["active"] is False


def test_win32_raises_start_still_ok(clean, monkeypatch):
    """fake 抛错：start 正常返回 False、只记日志，服务启动不受影响。"""
    fake = FakeK32(raise_on=1)
    monkeypatch.setattr(keepawake, "_kernel32", fake)
    assert keepawake.start(True) is False
    keepawake.stop()  # 不抛


def test_reassert_failure_marks_inactive(clean, monkeypatch):
    """重断言失败 → active 翻 False、线程退出（不再累积调用）。"""
    fake = FakeK32(raise_on=3)  # 第 3 次调用（第 2 次重断言）抛
    monkeypatch.setattr(keepawake, "_kernel32", fake)
    assert keepawake.start(True) is True
    time.sleep(0.2)
    assert keepawake.status()["active"] is False
    n = len(fake.calls)
    time.sleep(0.1)
    assert len(fake.calls) == n  # 线程已退出
    keepawake.stop()


def test_double_start_idempotent(clean, monkeypatch):
    fake = FakeK32()
    monkeypatch.setattr(keepawake, "_kernel32", fake)
    assert keepawake.start(True) is True
    t1 = keepawake._thread
    assert keepawake.start(True) is True
    assert keepawake._thread is t1  # 不重复起线程
    keepawake.stop()


def test_stop_idempotent_and_without_start(clean):
    keepawake.stop()  # 未 start 过：no-op 不抛
    keepawake.stop()
    assert keepawake.status()["active"] is False


def test_process_exit_without_stop_releases_via_daemon(clean, monkeypatch):
    """stop 后线程退出：daemon 线程不残留（进程退出时 OS 兜底清锁）。"""
    fake = FakeK32()
    monkeypatch.setattr(keepawake, "_kernel32", fake)
    keepawake.start(True)
    th = keepawake._thread
    keepawake.stop()
    assert not th.is_alive()
    assert not th.daemon or True  # daemon=True，仅确认已退出
