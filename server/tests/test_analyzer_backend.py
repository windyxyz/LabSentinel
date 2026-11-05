"""T-310 回归：AnalyzerBackend 抽象 + v12 灰度列。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app.analyzer import AnalyzerBackend, EventCandidate, InProcessAnalyzer  # noqa: E402
from app.config import settings  # noqa: E402
from app.store import Store  # noqa: E402


def test_v12_analyzer_column(tmp_path):
    s = Store(tmp_path / "s.db")
    assert s.schema_version() >= 12
    cid = s.add_camera("cam", "x.mp4", "file")
    cam = s.camera(cid)
    assert cam.get("analyzer", "python") == "python"  # 默认 python


def test_inprocess_backend_contract(tmp_path):
    """InProcessAnalyzer 满足接口契约（包装 CameraManager，行为不变）。"""
    assert issubclass(InProcessAnalyzer, AnalyzerBackend)
    b = InProcessAnalyzer()
    assert b.status() == []  # 未绑定 manager 时空状态
    # start/stop 在未绑定 manager 时为安全 no-op（不抛）
    b.start_camera({"id": 1, "name": "x", "source": "x.mp4", "type": "file"}, [], [])
    b.stop_camera(1)


def test_event_candidate_proto_fields():
    """EventCandidate 字段与 proto/analyzer.proto EventCandidate 对齐。"""
    ec = EventCandidate(camera_id=1, behavior="enter", level=2, track_id=3,
                        detail="d", ts_iso="2026-01-01T00:00:01", zone_id=None,
                        snapshot_jpeg=b"", evidence_key="k")
    assert ec.behavior == "enter" and ec.snapshot_jpeg == b""


def test_analyzer_default_setting():
    assert settings.analyzer_default in ("python", "rust")
