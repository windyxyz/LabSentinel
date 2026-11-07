"""T-336：迁移期一键切换/回退 + 事件率异常检测。

运行: pytest tests/test_t336_analyzer_switch.py -q
覆盖：store.set_camera_analyzer 校验/持久化、events_count 时间窗、
PUT /api/cameras/{id}/analyzer（校验/404/幂等/重启路由）、
_anomaly_check ±50% 告警与基线近零不误报。
"""
import datetime as dt

import pytest
from fastapi.testclient import TestClient

from app import api as api_mod
from app.main import app
from app.pipeline import CameraManager
from app.store import Store


# ---- store 层 ----

def test_set_camera_analyzer_store(tmp_path):
    st = Store(tmp_path / "s.db")
    cid = st.add_camera("A", "D:/a.mp4", "file")
    assert st.camera(cid)["analyzer"] == "python"  # 默认
    assert st.set_camera_analyzer(cid, "rust") is True
    assert st.camera(cid)["analyzer"] == "rust"
    with pytest.raises(ValueError):
        st.set_camera_analyzer(cid, "grpc")  # 非法值
    assert st.set_camera_analyzer(99999, "rust") is False  # 不存在


def test_events_count_windows(tmp_path):
    st = Store(tmp_path / "s.db")
    cid = st.add_camera("A", "D:/a.mp4", "file")
    now = dt.datetime.now()
    old = (now - dt.timedelta(days=3)).isoformat(timespec="seconds")
    recent = (now - dt.timedelta(hours=1)).isoformat(timespec="seconds")
    for ts in (old, old, recent):
        st.add_event(cid, "enter", 2, 1, "x", ts=ts)
    cutoff = (now - dt.timedelta(days=1)).isoformat(timespec="seconds")
    assert st.events_count(cid) == 3
    assert st.events_count(cid, start_ts=cutoff) == 1  # 最近 24h
    assert st.events_count(cid, start_ts=cutoff,
                           end_ts=(now - dt.timedelta(hours=2)
                                   ).isoformat(timespec="seconds")) == 0
    assert st.events_count(999) == 0


# ---- API ----

@pytest.fixture()
def client():
    with TestClient(app) as tc:
        yield tc


def test_analyzer_api_validation(client):
    assert client.put("/api/cameras/1/analyzer",
                      json={"analyzer": "grpc"}).status_code == 422
    assert client.put("/api/cameras/99999/analyzer",
                      json={"analyzer": "rust"}).status_code == 404


def test_analyzer_api_set_and_idempotent(client):
    cam = client.post("/api/cameras",
                      json={"name": "t336", "source": "D:/x.mp4",
                            "type": "file", "autostart": False}).json()
    cid = cam["id"]
    # 停止态：直接落库，不重启
    r = client.put(f"/api/cameras/{cid}/analyzer", json={"analyzer": "rust"})
    assert r.status_code == 200 and r.json() == {"id": cid, "analyzer": "rust",
                                                 "restarted": False}
    assert client.get("/api/cameras").json()[0]["analyzer"] == "rust"
    # 幂等：同值再设不重启
    r = client.put(f"/api/cameras/{cid}/analyzer", json={"analyzer": "rust"})
    assert r.json()["restarted"] is False


# ---- 异常检测 ----

def _seed_events(store, cid, switch_at: dt.datetime, baseline_n: int, after_n: int):
    """切换前 7 天均匀 baseline_n 条/天；切换后按 after_n 条/小时铺。"""
    for d in range(7):
        day = switch_at - dt.timedelta(days=7 - d)
        for i in range(baseline_n):
            ts = (day + dt.timedelta(hours=i)).isoformat(timespec="seconds")
            store.add_event(cid, "enter", 2, 1, "x", ts=ts)
    for h in range(int(after_n)):
        ts = (switch_at + dt.timedelta(hours=h)).isoformat(timespec="seconds")
        store.add_event(cid, "enter", 2, 1, "x", ts=ts)


def test_anomaly_warning_on_deviation(tmp_path, capsys):
    st = Store(tmp_path / "s.db")
    cid = st.add_camera("A", "rtsp://x", "rtsp")
    mgr = CameraManager(st)
    switch_at = dt.datetime.now() - dt.timedelta(hours=2)
    _seed_events(st, cid, switch_at, baseline_n=10, after_n=0)  # 日均 10 vs 0 件/24h
    mgr._note_analyzer(cid, {"analyzer": "python"})
    mgr._note_analyzer(cid, {"analyzer": "rust"})  # 触发切换记录
    mgr._switch_at[cid] = switch_at.timestamp()
    mgr._anomaly_check()
    out = capsys.readouterr().out
    assert "事件率异常" in out and "一键回退" in out
    assert cid in mgr._switch_warned  # 同一开关只告警一次
    mgr._anomaly_check()
    assert capsys.readouterr().out.count("事件率异常") == 0


def test_anomaly_no_false_positive_normal_rate(tmp_path, capsys):
    st = Store(tmp_path / "s.db")
    cid = st.add_camera("A", "rtsp://x", "rtsp")
    mgr = CameraManager(st)
    switch_at = dt.datetime.now() - dt.timedelta(hours=2)
    _seed_events(st, cid, switch_at, baseline_n=10, after_n=1)  # ~12 件/24h vs 10：±50% 内
    mgr._note_analyzer(cid, {"analyzer": "python"})
    mgr._note_analyzer(cid, {"analyzer": "rust"})
    mgr._switch_at[cid] = switch_at.timestamp()
    mgr._anomaly_check()
    assert "事件率异常" not in capsys.readouterr().out
    assert cid not in mgr._switch_warned


def test_anomaly_baseline_near_zero_skipped(tmp_path, capsys):
    """基线近零（<0.5 件/天）不判：±50% 判据对低频路无意义。"""
    st = Store(tmp_path / "s.db")
    cid = st.add_camera("A", "rtsp://x", "rtsp")
    mgr = CameraManager(st)
    switch_at = dt.datetime.now() - dt.timedelta(hours=2)
    _seed_events(st, cid, switch_at, baseline_n=0, after_n=3)
    mgr._note_analyzer(cid, {"analyzer": "python"})
    mgr._note_analyzer(cid, {"analyzer": "rust"})
    mgr._switch_at[cid] = switch_at.timestamp()
    mgr._anomaly_check()
    assert "事件率异常" not in capsys.readouterr().out


def test_anomaly_observation_window_closes(tmp_path):
    """切换超 24h：观测窗关闭，条目回收。"""
    st = Store(tmp_path / "s.db")
    cid = st.add_camera("A", "rtsp://x", "rtsp")
    mgr = CameraManager(st)
    mgr._note_analyzer(cid, {"analyzer": "python"})
    mgr._note_analyzer(cid, {"analyzer": "rust"})
    mgr._switch_at[cid] = dt.datetime.now().timestamp() - 25 * 3600
    mgr._anomaly_check()
    assert cid not in mgr._switch_at
