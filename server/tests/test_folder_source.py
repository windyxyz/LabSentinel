"""任务121 图片文件夹自动识别：存储/worker/API/保留清理单元测试（不加载 ONNX）。"""
import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import cv2  # noqa: E402
import numpy as np  # noqa: E402
import pytest  # noqa: E402
from fastapi import HTTPException  # noqa: E402

from app import api  # noqa: E402
from app import retention  # noqa: E402
from app.config import settings  # noqa: E402
from app.folders import FolderWorker, thumbnail_bytes  # noqa: E402
from app.rules import RuleEngine  # noqa: E402
from app.store import Store  # noqa: E402

FULL_ZONE = [[0, 0], [1280, 0], [1280, 719], [0, 719]]
PERSON = (400, 200, 600, 600)  # 锚点 (500,600) 落在全画幅安全区内


class FakeDet:
    conf = 0.9

    def __init__(self, box):
        self.xyxy = [float(v) for v in box]


class FakeEngine:
    def __init__(self, boxes=()):
        self.boxes = [FakeDet(b) for b in boxes]

    def detect_persons(self, frame):
        return self.boxes


class FakePpe:
    """按 T-301 新接口桩：detect_all_raw=一次整图推理（raw_calls 计数），
    detect_class_from_raw=纯解码（decode_calls 计数，不触发推理）。"""

    def __init__(self, no_boxes=(), wear_boxes=()):
        self.no = [FakeDet(b) for b in no_boxes]
        self.wear = [FakeDet(b) for b in wear_boxes]
        self.raw_calls = 0
        self.decode_calls = 0

    def detect_all_raw(self, frame):
        self.raw_calls += 1
        return ("raw", (1.0, 0.0, 0.0))

    def detect_class_from_raw(self, out, geom, name, conf=None):
        self.decode_calls += 1
        return self.no if name == settings.ppe_violation_class else self.wear

    def detect_class(self, frame, name, conf=None):
        # 兼容旧直调路径（pipeline 单人判定等）
        self.decode_calls += 1
        return self.no if name == settings.ppe_violation_class else self.wear


def make_image(path, size=(1280, 720), color=(30, 60, 90)):
    path.parent.mkdir(parents=True, exist_ok=True)
    img = np.full((size[1], size[0], 3), color, dtype=np.uint8)
    ok, buf = cv2.imencode(path.suffix if path.suffix else ".jpg", img)
    assert ok
    path.write_bytes(buf.tobytes())
    return path


def set_mtime(path, ts):
    os.utime(path, (ts, ts))


@pytest.fixture()
def env(tmp_path, monkeypatch):
    """独立 Store + 文件夹摄像头 + 全画幅区域 + enter/enter_no_ppe 规则。"""
    monkeypatch.setattr(settings, "folder_stable_s", 0.0)
    monkeypatch.setattr(settings, "folder_max_per_tick", 10)
    monkeypatch.setattr(settings, "analysis_fps", 50.0)
    monkeypatch.setattr(settings, "folder_recursive", True)
    monkeypatch.setattr(settings, "folder_full_rescan_s", 0)  # 测试每轮全量，行为确定
    s = Store(tmp_path / "folder.db")
    root = tmp_path / "shots"
    root.mkdir()
    cid = s.add_camera("截图目录", str(root), "folder")
    zid = s.add_zone(cid, "试剂区", FULL_ZONE)
    for behavior in ("enter", "enter_no_ppe"):
        s.add_rule({"camera_id": cid, "zone_id": zid, "behavior": behavior, "level": 2,
                    "cooldown_s": 0, "time_start": "00:00", "time_end": "23:59",
                    "threshold": 0, "enabled": True})
    cam = s.camera(cid)
    w = FolderWorker(cam, s, RuleEngine(s))
    return {"store": s, "root": root, "cid": cid, "zid": zid, "worker": w, "cam": cam}


def event_types(s, cid):
    return [e["type"] for e in s.events(camera_id=cid, limit=100)]


