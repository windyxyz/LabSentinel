"""T-331（L3 灰度）回归：rust 摄像头路由 + Events 流入库桥。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app.analyzer import AnalyzerBackend  # noqa: E402
from app.pipeline import CameraManager  # noqa: E402
from app.sidecar_events import SidecarEventBridge  # noqa: E402


class FakeAnalyzer(AnalyzerBackend):
    """记录调用的假 sidecar（不拉真实进程）。"""

    def __init__(self):
        self.started = []
        self.stopped = []
        self._stub = None
        self._token = "t"

    def start(self):
        return None

    def start_camera(self, camera, zones, rules):
        self.started.append((camera, zones, rules))

    def stop_camera(self, camera_id):
        self.stopped.append(camera_id)

    def status(self):
        return []


class FakeBridge:
    def __init__(self):
        self.starts = 0
        self.stops = 0

    def start(self):
        self.starts += 1
        return True

    def stop(self):
        self.stops += 1


class FakeStore:
    def __init__(self):
        self.cameras_list = []
        self.zon = []
        self.rul = []
        self.status = {}

    def cameras(self):
        return self.cameras_list

    def config_version(self):
        return 1

    def zones(self, camera_id=None):
        return self.zon

    def rules(self, camera_id=None):
        return self.rul

    def set_camera_status(self, cid, status):
        self.status[cid] = status


def test_rust_camera_routes_to_sidecar(monkeypatch):
    """analyzer=rust 的路不创建 Python worker，配置全量下发给 sidecar。"""
    monkeypatch.setattr("app.config.settings.analyzer_default", "python")
    st = FakeStore()
    mgr = CameraManager(st)
    ana, bridge = FakeAnalyzer(), FakeBridge()
    mgr.bind_sidecar(ana, bridge)

    # 让 configcache 与 RuleEngine 走伪 store
    monkeypatch.setattr("app.configcache.zones", lambda store, cid: st.zon)
    monkeypatch.setattr("app.configcache.rules",
                        lambda store, camera_id=None: st.rul)
    st.zon = [{"id": 3, "camera_id": 9, "name": "A", "polygon": [[0, 0]]}]
    st.rul = [{"id": 1, "camera_id": 9, "behavior": "enter"}]
    cam = {"id": 9, "name": "c", "source": "x.mp4", "type": "file", "analyzer": "rust"}

    mgr.start_camera(cam)
    assert bridge.starts == 1
    assert len(ana.started) == 1
    got_cam, got_zones, got_rules = ana.started[0]
    assert got_cam["id"] == 9 and got_zones == st.zon and got_rules == st.rul
    assert 9 not in mgr.workers, "rust 路不应创建 Python CameraWorker"
    assert 9 in mgr._rust_cams
    assert st.status[9] == "running"

    st_status = mgr.status()
    assert any(c["id"] == 9 and c.get("analyzer") == "rust" for c in st_status["cameras"])

    mgr.stop_camera(9)
    assert ana.stopped == [9]
    assert st.status[9] == "stopped"


def test_python_camera_still_uses_worker(monkeypatch):
    """analyzer=python 保持原路径（灰度绑定不改变默认行为）。"""
    monkeypatch.setattr("app.config.settings.analyzer_default", "python")
    st = FakeStore()
    mgr = CameraManager(st)
    ana, bridge = FakeAnalyzer(), FakeBridge()
    mgr.bind_sidecar(ana, bridge)
    spawned = []
    monkeypatch.setattr(mgr, "_mk_worker", lambda cam, cid: spawned.append(cid), raising=False)
    # patch 掉具体 worker 构造：只验证没有走 rust
    monkeypatch.setattr("app.pipeline.CameraWorker",
                        lambda *a, **kw: type("W", (), {"start": lambda s: None})())
    cam = {"id": 5, "name": "c", "source": "x.mp4", "type": "file", "analyzer": "python"}
    mgr.start_camera(cam)
    assert bridge.starts == 0 and ana.started == []
    assert 5 in mgr.workers


def test_event_bridge_ingest_dedup_and_dispatch():
    """候选 → add_event → dispatcher.dispatch；重放同 evidence_key 只入一次。"""
    import app.sidecar_events as se

    class Cand:
        def __init__(self, key):
            self.camera_id = 2
            self.zone_id = 1
            self.behavior = "enter"
            self.level = 2
            self.track_id = 7
            self.detail = "进入 A 区"
            self.ts_iso = "2026-01-01T00:00:01"
            self.evidence_key = key
            self.snapshot_jpeg = b""

    class S:
        def __init__(self):
            self.added = []

        def add_event(self, *a, **kw):
            self.added.append((a, kw))
            return len(self.added)

        def event_by_id(self, eid):
            return {"id": eid, "snapshot": "/abs/a.jpg", "video": None}

    class D:
        def __init__(self):
            self.rows = []

        def dispatch(self, row):
            self.rows.append(row)

    s, d = S(), D()
    bridge = SidecarEventBridge(s, d, FakeAnalyzer())
    cand = Cand("cam2:frame10")
    bridge._ingest(cand)
    bridge._ingest(cand)  # 重放
    assert len(s.added) == 1, "重连重放必须去重"
    assert s.added[0][1]["ts"] == cand.ts_iso
    assert len(d.rows) == 1
    assert "snapshot" not in d.rows[0] and d.rows[0]["has_snapshot"] is False
    assert bridge.status()["ingested"] == 1
