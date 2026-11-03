"""任务108：告警派发退出可打断 + send_test 并发（不访问网络）。"""
import os
import sys
import threading
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402

from app import alerts  # noqa: E402
from app.alerts import AlertDispatcher  # noqa: E402


class SlowChannel:
    def __init__(self, name, delay=1.5):
        self.name = name
        self.delay = delay

    def configured(self):
        return True

    def send(self, level, text):
        time.sleep(self.delay)
        return True


class OffChannel:
    def __init__(self, name):
        self.name = name

    def configured(self):
        return False

    def send(self, level, text):
        raise AssertionError("未配置渠道不应被调用")


class FailChannel:
    name = "fail"

    def send(self, level, text):
        return False


def test_send_test_parallel_and_stable_order():
    d = AlertDispatcher(store=None)
    d.channels = [SlowChannel("a", 1.2), SlowChannel("b", 1.2), OffChannel("c")]
    t0 = time.monotonic()
    results = d.send_test()
    elapsed = time.monotonic() - t0
    assert elapsed < 3.0, f"并发自检不应串行：{elapsed:.2f}s"
    assert [r["channel"] for r in results] == ["a", "b", "c"]
    assert results[0]["ok"] and results[1]["ok"]
    assert results[2]["error"] == "未配置"


def test_send_test_unconfigured_writes_no_delivery():
    class Store:
        def add_delivery(self, *a):
            raise AssertionError("未配置渠道不得建投递记录")

    d = AlertDispatcher(store=Store())
    d.channels = [OffChannel("off")]
    assert d.send_test()[0]["error"] == "未配置"


def test_shutdown_interrupts_retry_backoff(monkeypatch):
    monkeypatch.setattr(alerts, "RETRY_BACKOFF", (5.0, 5.0, 5.0))
    d = AlertDispatcher(store=None)
    th = threading.Thread(target=d._deliver_with_retry,
                          args=(FailChannel(), 1, 1, "x"), daemon=True)
    th.start()
    time.sleep(0.2)  # 让它进入第一次退避 wait(5)
    t0 = time.monotonic()
    d.shutdown(wait=False, timeout=3)
    th.join(timeout=2.0)
    assert not th.is_alive(), "shutdown 后重试线程应被立即打断"
    assert time.monotonic() - t0 < 1.5


def test_shutdown_idempotent_and_reset():
    d = AlertDispatcher(store=None)
    d.shutdown(wait=False)
    d.shutdown(wait=False)  # 重复调用不抛
    assert d._closing.is_set()
    d.reset()
    assert not d._closing.is_set()
    d.shutdown(wait=False)