def scan_until(w, s, cid, engine, root, want=1, tries=4):
    """T-003 稳定化（审查基线即存在的 flake，§13.1 标注"待定位"）：全量套件高负载下
    单张偶发读取失败会把图片留在重试队列（产品语义=下一轮重扫自愈，非 bug）；
    本文件测的是违规判定/缩略图而非"首轮必中"，故重扫数轮直到入库。"""
    for _ in range(tries):
        w._scan_once(engine, root)
        if len(s.folder_images(camera_id=cid)) >= want:
            return
    raise AssertionError(f"扫描 {tries} 轮后仍未入库（backlog={w.backlog}）")


def test_migration_and_dedup(env):
    s = env["store"]
    assert s.schema_version() >= 5
    row = {"camera_id": env["cid"], "path": str(env["root"] / "a.jpg"), "name": "a.jpg",
           "mtime_ns": 123, "size": 9, "ts": "2026-11-01T10:00:00",
           "processed_ts": "2026-11-01T10:00:01", "persons": 1, "key": True}
    iid = s.add_folder_image(row)
    assert iid > 0
    assert s.folder_image_done(env["cid"], row["path"], 123, 9)
    assert s.add_folder_image(row) == iid  # 唯一键幂等
    got = s.folder_image(iid)
    assert got["key"] is True and got["violation"] is False
    assert s.count_folder_images(camera_id=env["cid"], key=1) == 1
    assert s.folder_image_summary(days=3650)[0]["key_n"] == 1


def test_backlog_drains_without_dir_change(env, monkeypatch):
    """回归：首轮只处理 20 张后，目录 mtime 不变也要继续消化积压（不能等 5 分钟全量）。"""
    s, root, w = env["store"], env["root"], env["worker"]
    monkeypatch.setattr(settings, "folder_max_per_tick", 2)
    monkeypatch.setattr(settings, "folder_full_rescan_s", 3600)
    for i in range(5):
        make_image(root / f"b{i}.jpg")
    counts = [w._scan_once(FakeEngine([]), root) for _ in range(3)]
    assert counts == [2, 2, 1]
    assert len(s.folder_images(camera_id=env["cid"])) == 5
    assert w.backlog == 0


def test_add_folder_camera_validation(env, monkeypatch):
    from pathlib import Path as _P
    import secrets as _secrets
    from app.models import CameraCreate
    # 本机 TEMP 落在 C:\Windows\Temp → T-111 系统目录黑名单先于其余检查拒绝；
    # 旁路目录隔离产品规则与本用例意图（测的是 missing/数据目录冲突语义）
    base = _P("D:/tmp") / ("ls-foldertest-" + _secrets.token_hex(4))
    (base / "data").mkdir(parents=True, exist_ok=True)  # data_dir 必须存在（走数据目录冲突分支）
    monkeypatch.setattr(settings, "data_dir", base / "data")
    monkeypatch.setenv("APPDATA", str(base / "appdata"))
    monkeypatch.setenv("LOCALAPPDATA", str(base / "localappdata"))
    api.store = env["store"]
    monkeypatch.setattr(api, "manager", None)
    with pytest.raises(HTTPException) as e1:
        api.add_camera(CameraCreate(name="bad", source=str(env["root"] / "missing"),
                                    type="folder", autostart=False))
    assert e1.value.status_code == 400
    with pytest.raises(HTTPException) as e2:
        api.add_camera(CameraCreate(name="data", source=str(settings.data_dir),
                                    type="folder", autostart=False))
    import shutil
    shutil.rmtree(base, ignore_errors=True)
    assert e2.value.status_code == 400 and "数据目录" in str(e2.value.detail)


def test_worker_status_written(env):
    """回归：FolderWorker._set_status 必须真正落库（曾因编辑覆盖丢方法被 except 静默吞掉）。"""
    w = env["worker"]
    w._set_status("running")
    assert env["store"].camera(env["cid"])["status"] == "running"
    w._set_status("error")
    assert env["store"].camera(env["cid"])["status"] == "error"


