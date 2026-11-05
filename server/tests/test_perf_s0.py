"""T-302/T-303/T-309 回归：线程本地连接、ConfigCache 失效、v11 索引。"""
import os
import sys
import threading

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app import configcache  # noqa: E402
from app.store import Store  # noqa: E402


def test_thread_local_connection_reused(tmp_path):
    s = Store(tmp_path / "s.db")
    ids = []

    def work():
        with s._conn() as c:
            ids.append(id(c))
        with s._conn() as c:
            ids.append(id(c))

    t = threading.Thread(target=work)
    t.start()
    t.join()
    assert ids[0] == ids[1], "同线程内应复用同一连接（T-302）"


def test_config_cache_invalidates_on_writes(tmp_path):
    s = Store(tmp_path / "s.db")
    cid = s.add_camera("cam", "x.mp4", "file")
    zid = s.add_zone(cid, "z", [[0, 0], [100, 0], [100, 100]])
    assert len(configcache.zones(s, cid)) == 1
    # 新增规则 → 版本 bump → 缓存重建
    s.add_rule({"camera_id": cid, "zone_id": zid, "behavior": "enter"})
    assert len(configcache.rules(s, cid)) == 1
    s.delete_zone(zid)
    assert configcache.zones(s, cid) == []
    assert configcache.camera_name(s, cid) == "cam"


def test_v11_indexes_exist(tmp_path):
    import sqlite3
    s = Store(tmp_path / "s.db")
    assert s.schema_version() >= 11
    with s._conn() as c:
        names = {r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='index'")}
    assert {"idx_events_snapshot", "idx_deliveries_status",
            "idx_deliveries_channel_status"} <= names
