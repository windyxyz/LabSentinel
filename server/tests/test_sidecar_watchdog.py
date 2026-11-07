"""T-327 回归：sidecar 崩溃自恢复 watchdog + 配置版本推送 + supervisor 纳入 rust 路。

不拉真实进程：用 FakeProc/FakeStub 模拟进程死亡与 gRPC 重连，验证
ensure_running 的重建/快照重放/版本推送语义与 /api/system 观测字段。
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app.analyzer_sidecar import SidecarAnalyzer  # noqa: E402
from app.pipeline import CameraManager  # noqa: E402


class FakeProc:
    """可控存活状态的假进程（poll() 返回非 None = 已退出）。"""

    def __init__(self):
        self.alive = True
        self.stdout = None
        self.killed = False

    def poll(self):
        return None if self.alive else 1

    def terminate(self):
        self.alive = False
        self.killed = True

    def wait(self, timeout=None):
        self.alive = False
        return 0

    def kill(self):
        self.alive = False


class FakeStub:
    def __init__(self):
        self.configs = []
        self.starts = []
        self.stops = []

    def ApplyConfig(self, cfg, metadata=None):
        self.configs.append(cfg)
        return type("Ack", (), {"ok": True})()

    def StartCamera(self, ref, metadata=None):
        self.starts.append(ref.camera_id)

    def StopCamera(self, ref, metadata=None):
        self.stops.append(ref.camera_id)


def _ready_analyzer():
    """构造已就绪的 SidecarAnalyzer（绕过 Popen/READY 握手，只测 watchdog 逻辑）。"""
    a = SidecarAnalyzer(exe=None)
    a._started = True
    a._proc = FakeProc()
    a._stub = FakeStub()
    a._token = "t"
    a._port = 59000
    a._cams = {1: {"id": 1, "name": "c1", "source": "x.mp4", "type": "file"}}
    a._zones = {3: {"id": 3, "camera_id": 1, "name": "A", "polygon": [[0, 0]]}}
    a._rules = {5: {"id": 5, "camera_id": 1, "behavior": "enter"}}
    return a


def test_watchdog_restarts_dead_process_and_replays(monkeypatch):
    """进程死亡 → ensure_running 重建 + ApplyConfig + 每路 StartCamera + 恢复回调。"""
    a = _ready_analyzer()
    old_stub = a._stub
    recovered = []
    a.set_recovery_hook(lambda: recovered.append(1))
    # 替换 start()：不 Popen，只生成新 stub/新 proc/新 token
    calls = []

    def fake_start():
        calls.append("start")
        a._started = True
        a._proc = FakeProc()
        a._stub = FakeStub()
        a._token = "t2"
        a._port = 59001

    monkeypatch.setattr(a, "start", fake_start)
    a._proc.alive = False  # 模拟被 kill

    assert a.ensure_running() is True
    assert calls == ["start"]
    assert a.watchdog_status()["running"] is True
    assert a.watchdog_status()["restarts"] == 1
    assert a.watchdog_status()["last_restart_ts"]
    assert recovered == [1], "恢复后必须唤醒事件桥重建订阅"
    # 全量快照重放：新 stub 收到 1 次 ApplyConfig + 1 次 StartCamera
    assert len(a._stub.configs) == 1
    assert a._stub.starts == [1]
    assert a._stub.configs[0].version == 1  # 重放递增版本
    assert old_stub is not a._stub, "必须换新 stub（新进程/新端口）"
    # 再调一次（进程存活）：不重复重建
    assert a.ensure_running() is True
    assert calls == ["start"] and a.watchdog_status()["restarts"] == 1


def test_ensure_running_noop_when_alive():
    a = _ready_analyzer()
    assert a.ensure_running() is True
    assert a.watchdog_status()["restarts"] == 0
    assert a._cfg_version == 0


def test_config_version_push(monkeypatch):
    """Store.config_version 变化 → 重建快照 → ApplyConfig；无变化零开销。"""
    a = _ready_analyzer()

    class S:
        def __init__(self, ver):
            self.ver = ver
            self.bump_calls = 0

        def config_version(self):
            return self.ver

        def cameras(self):
            return [{"id": 1, "name": "c1", "source": "x.mp4", "type": "file",
                     "analyzer": "rust"}]

    monkeypatch.setattr("app.configcache.zones", lambda store, cid:
                        [{"id": 3, "camera_id": 1, "name": "A", "polygon": [[0, 0]]}])
    monkeypatch.setattr("app.configcache.rules", lambda store, camera_id=None:
                        [{"id": 5, "camera_id": 1, "behavior": "enter"}])
    s = S(1)
    assert a.sync_config(s) is True
    assert len(a._stub.configs) == 1 and a._stub.starts == [1]
    assert a._store_config_version == 1
    # 同版本不推
    assert a.sync_config(s) is False
    assert len(a._stub.configs) == 1
    # 版本变化再推
    s.ver = 2
    assert a.sync_config(s) is True
    assert len(a._stub.configs) == 2
    assert a._stub.configs[1].version == 2  # 本地快照版本逐次递增


def test_supervisor_tick_calls_watchdog(monkeypatch):
    """rust 路纳入守护：supervise_tick 调 sidecar.ensure_running(store)。"""
    monkeypatch.setattr("app.config.settings.analyzer_default", "python")
    st = type("S", (), {"cameras": lambda s: [], "config_version": lambda s: 1})()
    mgr = CameraManager(st)
    hits = []

    class A:
        def ensure_running(self, store=None):
            hits.append(store)

        def watchdog_status(self):
            return {"running": True, "restarts": 0}

        def status(self):
            return []

        def version(self):
            return "unknown"

    mgr.bind_sidecar(A(), None)
    mgr._rust_cams.add(9)
    mgr._desired_running.add(9)
    mgr.supervise_tick()
    assert hits == [st], "supervise_tick 必须对 rust 路跑 watchdog"
    # status() 暴露 watchdog 观测（soak 演练用）
    assert mgr.status()["sidecar"] == {"running": True, "restarts": 0}


def test_watchdog_status_false_when_dead():
    """观测字段必须真实反映进程死亡（不依赖默认 True 的宽松判定）。"""
    a = _ready_analyzer()
    a._proc.alive = False
    assert a.watchdog_status()["running"] is False