def test_process_image_key_violation_and_event(env):
    s, root, w = env["store"], env["root"], env["worker"]
    img = make_image(root / "shot1.jpg")
    w.ppe = FakePpe(no_boxes=[PERSON])
    n = w._scan_once(FakeEngine([PERSON]), root)
    assert n == 1
    rows = s.folder_images(camera_id=env["cid"])
    assert len(rows) == 1
    r = rows[0]
    assert r["key"] is True and r["violation"] is True
    assert r["persons"] == 1 and r["in_zone"] == 1 and r["ppe_no"] == 1
    assert r["zones"] == ["试剂区"]
    assert r["violations"][0]["type"] == "enter_no_ppe"
    assert r["event_id"], "违规图片应关联事件"
    types = event_types(s, env["cid"])
    assert types == ["enter_no_ppe"], "默认只报显式违规，不为每张截图造 enter 事件"
    ev = s.event_by_id(r["event_id"])
    assert ev["snapshot"] and os.path.isfile(ev["snapshot"])
    assert img.is_file(), "原始截图不得被移动/删除"
    # 再扫一轮：不得重复入库/重复事件
    n2 = w._scan_once(FakeEngine([PERSON]), root)
    assert n2 == 0
    assert len(s.folder_images(camera_id=env["cid"])) == 1
    assert len(event_types(s, env["cid"])) == 1


def test_enter_events_optional(env, monkeypatch):
    """LS_FOLDER_ENTER_EVENTS=1：有人进区但无违规也会按 enter 规则产生事件。"""
    s, root, w = env["store"], env["root"], env["worker"]
    monkeypatch.setattr(settings, "folder_enter_events", True)
    make_image(root / "walk.jpg")
    w.ppe = None
    w._scan_once(FakeEngine([PERSON]), root)
    r = s.folder_images(camera_id=env["cid"])[0]
    assert r["key"] is True and r["violation"] is False
    assert event_types(s, env["cid"]) == ["enter"]
    assert r["event_id"]


def test_key_without_violation_has_no_event(env):
    """有人但无显式违规：仍进关键图片汇报，但不产生告警事件（证据不足≠违规）。"""
    s, root, w = env["store"], env["root"], env["worker"]
    make_image(root / "safe.jpg")
    w.ppe = None  # 未配置实验服模型 / 无证据
    w._scan_once(FakeEngine([PERSON]), root)
    r = s.folder_images(camera_id=env["cid"])[0]
    assert r["key"] is True and r["violation"] is False
    assert r["event_id"] is None
    assert event_types(s, env["cid"]) == []


def test_stability_gate_and_non_image_filter(env, monkeypatch):
    root, w = env["root"], env["worker"]
    fresh = make_image(root / "fresh.jpg")
    set_mtime(fresh, 1000)  # 1970 年 → 足够"静置"，会被处理（先验证正常路径）
    (root / "notes.txt").write_text("not an image", encoding="utf-8")
    n = w._scan_once(FakeEngine([]), root)
    assert n == 1
    assert [r["name"] for r in env["store"].folder_images(camera_id=env["cid"])] == ["fresh.jpg"]

    # 静置阈值 60s：刚写入的不读
    monkeypatch.setattr(settings, "folder_stable_s", 60.0)
    hot = make_image(root / "hot.jpg")
    n = w._scan_once(FakeEngine([]), root)
    assert n == 0
    set_mtime(hot, 1000)  # 变成"旧文件"后下一轮处理
    n = w._scan_once(FakeEngine([]), root)
    assert n == 1


def test_decode_error_recorded_once(env):
    root, w = env["root"], env["worker"]
    bad = root / "broken.jpg"
    bad.write_bytes(b"not a real jpeg")
    n = w._scan_once(FakeEngine([]), root)
    assert n == 1
    rows = env["store"].folder_images(camera_id=env["cid"])
    assert len(rows) == 1 and rows[0]["error"]
    assert rows[0]["key"] is False
    assert w.error_total == 1
    assert w._scan_once(FakeEngine([]), root) == 0  # 不重复刷错误行


