"""单元测试：任务33 事件证据包 ZIP（打包内容/超限 400/路径校验）。运行: pytest tests/ -q"""
import io
import os
import sys
import zipfile

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import api as api_mod  # noqa: E402
from app.config import settings  # noqa: E402
from app.export import EVIDENCE_MAX_EVENTS, build_evidence_zip  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402


@pytest.fixture()
def env(tmp_path, monkeypatch):
    s = Store(path=tmp_path / "t.db")
    data_dir = tmp_path / "data"
    snap, vids = data_dir / "snapshots", data_dir / "videos"
    snap.mkdir(parents=True, exist_ok=True); vids.mkdir(parents=True, exist_ok=True)
    monkeypatch.setattr(api_mod, "store", s)
    monkeypatch.setattr(settings, "data_dir", data_dir)
    return s, data_dir, snap, vids


def _event_with_files(s, snap, vids, eid, with_snapshot=True, with_video=True,
                      snapshot_path=None):
    from datetime import datetime
    ts = datetime.now().isoformat(timespec="seconds")
    with s._lock, s._conn() as c:
        cur = c.execute("INSERT INTO events(ts,camera_id,type,level,track_id,detail,snapshot,video,status)"
                        " VALUES(?,?,?,?,?,?,NULL,NULL,'new')", (ts, 1, "enter", 2, 1, ""))
        eid = cur.lastrowid
    if snapshot_path is None and with_snapshot:
        snapshot_path = snap / f"ev_{eid}.jpg"
        snapshot_path.write_bytes(b"\xff\xd8jpg")
    if with_video:
        v = vids / f"ev_{eid}.mp4"
        v.write_bytes(b"\x00mp4")
    with s._lock, s._conn() as c:
        if snapshot_path is not None:
            c.execute("UPDATE events SET snapshot=? WHERE id=?", (str(snapshot_path), eid))
        c.execute("UPDATE events SET video=? WHERE id=?", (str(vids / f"ev_{eid}.mp4"), eid))
    return eid


def test_zip_contents(env):
    """范围内打包：events.csv(BOM) + snapshots/ + videos/ + README.txt。"""
    s, data_dir, snap, vids = env
    e1 = _event_with_files(s, snap, vids, 1, with_video=False)
    e2 = _event_with_files(s, snap, vids, 2)
    data, st = build_evidence_zip(s, data_dir)
    assert st["events"] == 2 and st["files"] == 3  # 2 截图 + 1 视频
    z = zipfile.ZipFile(io.BytesIO(data))
    names = z.namelist()
    assert "events.csv" in names and "README.txt" in names
    assert f"snapshots/ev_{e1}.jpg" in names and f"videos/ev_{e2}.mp4" in names
    csv_text = z.read("events.csv").decode("utf-8-sig")
    assert csv_text.splitlines()[0] == "时间,摄像头,类型,级别,track,详情,状态,处理人,处理时间"
    assert "LabSentinel 事件证据包" in z.read("README.txt").decode("utf-8")
    assert "合规提示" in z.read("README.txt").decode("utf-8")


def test_zip_over_limit_400(env, monkeypatch):
    """事件数超上限 → build 抛 ValueError（API 层转 400）。"""
    s, data_dir, snap, vids = env
    monkeypatch.setattr("app.export.EVIDENCE_MAX_EVENTS", 2)
    _event_with_files(s, snap, vids, 1)
    _event_with_files(s, snap, vids, 2)
    _event_with_files(s, snap, vids, 3)
    with pytest.raises(ValueError, match="超过打包上限"):
        build_evidence_zip(s, data_dir)
    # API 层转 400
    monkeypatch.setattr(api_mod, "store", s)
    c = TestClient(app)
    assert c.get("/api/export/evidence.zip").status_code == 400


def test_zip_path_traversal_rejected(env):
    """snapshot 指向 data_dir 外（穿越）→ 不打包、不报错。"""
    s, data_dir, snap, vids = env
    outside = data_dir.parent / "evil.jpg"
    outside.write_bytes(b"secret")
    _event_with_files(s, snap, vids, 1, snapshot_path=outside)  # 越界路径
    data, st = build_evidence_zip(s, data_dir)
    z = zipfile.ZipFile(io.BytesIO(data))
    assert not any("evil" in n for n in z.namelist())  # 越界文件不入包
    assert "events.csv" in z.namelist()  # 事件行仍在


def test_zip_api_endpoint(env, monkeypatch):
    s, data_dir, snap, vids = env
    monkeypatch.setattr(api_mod, "store", s)
    _event_with_files(s, snap, vids, 1)
    c = TestClient(app)
    r = c.get("/api/export/evidence.zip")
    assert r.status_code == 200
    assert r.headers["content-type"] == "application/zip"
    assert r.headers["content-disposition"].startswith("attachment; filename=labsentinel_evidence_")
    z = zipfile.ZipFile(io.BytesIO(r.content))
    assert "events.csv" in z.namelist()
