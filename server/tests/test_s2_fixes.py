"""T-203/T-208 回归：统计当天口径 + MQTT has_snapshot/camera_name。"""
import json
import os
import sys
from datetime import datetime

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app import configcache  # noqa: E402
from app.mqtt_bridge import build_payload  # noqa: E402
from app.stats import quality  # noqa: E402
from app.store import Store  # noqa: E402


def test_quality_counts_today_events(tmp_path):
    """B-06：'今天 00:00:01' 的事件在 days=1 必须计入（原空格分隔 ts 把当天全排除）。"""
    s = Store(tmp_path / "s.db")
    cid = s.add_camera("c", "x.mp4", "file")
    today = datetime.now().replace(hour=0, minute=0, second=1, microsecond=0)
    eid = s.add_event(cid, "enter", 2, 1, "t", ts=today.isoformat(timespec="seconds"))
    s.set_event_status(eid, "acked")
    q = quality(s, days=1)
    assert q["total"] >= 1, q


def test_mqtt_has_snapshot_from_boolean_and_camera_name(tmp_path):
    """B-14：派发前行 snapshot 已 pop，has_snapshot 布尔可读；camera_name 注入。"""
    s = Store(tmp_path / "s.db")
    cid = s.add_camera("cams-name", "x.mp4", "file")
    row = {"id": 9, "ts": "2026-11-05T10:00:00", "camera_id": cid, "type": "enter",
           "level": 2, "track_id": 3, "detail": "d", "has_snapshot": True}
    payload = json.loads(build_payload(row, camera_name="cams-name"))
    assert payload["has_snapshot"] is True
    assert payload["camera_name"] == "cams-name"
    # 直传完整行（旧调用形态）也兼容
    row2 = dict(row, snapshot="C:/some.jpg")
    row2.pop("has_snapshot")
    assert json.loads(build_payload(row2, None))["has_snapshot"] is True


def test_camera_name_cache(tmp_path):
    s = Store(tmp_path / "s.db")
    cid = s.add_camera("名称A", "x.mp4", "file")
    assert configcache.camera_name(s, cid) == "名称A"
    s.add_camera("名称B", "x.mp4", "file")
    # 写入后版本 bump，缓存自动重建
    assert configcache.camera_name(s, cid + 1) == "名称B"