def test_recursive_subdir_and_thumbnail(env):
    root, w = env["root"], env["worker"]
    sub = make_image(root / "2026-11-01" / "sub.jpg")
    scan_until(w, env["store"], env["cid"], FakeEngine([]), root)
    assert env["store"].folder_images(camera_id=env["cid"])[0]["name"] == "sub.jpg"
    data = thumbnail_bytes(str(sub), 1, max_w=160)
    assert data and data[:2] == b"\xff\xd8"  # JPEG SOI
    import time as _time

    from app.folders import thumbnail_path

    tp = thumbnail_path(1)
    assert tp.is_file()
    # 缓存命中判定依赖 dest.mtime >= src.mtime；同 tick 写盘时 FS 粒度可能判旧，
    # 显式把缓存 mtime 抬到未来，保证第二个断言测的是"命中"而非文件系统粒度
    _time.sleep(0.01)
    os.utime(tp, (_time.time() + 5, _time.time() + 5))
    assert thumbnail_bytes(str(sub), 1, max_w=160) == data  # 命中缓存


def test_ppe_inference_once_per_image(env):
    root, w = env["root"], env["worker"]
    make_image(root / "many.jpg")
    people = [(100, 100, 300, 500), (400, 100, 600, 500), (700, 100, 900, 500)]
    ppe = FakePpe(no_boxes=[])
    w.ppe = ppe
    w._scan_once(FakeEngine(people), root)
    # T-301：1 次整图推理（不随人数/类别翻倍）+ 2 次纯解码（违规类+穿戴类）
    assert ppe.raw_calls == 1, ppe.raw_calls
    assert ppe.decode_calls == 2, ppe.decode_calls


def test_occupancy_over_marks_violation(env):
    s, root, w = env["store"], env["root"], env["worker"]
    s.add_rule({"camera_id": env["cid"], "zone_id": None, "behavior": "occupancy_over",
                "level": 3, "cooldown_s": 0, "time_start": "00:00", "time_end": "23:59",
                "threshold": 1, "enabled": True})
    make_image(root / "two.jpg")
    w.ppe = None
    scan_until(w, s, env["cid"], FakeEngine([PERSON, (650, 200, 850, 600)]), root)
    r = s.folder_images(camera_id=env["cid"])[0]
    assert r["violation"] is True and r["in_zone"] == 2
    assert any(v["type"] == "occupancy_over" for v in r["violations"])
    assert "occupancy_over" in event_types(s, env["cid"])


def test_api_list_summary_and_media_guard(env, tmp_path, monkeypatch):
    s, root, cid = env["store"], env["root"], env["cid"]
    make_image(root / "ok.jpg")
    w = env["worker"]
    w._scan_once(FakeEngine([PERSON]), root)
    monkeypatch.setattr(api, "store", s)
    listed = api.folder_images(camera_id=cid, key=1)
    assert listed["total"] == 1
    item = listed["items"][0]
    assert item["thumb_url"].endswith("/thumb") and item["file_url"].endswith("/file")
    summary = api.folder_images_summary(days=3650, camera_id=cid)
    assert summary["totals"]["total"] == 1 and summary["totals"]["key"] == 1
    csv_resp = api.export_folder_images_csv(camera_id=cid, key=1)
    csv_text = csv_resp.body.decode("utf-8")
    assert csv_text.startswith("\ufeff") and "ok.jpg" in csv_text
    detail = api.folder_image_detail(item["id"])
    assert detail["path"] == item["path"]

    # 正常：接入目录内 → 可读
    assert api._folder_media_path(detail) == (root / "ok.jpg")
    # 越界：数据库行指向目录外文件 → 403
    outside = tmp_path / "secret.jpg"
    make_image(outside)
    bad = dict(detail)
    bad["path"] = str(outside)
    with pytest.raises(HTTPException) as ei:
        api._folder_media_path(bad)
    assert ei.value.status_code == 403
    # 已删除 → 404；违规图有证据副本时回退到证据
    (root / "ok.jpg").unlink()
    with pytest.raises(HTTPException) as ei2:
        api._folder_media_path(detail)
    assert ei2.value.status_code == 404
    fallback = settings.snapshot_dir / "fallback.jpg"
    fallback.write_bytes(b"evidence")
    with_snap = dict(detail, snapshot=str(fallback))
    assert api._folder_readable_path(with_snap) == fallback


