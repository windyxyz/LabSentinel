"""单元测试：任务103 单事件证据包（路径安全/缺失跳过/404/超大 400/鉴权）。
运行: pytest tests/test_event_evidence.py -q"""
import io
import zipfile

import pytest
from fastapi.testclient import TestClient

from app import api as api_mod
from app.export import build_event_evidence
from app.main import app
from app.store import Store


@pytest.fixture()
def st(tmp_path):
    (tmp_path / "snapshots").mkdir()
    (tmp_path / "videos").mkdir()
    s = Store(tmp_path / "s.db")
    cid = s.add_camera("化一", "D:/x.mp4", "file")
    return s, cid, tmp_path


def _event(st, cid, snapshot=None, video=None):
    eid = st.add_event(cid, "enter", 2, 0, "测试事件")
    with st._conn() as c:
        c.execute("UPDATE events SET snapshot=?, video=? WHERE id=?", (snapshot, video, eid))
    return eid


def test_full_pack(st):
    s, cid, tmp = st
    snap = tmp / "snapshots" / "1.jpg"
    vid = tmp / "videos" / "1.mp4"
    snap.write_bytes(b"\xff\xd8jpegdata")
    vid.write_bytes(b"\x00\x00\x00\x18ftypmp4v")
    eid = _event(s, cid, str(snap), str(vid))
    data, meta = build_event_evidence(s, tmp, eid)
    assert meta["files"] == 2 and meta["skipped"] == []
    z = zipfile.ZipFile(io.BytesIO(data))
    names = z.namelist()
    assert names == ["event.json", "snapshot.jpg", "video.mp4", "README.txt"]
    import json
    ev = json.loads(z.read("event.json"))
    assert ev["id"] == eid and ev["camera_name"] == "化一" and "exported_at" in ev
    assert ev["type"] == "enter" and ev["level"] == 2
    assert z.read("snapshot.jpg") == b"\xff\xd8jpegdata"
    assert "README.txt" in names


def test_missing_media_and_oob_paths(st):
    """无媒体 → 仍 200 出包（JSON+README 带说明）；越界/不存在路径跳过。"""
    s, cid, tmp = st
    eid = _event(s, cid)  # 什么都没存
    data, meta = build_event_evidence(s, tmp, eid)
    assert meta["files"] == 0 and len(meta["skipped"]) == 2
    z = zipfile.ZipFile(io.BytesIO(data))
    assert z.namelist() == ["event.json", "README.txt"]
    readme = z.read("README.txt").decode("utf-8")
    assert "未打包条目说明" in readme and "snapshot" in readme
    # 越界路径 + 不存在文件
    eid2 = _event(s, cid, snapshot="D:/outside/evil.jpg", video=str(tmp / "gone.mp4"))
    data2, meta2 = build_event_evidence(s, tmp, eid2)
    assert meta2["files"] == 0
    assert any("数据目录外" in x for x in meta2["skipped"])
    assert any("不存在" in x for x in meta2["skipped"])
    # .. 穿越形态同样被拒（包含判断）
    eid3 = _event(s, cid, snapshot=str(tmp / "snapshots" / ".." / ".." / "evil.jpg"))
    _, meta3 = build_event_evidence(s, tmp, eid3)
    assert meta3["files"] == 0


def test_not_found_and_oversize(st):
    s, cid, tmp = st
    assert build_event_evidence(s, tmp, 99999) is None  # 事件不存在 → None（404）
    # 超大：一个 >200MB 的文件（稀疏写太慢，直接 mock getsize）
    snap = tmp / "snapshots" / "big.jpg"
    snap.write_bytes(b"x")
    eid = _event(s, cid, snapshot=str(snap))
    import app.export as ex
    orig = ex.os.path.getsize
    ex.os.path.getsize = lambda p: 300 * 1024 * 1024 if str(p) == str(snap) else orig(p)
    try:
        with pytest.raises(ValueError) as ei:
            build_event_evidence(s, tmp, eid)
        assert "批量证据包" in str(ei.value)
    finally:
        ex.os.path.getsize = orig


# ---- API ----

@pytest.fixture()
def client(tmp_path):
    with TestClient(app) as tc:
        yield tc, api_mod.store


def test_api_pack_and_404(client):
    tc, st = client
    cid = st.add_camera("api化一", "D:/x.mp4", "file")
    snap = api_mod.settings.data_dir / "snapshots" / "api-1.jpg"
    snap.parent.mkdir(parents=True, exist_ok=True)
    snap.write_bytes(b"jpegdata")
    eid = _event(st, cid, snapshot=str(snap))
    r = tc.get(f"/api/events/{eid}/evidence.zip")
    assert r.status_code == 200
    assert r.headers["content-type"] == "application/zip"
    assert f"event-{eid}-evidence.zip" in r.headers["content-disposition"]
    z = zipfile.ZipFile(io.BytesIO(r.content))
    assert "event.json" in z.namelist() and "snapshot.jpg" in z.namelist()
    assert tc.get("/api/events/99999/evidence.zip").status_code == 404


def test_api_auth(client, monkeypatch):
    tc, _st = client
    monkeypatch.setattr(api_mod.settings, "admin_token", "sec")
    monkeypatch.setattr(api_mod.settings, "auth_read_protected", True)
    assert tc.get("/api/events/1/evidence.zip").status_code == 401
    assert tc.get("/api/events/1/evidence.zip",
                  headers={"X-Admin-Token": "sec"}).status_code == 404  # 有令牌过鉴权，事件不存在
    monkeypatch.setattr(api_mod.settings, "admin_token", "")
    monkeypatch.setattr(api_mod.settings, "auth_read_protected", False)
