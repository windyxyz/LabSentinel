"""单元测试：任务68 保留预演（只读）+ maintenance status 水位字段。
运行: pytest tests/test_retention_preview.py -q"""
import os
import sqlite3
import sys
import time
from datetime import datetime, timedelta
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import retention  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402


def make_db(tmp_path: Path, expired: int = 3, fresh: int = 2, with_media: bool = True) -> Store:
    (tmp_path / "snapshots").mkdir(exist_ok=True)
    (tmp_path / "videos").mkdir(exist_ok=True)
    (tmp_path / "pool").mkdir(exist_ok=True)
    s = Store(tmp_path / "s.db")
    old = (datetime.now() - timedelta(days=settings.retention_events_days + 5)).isoformat(timespec="seconds")
    new = datetime.now().isoformat(timespec="seconds")
    con = sqlite3.connect(tmp_path / "s.db")
    for i in range(expired):
        snap = vid = None
        if with_media:
            snap = str(tmp_path / "snapshots" / f"old{i}.jpg")
            vid = str(tmp_path / "videos" / f"old{i}.mp4")
            Path(snap).write_bytes(b"S" * 100)
            Path(vid).write_bytes(b"V" * 400)
        con.execute("insert into events(ts, camera_id, type, level, detail, snapshot, video) "
                    "values(?,?,?,?,?,?,?)", (old, 1, "enter", 2, f"old{i}", snap, vid))
    for i in range(fresh):
        snap = str(tmp_path / "snapshots" / f"new{i}.jpg")
        Path(snap).write_bytes(b"N" * 50) if with_media else None
        con.execute("insert into events(ts, camera_id, type, level, detail, snapshot) "
                    "values(?,?,?,?,?,?)", (new, 1, "enter", 2, f"new{i}", snap))
    con.commit()
    con.close()
    # 误报池：1 过期 + 1 新鲜
    old_pool = tmp_path / "pool" / "old.png"
    old_pool.write_bytes(b"P" * 200)
    stamp = time.time() - (settings.retention_pool_days + 2) * 86400
    os.utime(old_pool, (stamp, stamp))
    (tmp_path / "pool" / "new.png").write_bytes(b"Q" * 10)
    # settings 指向临时目录（monkeypatch 已做 data_dir；pool/snapshot/video 需分别指）
    return s


@pytest.fixture()
def env(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    monkeypatch.setattr(settings, "snapshot_dir", tmp_path / "snapshots")
    monkeypatch.setattr(settings, "video_dir", tmp_path / "videos")
    monkeypatch.setattr(settings, "annotation_pool_dir", tmp_path / "pool")
    monkeypatch.setattr(settings, "db_path", tmp_path / "s.db")
    s = make_db(tmp_path)
    yield tmp_path, s


def test_preview_readonly_and_counts(env):
    tmp_path, s = env
    before_files = sorted(str(p) for p in tmp_path.rglob("*") if p.is_file())
    p = retention.preview_cleanup(s)
    assert p["events"]["count"] == 3
    assert p["events"]["bytes"] == 3 * 500  # 100+400 每条
    assert p["pool"]["files"] == 1 and p["pool"]["bytes"] == 200
    assert p["would_emergency_clean"] is False
    after_files = sorted(str(x) for x in tmp_path.rglob("*") if x.is_file())
    assert before_files == after_files  # 绝不删除
    con = sqlite3.connect(tmp_path / "s.db")
    assert con.execute("select count(*) from events").fetchone()[0] == 5
    con.close()


def test_run_cleanup_matches_preview_count(env):
    tmp_path, s = env
    p = retention.preview_cleanup(s)
    stats = retention.run_cleanup(s)
    assert stats["deleted_events"] == p["events"]["count"]  # 预演 count 与实删一致
    con = sqlite3.connect(tmp_path / "s.db")
    left = con.execute("select count(*) from events").fetchone()[0]
    con.close()
    assert left == 2
    assert not (tmp_path / "pool" / "old.png").exists()
    assert (tmp_path / "pool" / "new.png").exists()


def test_preview_missing_files_and_traversal(env):
    tmp_path, s = env
    con = sqlite3.connect(tmp_path / "s.db")
    con.execute("insert into events(ts, camera_id, type, level, detail, snapshot, video) "
                "values(?,?,?,?,?,?,?)",
                ((datetime.now() - timedelta(days=400)).isoformat(timespec="seconds"),
                 1, "enter", 2, "gone", str(tmp_path / "nope.jpg"), "../../etc/passwd"))
    con.commit()
    con.close()
    p = retention.preview_cleanup(s)  # 缺失文件/穿越路径不崩，穿越路径不计字节
    assert p["events"]["count"] == 4


def test_status_endpoint_fields(env, monkeypatch):
    tmp_path, s = env
    monkeypatch.setattr(retention, "start_scheduler", lambda store: None)  # 防 lifespan 后台清理删掉过期数据
    with TestClient(app) as c:
        d = c.get("/api/maintenance/status").json()
        for k in ("disk_free_gb", "disk_total_gb", "disk_free_pct", "data_bytes",
                  "data_bytes_capped", "low_disk", "forecast_days", "retention"):
            assert k in d
        assert d["data_bytes"] > 0
        pv = c.get("/api/maintenance/preview").json()
        for k in ("cutoff", "events", "pool", "disk_free_gb",
                  "estimated_free_gb", "would_emergency_clean"):
            assert k in pv
        assert pv["events"]["count"] == 3