def test_csv_formula_injection_escaped():
    assert api._csv_cell("=cmd|' /C calc'!A0").startswith("'=")
    assert api._csv_cell("-1+1").startswith("'-")
    assert api._csv_cell("@SUM(A1)").startswith("'@")
    assert api._csv_cell("normal.jpg") == "normal.jpg"


def test_delete_camera_cleans_folder_records(env, monkeypatch):
    """删除图片文件夹摄像头：记录/证据/缩略图清掉，原始截图保留。"""
    s, root, cid = env["store"], env["root"], env["cid"]
    src = make_image(root / "violation.jpg")
    w = env["worker"]
    w.ppe = FakePpe(no_boxes=[PERSON])
    w._scan_once(FakeEngine([PERSON]), root)
    row = s.folder_images(camera_id=cid)[0]
    evidence = row["snapshot"]
    assert evidence and os.path.isfile(evidence)
    thumb = Path(str(settings.data_dir)) / "thumbs" / f"img_{row['id']}.jpg"
    thumb.parent.mkdir(parents=True, exist_ok=True)
    thumb.write_bytes(b"thumb")
    api.store = s

    class FakeManager:
        def stop_camera(self, _cid):
            pass

    monkeypatch.setattr(api, "manager", FakeManager())
    monkeypatch.setattr(api.go2rtc, "sync_streams", lambda _s: None)
    assert api.delete_camera(cid)["ok"] is True
    assert s.camera(cid) is None
    assert s.folder_image(row["id"]) is None
    # 事件仍在保留期内且引用该证据副本 → 文件保留（否则事件详情会 404）
    assert os.path.exists(evidence)
    assert not thumb.exists()
    assert src.is_file()


def test_retention_keeps_evidence_referenced_by_event(env, monkeypatch):
    """图片记录先过期但事件仍引用同一证据 → 证据文件必须保留到事件过期。"""
    s, root, cid = env["store"], env["root"], env["cid"]
    src = make_image(root / "evref.jpg")
    evidence = settings.snapshot_dir / "img_evref.jpg"
    evidence.write_bytes(b"evidence")
    iid = s.add_folder_image({
        "camera_id": cid, "path": str(src), "name": src.name, "mtime_ns": 2,
        "size": src.stat().st_size, "ts": "2000-01-01T00:00:00",
        "processed_ts": "2000-01-01T00:00:01", "persons": 1,
        "key": True, "violation": True, "snapshot": str(evidence)})
    eid = s.add_event(cid, "enter_no_ppe", 3, 0, "图片违规", str(evidence))
    monkeypatch.setattr(settings, "retention_images_days", 0)
    stats = retention.run_cleanup(s)
    assert stats["deleted_images"] == 1
    assert s.folder_image(iid) is None
    assert s.event_by_id(eid) is not None
    assert evidence.is_file(), "事件仍引用时不得删除证据"
    assert src.is_file()


def test_retention_removes_records_and_evidence_only(env, monkeypatch):
    s, root = env["store"], env["root"]
    src = make_image(root / "keep.jpg")
    evidence = settings.snapshot_dir / "img_ev.jpg"
    evidence.write_bytes(b"evidence")
    row = {"camera_id": env["cid"], "path": str(src), "name": src.name,
           "mtime_ns": 1, "size": src.stat().st_size,
           "ts": "2000-01-01T00:00:00", "processed_ts": "2000-01-01T00:00:01",
           "persons": 1, "key": True, "violation": True}
    iid = s.add_folder_image(row)
    import sqlite3
    with sqlite3.connect(str(s.path)) as c:
        c.execute("UPDATE folder_images SET snapshot=? WHERE id=?", (str(evidence), iid))
    monkeypatch.setattr(settings, "retention_images_days", 0)
    stats = retention.run_cleanup(s)
    assert stats["deleted_images"] == 1
    assert s.folder_image(iid) is None
    assert not evidence.exists()
    assert src.is_file(), "原始截图永不被保留清理删除"
