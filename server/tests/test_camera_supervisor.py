"""单元测试：任务114 摄像头 worker 守护自动重启（注入 clock，不真 sleep/不真摄像头）。
运行: pytest tests/test_camera_supervisor.py -q"""
import time

import pytest

from app import config as cfgmod
from app.config import settings
from app.pipeline import CameraManager
from app.store import Store


class FakeWorker:
    """最小 worker 桩：只带 manager.status()/supervisor 需要的属性。"""

    def __init__(self):
        self.analysis_fps_now = 0.0
        self.last_frame_ts = None
        self.last_error = None
        self.last_error_ts = None
        self.reconnect_count = 0
        self.opened_at = None
        self.frames_total = 0
        self.alive = True

    def is_alive(self):
        return self.alive

    def stop(self):
        self.alive = False


class FakeManager(CameraManager):
    """start_camera 复刻守护簿记（desired/given_up 清除）+ 记录调用 + 安置假 worker。"""

    def __init__(self, store: Store):
        super().__init__(store)
        self.start_calls: list[int] = []

    def start_camera(self, camera: dict):
        cid = camera["id"]
        with self._mgmt_lock:
            if camera.get("type") != "file":
                self._desired_running.add(cid)
                st = self._sup.setdefault(cid, self._sup_new_state())
                if st["state"] == "given_up":  # 手动 start 清除 given_up
                    st.update({"state": "running", "restarts": 0, "next_attempt_ts": 0.0})
        self.start_calls.append(cid)
        self.workers[cid] = FakeWorker()  # 模拟"拉起后 worker 存活"；死亡由测试事后置


class Clock:
    def __init__(self, t: float = 1000.0):
        self.t = t

    def __call__(self) -> float:
        return self.t

    def advance(self, dt: float) -> None:
        self.t += dt


@pytest.fixture()
def env(tmp_path, monkeypatch):
    store = Store(tmp_path / "s.db")
    cid = store.add_camera("cam-a", "rtsp://127.0.0.1:8554/demo", "rtsp")
    mgr = FakeManager(store)
    clock = Clock()
    mgr._sup_clock = clock
    monkeypatch.setattr(settings, "supervisor_max_restarts", 10)
    monkeypatch.setattr(settings, "supervisor_enabled", True)
    return mgr, store, cid, clock


def _tick(mgr, n=1):
    for _ in range(n):
        mgr.supervise_tick()


def test_restart_dead_worker_with_backoff(env):
    mgr, store, cid, clock = env
    mgr.start_camera(store.camera(cid))
    mgr.workers[cid].alive = False  # worker 退出
    _tick(mgr)
    assert mgr.start_calls == [cid, cid]  # 守护拉起一次
    st = mgr._sup[cid]
    assert st["restarts"] == 1 and st["state"] == "backoff"
    assert st["next_attempt_ts"] == pytest.approx(clock() + 1.0)  # 第 1 次失败后等 1s
    assert st["last_restart_ts"] is not None


def test_backoff_waits_1_5_30_60(env):
    mgr, store, cid, clock = env
    mgr.start_camera(store.camera(cid))
    expects = [1.0, 5.0, 30.0, 60.0, 60.0]
    for i, wait in enumerate(expects):
        mgr.workers[cid].alive = False
        _tick(mgr)
        st = mgr._sup[cid]
        assert st["restarts"] == i + 1
        assert st["next_attempt_ts"] == pytest.approx(clock() + wait), f"第{i + 1}次失败后退避"
        clock.advance(wait + 0.1)  # 退避到期（worker 仍是"重启后存活"态）


def test_no_restart_during_backoff_window(env):
    mgr, store, cid, clock = env
    mgr.start_camera(store.camera(cid))
    mgr.workers[cid].alive = False
    _tick(mgr)  # 第一次重启
    mgr.workers[cid].alive = False  # 重启后立刻又死
    calls_after_first = len(mgr.start_calls)
    _tick(mgr)  # 退避窗口内再巡检：不重启
    assert len(mgr.start_calls) == calls_after_first
    assert mgr._sup[cid]["state"] == "backoff"


def test_max_restarts_gives_up(env, monkeypatch):
    monkeypatch.setattr(settings, "supervisor_max_restarts", 3)
    mgr, store, cid, clock = env
    mgr.start_camera(store.camera(cid))
    for _ in range(3):
        mgr.workers[cid].alive = False
        _tick(mgr)
        clock.advance(61)
    assert mgr.start_calls == [cid] * 4  # 首次手动 + 3 次自动
    mgr.workers[cid].alive = False
    _tick(mgr)  # 第 4 次：达上限 → given_up，不再调用 start
    assert len(mgr.start_calls) == 4
    st = mgr._sup[cid]
    assert st["state"] == "given_up"
    assert cid not in mgr._desired_running
    assert store.camera(cid)["status"] == "error"
    _tick(mgr)
    assert len(mgr.start_calls) == 4  # 放弃后不再重启
    info = mgr.supervisor_info(cid)
    assert info["supervisor_state"] == "given_up" and info["restart_count"] == 3


