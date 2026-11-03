"""任务107：孤儿媒体回收（引用文件/宽限期/路径安全/预演）。"""
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402

from app import retention  # noqa: E402
from app.config import settings  # noqa: E402
from app.store import Store  # noqa: E402


def old(path, seconds=7200):
    ts = time.time() - seconds
    os.utime(path, (ts, ts))
    return path


@pytest.fixture()
def media(tmp_path):
    s = Store(tmp_path / "media.db")
    cid = s.add_camera("cam", "x.mp4", "file")
    snap_dir, vid_dir = settings.snapshot_dir, settings.video_dir

    ref_snap = snap_dir / "ref_snap.jpg"
    ref_snap.write_bytes(b"ref-snap")
    ref_vid = vid_dir / "ref_vid.mp4"
    ref_vid.write_bytes(b"ref-video")
    old(ref_snap)
    old(ref_vid)
    eid = s.add_event(cid, "enter", 2, 0, "引用事件", str(ref_snap))
    s.set_event_video(eid, str(ref_vid))

    folder_ref = snap_dir / "folder_ref.jpg"
    folder_ref.write_bytes(b"folder-ref")
    old(folder_ref)
    s.add_folder_image({
        "camera_id": cid, "path": str(tmp_path / "src.jpg"), "name": "src.jpg",
        "mtime_ns": 1, "size": 1, "ts": "2026-11-03T10:00:00",
        "processed_ts": "2026-11-03T10:00:01", "key": True, "violation": True,
        "snapshot": str(folder_ref)})

    old1 = snap_dir / "old1.jpg"
    old1.write_bytes(b"x" * 1_000_000)
    old(old1)
    old2 = vid_dir / "old2.mp4"
    old2.write_bytes(b"x" * 1_000_000)
    old(old2)
    fresh = snap_dir / "fresh.jpg"  # 10 分钟前：宽限期内
    fresh.write_bytes(b"fresh")
    old(fresh, 600)
    return {"store": s, "cid": cid, "snap": snap_dir, "vid": vid_dir,
            "ref": [ref_snap, ref_vid, folder_ref], "orphans": [old1, old2],
            "fresh": fresh}


def test_preview_counts_but_never_deletes(media):
    preview = retention.preview_cleanup(media["store"])
    assert preview["orphans"]["files"] == 2
    assert preview["orphans"]["bytes"] > 0
    assert all(p.is_file() for p in media["orphans"] + media["ref"] + [media["fresh"]])


def test_cleanup_deletes_only_old_unreferenced(media):
    stats = retention.run_cleanup(media["store"])
    assert stats["deleted_orphans"] == 2
    assert stats["freed_orphan_mb"] > 0
    assert not any(p.exists() for p in media["orphans"])
    assert media["fresh"].is_file(), "宽限期内的新文件不得删除"
    assert all(p.is_file() for p in media["ref"]), "事件/图片记录引用的文件不得删除"
    preview = retention.preview_cleanup(media["store"])
    assert preview["orphans"]["files"] == 0


def test_outside_data_dir_not_deleted(tmp_path, monkeypatch):
    outside = tmp_path / "outside"
    outside.mkdir()
    f = outside / "outside.jpg"
    f.write_bytes(b"outside")
    old(f)
    monkeypatch.setattr(settings, "snapshot_dir", outside)
    s = Store(tmp_path / "outside.db")
    preview = retention.preview_cleanup(s)
    assert preview["orphans"]["files"] == 0
    retention.run_cleanup(s)
    assert f.is_file()


def test_empty_or_missing_dirs_no_crash(tmp_path, monkeypatch):
    empty = tmp_path / "empty"
    empty.mkdir()
    monkeypatch.setattr(settings, "snapshot_dir", empty)
    monkeypatch.setattr(settings, "video_dir", tmp_path / "missing")
    s = Store(tmp_path / "empty.db")
    preview = retention.preview_cleanup(s)
    assert preview["orphans"]["files"] == 0
    stats = retention.run_cleanup(s)
    assert stats["deleted_orphans"] == 0
