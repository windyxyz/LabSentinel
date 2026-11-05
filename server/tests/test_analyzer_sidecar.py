"""T-327 集成：Python SidecarAnalyzer ↔ Rust ls-analyzer.exe 端到端。

exe 缺失（CI 干净检出）自动 skip；本机构建产物在 D:/tmp/ls-rs-target/debug/。
"""
import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402

from app.analyzer import AnalyzerBackend  # noqa: E402
from app.analyzer_sidecar import SidecarAnalyzer  # noqa: E402

EXE = Path(os.environ.get("LS_ANALYZER_EXE", "D:/tmp/ls-rs-target/debug/ls-analyzer.exe"))
pytestmark = pytest.mark.skipif(not EXE.exists(), reason="ls-analyzer.exe 未构建")


@pytest.fixture()
def sidecar():
    sa = SidecarAnalyzer(exe=EXE)
    sa.start()
    yield sa
    sa.stop()


def test_sidecar_status_roundtrip(sidecar):
    rows = sidecar.status()
    assert rows == []  # 未启动摄像头时空状态


def test_sidecar_camera_lifecycle(sidecar):
    sidecar.start_camera({"id": 7, "name": "t", "source": "x.mp4", "type": "file"}, [], [])
    rows = sidecar.status()
    assert len(rows) == 1 and rows[0].camera_id == 7 and rows[0].state == "running"
    sidecar.stop_camera(7)
    rows = sidecar.status()
    assert rows[0].state == "stopped"


def test_sidecar_is_backend():
    assert issubclass(SidecarAnalyzer, AnalyzerBackend)