def test_user_stop_disables_autorestart(env):
    mgr, store, cid, clock = env
    mgr.start_camera(store.camera(cid))
    mgr.stop_camera(cid)  # 用户主动停止
    _tick(mgr, n=2)
    assert len(mgr.start_calls) == 1  # 不自动拉起
    assert mgr.supervisor_info(cid)["supervisor_state"] == "stopped"
    assert cid not in mgr._desired_running


def test_manual_start_clears_given_up(env, monkeypatch):
    monkeypatch.setattr(settings, "supervisor_max_restarts", 3)
    mgr, store, cid, clock = env
    mgr.start_camera(store.camera(cid))
    for _ in range(4):
        mgr.workers[cid].alive = False
        _tick(mgr)
        clock.advance(61)
    assert mgr._sup[cid]["state"] == "given_up"
    mgr.start_camera(store.camera(cid))  # 手动启动：清除 given_up、重新纳入守护
    st = mgr._sup[cid]
    assert st["state"] == "running" and st["restarts"] == 0
    assert cid in mgr._desired_running


def test_success_resets_backoff_counter(env):
    mgr, store, cid, clock = env
    mgr.start_camera(store.camera(cid))
    mgr.workers[cid].alive = False
    _tick(mgr)  # 重启一次 → restarts=1, start_ts=clock
    assert mgr._sup[cid]["restarts"] == 1
    mgr.workers[cid].alive = True  # 重启后存活
    clock.advance(6.0)  # 存活超过 5s
    _tick(mgr)
    st = mgr._sup[cid]
    assert st["restarts"] == 0 and st["state"] == "running"


def test_deleted_camera_exits_supervision(env):
    mgr, store, cid, clock = env
    mgr.start_camera(store.camera(cid))
    store.delete_camera(cid)
    mgr.workers[cid].alive = False
    _tick(mgr)
    assert cid not in mgr._desired_running and cid not in mgr._sup


def test_status_includes_supervisor_fields(env):
    mgr, store, cid, clock = env
    mgr.start_camera(store.camera(cid))
    mgr.workers[cid].alive = False
    _tick(mgr)
    entry = mgr.status()["cameras"][0]
    assert entry["restart_count"] == 1
    assert entry["supervisor_state"] == "backoff"
    assert entry["last_restart_ts"] is not None


def test_supervisor_info_never_managed(env):
    mgr, _, _, _ = env
    assert mgr.supervisor_info(999) == {"restart_count": 0, "last_restart_ts": None,
                                        "supervisor_state": None}


def test_start_supervisor_disabled(env, monkeypatch):
    mgr, _, _, _ = env
    monkeypatch.setattr(settings, "supervisor_enabled", False)
    mgr.start_supervisor()
    assert mgr._sup_thread is None  # 禁用：不起线程


def test_start_supervisor_max_zero_disables(env, monkeypatch):
    mgr, _, _, _ = env
    monkeypatch.setattr(settings, "supervisor_max_restarts", 0)
    mgr.start_supervisor()
    assert mgr._sup_thread is None  # 0=禁用守护


def test_file_camera_not_supervised(env):
    """file 源播完/坏路径正常退出，不得被守护无限重启（否则演示视频循环/坏路径刷线程）。"""
    mgr, store, _, _ = env
    fcid = store.add_camera("filecam", "D:/nonexistent.mp4", "file")
    mgr.start_camera(store.camera(fcid))
    assert fcid not in mgr._desired_running
    mgr.workers[fcid].alive = False
    mgr.supervise_tick()
    assert mgr.start_calls == [fcid]  # 只启动了第一次，无自动重启


def test_supervisor_thread_lifecycle(env, monkeypatch):
    """守护线程启动后可停（不残留）；tick 循环真的在跑。"""
    mgr, _, _, _ = env
    monkeypatch.setattr(cfgmod.settings, "supervisor_interval_s", 0.05)
    mgr.start_supervisor()
    assert mgr._sup_thread is not None and mgr._sup_thread.is_alive()
    time.sleep(0.15)  # 至少跑过一轮 tick（无 desired 时为空操作）
    mgr.stop_supervisor()
    assert mgr._sup_thread is None
    assert mgr._sup_stop.is_set()
