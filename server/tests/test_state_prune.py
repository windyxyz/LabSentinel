"""任务106：ZoneState/RuleEngine 状态回收（长跑内存有界）。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import numpy as np  # noqa: E402
import pytest  # noqa: E402

from app.pipeline import CameraWorker  # noqa: E402
from app.rules import RuleEngine  # noqa: E402
from app.store import Store  # noqa: E402
from app.zones import ZoneState  # noqa: E402


def test_zone_retain_removes_inactive_and_keeps_active():
    st = ZoneState()
    for tid in range(10000):
        st.update(tid, tid % 2 == 0)
    assert st.size == 10000
    removed = st.retain(set(range(10)))
    assert removed == 9990
    assert st.size == 10
    assert st.inside_ids == [0, 2, 4, 6, 8]


def test_zone_retain_cap_evicts_non_active_only():
    st = ZoneState()
    for tid in range(100):
        st.update(tid, True)
    # active 只有 0..9：非 active 全部回收，size 落到 10（cap 10 满足）
    removed = st.retain(set(range(10)), max_size=10)
    assert removed == 90 and st.size == 10
    # 全部 active 且超过 cap：active 不淘汰（保住在场/离场语义）
    st.retain(set(range(10)), max_size=3)
    assert st.size == 10


@pytest.fixture()
def rule_engine(tmp_path):
    s = Store(tmp_path / "rules.db")
    cid = s.add_camera("cam", "x.mp4", "file")
    rid = s.add_rule({"camera_id": cid, "zone_id": None, "behavior": "enter", "level": 2,
                      "cooldown_s": 60, "time_start": "00:00", "time_end": "23:59",
                      "threshold": 0, "enabled": True})
    return RuleEngine(s), rid


def test_rule_prune_by_age(rule_engine):
    eng, rid = rule_engine
    now = eng._last_prune + 10000
    eng._last_fire = {(rid, 1): now - 100, (rid, 2): now - 10, (rid, 3): now - 5000}
    removed = eng.prune(now=now, max_age_s=3600)
    assert removed == 1
    assert (rid, 1) in eng._last_fire and (rid, 2) in eng._last_fire
    assert (rid, 3) not in eng._last_fire


def test_rule_prune_default_uses_max_cooldown(tmp_path):
    """默认保留期取 max(3600, 全部规则冷却)：长冷却规则不能被提前清掉。"""
    s = Store(tmp_path / "rules2.db")
    cid = s.add_camera("cam", "x.mp4", "file")
    rid = s.add_rule({"camera_id": cid, "zone_id": None, "behavior": "enter", "level": 2,
                      "cooldown_s": 7200, "time_start": "00:00", "time_end": "23:59",
                      "threshold": 0, "enabled": True})
    eng = RuleEngine(s)
    now = eng._last_prune + 10000
    eng._last_fire = {(rid, 1): now - 5000}
    assert eng.prune(now=now) == 0  # 5000 < 7200，仍在冷却保护期
    assert (rid, 1) in eng._last_fire


def test_rule_maybe_prune_periodically(rule_engine, monkeypatch):
    eng, rid = rule_engine
    eng.PRUNE_EVERY_CALLS = 2
    eng._last_fire = {(rid, 9): 0.0}  # 极旧
    eng._maybe_prune(eng._last_prune + 10000)  # 第 1 次不清理
    assert (rid, 9) in eng._last_fire
    eng._maybe_prune(eng._last_prune + 10000)  # 第 2 次触发
    assert (rid, 9) not in eng._last_fire


def test_pipeline_reclaims_empty_frame_state(tmp_path, monkeypatch):
    from app.config import settings
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    s = Store(tmp_path / "pipe.db")
    cid = s.add_camera("cam", "x.mp4", "file")
    zid = s.add_zone(cid, "z", [[100, 50], [1100, 50], [1100, 650], [100, 650]])
    s.add_rule({"camera_id": cid, "zone_id": zid, "behavior": "enter", "level": 2,
                "cooldown_s": 0, "time_start": "00:00", "time_end": "23:59",
                "threshold": 0, "enabled": True})
    s.add_rule({"camera_id": cid, "zone_id": zid, "behavior": "exit", "level": 2,
                "cooldown_s": 0, "time_start": "00:00", "time_end": "23:59",
                "threshold": 0, "enabled": True})
    w = CameraWorker(s.camera(cid), s, RuleEngine(s))
    w.ppe = None
    frame = np.zeros((720, 720, 3), dtype=np.uint8)

    class D:
        conf = 0.9
        xyxy = [300.0, 200.0, 500.0, 600.0]

    class E:
        def detect_persons(self, _frame):
            return [D()]

    class Empty:
        def detect_persons(self, _frame):
            return []

    w._analyze(E(), frame)
    assert sum(st.size for st in w._zone_states.values()) == 1
    w._analyze(Empty(), frame)
    assert sum(st.size for st in w._zone_states.values()) == 0  # 空帧后状态清空
