"""单元测试：任务14 数据保留（过旧行删除/文件删除/紧急清理/状态接口/幂等）。运行: pytest tests/ -q"""
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import api as api_mod  # noqa: E402
from app import retention  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402


@pytest.fixture()
def env(tmp_path, monkeypatch):
    """隔离环境：data 目录、保留期、磁盘下限全部注入 tmp。"""
    s = Store(path=tmp_path / "t.db")
    data_dir = tmp_path / "data"
    snap = data_dir / "snapshots"
    vids = data_dir / "videos"
    for d in (snap, vids):
        d.mkdir(parents=True)
    monkeypatch.setattr(api_mod, "store", s)
    monkeypatch.setattr(settings, "data_dir", data_dir)
    monkeypatch.setattr(settings, "snapshot_dir", snap)
    monkeypatch.setattr(settings, "video_dir", vids)
    monkeypatch.setattr(settings, "annotation_pool_dir", data_dir / "pool")
    monkeypatch.setattr(retention.settings, "data_dir", data_dir)
    monkeypatch.setattr(retention.settings, "snapshot_dir", snap)
    monkeypatch.setattr(retention.settings, "video_dir", vids)
    monkeypatch.setattr(retention.settings, "annotation_pool_dir", data_dir / "pool")
    return s, data_dir, snap, vids, tmp_path


def _aged_event(s, snap, vids, days_old, eid=1, with_files=True):
    """插入一条 days_old 天前的旧事件，可选带截图/视频文件。返回 (id, 文件路径列表)。"""
    from datetime import datetime, timedelta
    ts = (datetime.now() - timedelta(days=days_old)).isoformat(timespec="seconds")
    with s._lock, s._conn() as c:
        cur = c.execute("INSERT INTO events(ts,camera_id,type,level,track_id,detail,snapshot,video,status)"
                        " VALUES(?,?,?,?,?,?,?,?, 'new')",
                        (ts, 1, "enter", 2, 1, "旧事件", None, None))
        eid = cur.lastrowid
    paths = []
    if with_files:
        p1 = snap / f"ev_{eid}.jpg"
        p2 = vids / f"ev_{eid}.mp4"
        p1.write_bytes(b"j" * 1000)
        p2.write_bytes(b"v" * 2000)
        with s._lock, s._conn() as c:
            c.execute("UPDATE events SET snapshot=?, video=? WHERE id=?", (str(p1), str(p2), eid))
        paths = [p1, p2]
    return eid, paths


def test_cleanup_deletes_old_rows_and_files(env, monkeypatch):
    s, data_dir, snap, vids, tmp = env
    monkeypatch.setattr(settings, "retention_events_days", 30)
    old_eid, old_files = _aged_event(s, snap, vids, days_old=40)
    new_eid, _ = _aged_event(s, snap, vids, days_old=1, eid=2)
    stats = retention.run_cleanup(s)
    assert stats["deleted_events"] == 1 and stats["deleted_files"] == 2
    assert all(not p.exists() for p in old_files)  # 文件已删
    assert not s.events(limit=100) or all(r["id"] != old_eid for r in s.events(limit=100))
    assert any(r["id"] == new_eid for r in s.events(limit=100))  # 新事件保留
    assert retention.last_result()["deleted_events"] == 1  # 状态可查


def test_cleanup_idempotent(env, monkeypatch):
    s, *_ = env
    monkeypatch.setattr(settings, "retention_events_days", 30)
    _aged_event(s, *(env[2], env[3]), days_old=40)
    first = retention.run_cleanup(s)
    second = retention.run_cleanup(s)
    assert first["deleted_events"] == 1 and second["deleted_events"] == 0  # 二次无删除


def test_emergency_cleanup_oldest_first(env, monkeypatch):
    """磁盘下限触发紧急清理：从最旧开始删，删到满足空间为止。"""
    s, data_dir, snap, vids, tmp = env
    monkeypatch.setattr(settings, "retention_events_days", 3650)  # 不按期清理
    monkeypatch.setattr(settings, "disk_min_free_gb", 1e9)  # 必触发（磁盘必然 <1e9 GB）
    for i in range(120):
        _aged_event(s, snap, vids, days_old=1, eid=i + 1)
    stats = retention.run_cleanup(s)
    assert stats["deleted_events"] >= 100  # 批量删到满足/上限
    remaining = s.events(limit=1000)
    if remaining:  # 若没删光，保留的必须是最新（id 大）的
        assert min(r["id"] for r in remaining) > 1


def test_maintenance_api(env, monkeypatch):
    s, data_dir, snap, vids, tmp = env
    monkeypatch.setattr(settings, "retention_events_days", 30)
    c = TestClient(app)
    r = c.get("/api/maintenance/status")
    assert r.status_code == 200
    body = r.json()
    assert body["retention"]["events_days"] == 30 and "disk_free_gb" in body
    r = c.post("/api/maintenance/cleanup")
    assert r.status_code == 200
    assert {"deleted_events", "deleted_files", "freed_mb"} <= set(r.json())
