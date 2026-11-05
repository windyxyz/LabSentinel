"""T-310（L1 前置）：分析器后端抽象。

现有 Python 管线（CameraWorker）包装为 InProcessAnalyzer；Rust sidecar（ls-analyzer，
T-327）实现同一接口 SidecarAnalyzer——按摄像头灰度 analyzer=python|rust 切换
（cameras.analyzer 列，迁移 v12），全局默认 LS_ANALYZER_DEFAULT=python。
"""
from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass
from typing import Optional


@dataclass
class EventCandidate:
    """事件候选（未入库）；字段与 proto/analyzer.proto EventCandidate 一一对应。"""
    camera_id: int
    behavior: str
    level: int
    track_id: int
    detail: str
    ts_iso: str = ""
    zone_id: Optional[int] = None
    snapshot_jpeg: bytes = b""
    evidence_key: str = ""


@dataclass
class AnalyzerStatusRow:
    camera_id: int
    state: "str" = "stopped"
    analysis_fps_now: float = 0.0
    frame_age_s: float = 0.0
    reconnect_count: int = 0
    last_error: str = ""
    infer_ms_p50: float = 0.0
    queue_depth: int = 0


class AnalyzerBackend(ABC):
    """分析器后端接口：生命周期 + 状态。配置以版本化全量快照下发（T-320 Config）。"""

    @abstractmethod
    def start_camera(self, camera: dict, zones: list[dict], rules: list[dict]) -> None: ...

    @abstractmethod
    def stop_camera(self, camera_id: int) -> None: ...

    @abstractmethod
    def status(self) -> list[AnalyzerStatusRow]: ...


class InProcessAnalyzer(AnalyzerBackend):
    """现有 Python 管线的包装（默认后端，行为与主线完全一致）。"""

    def __init__(self, manager=None):
        self._manager = manager

    def bind(self, manager) -> None:
        self._manager = manager

    def start_camera(self, camera: dict, zones: list[dict], rules: list[dict]) -> None:
        if self._manager is not None:
            self._manager.start_camera(camera)

    def stop_camera(self, camera_id: int) -> None:
        if self._manager is not None:
            self._manager.stop_camera(camera_id)

    def status(self) -> list[AnalyzerStatusRow]:
        if self._manager is None:
            return []
        return [_row_from_manager(cam) for cam in self._manager.status()["cameras"]]


def _row_from_manager(cam: dict) -> AnalyzerStatusRow:
    return AnalyzerStatusRow(
        camera_id=cam["id"],
        state=cam.get("status", "stopped"),
        analysis_fps_now=float(cam.get("analysis_fps_now", 0.0) or 0.0),
        frame_age_s=float(cam.get("frame_age_s", 0.0) or 0.0),
        reconnect_count=int(cam.get("reconnect_count", 0) or 0),
        last_error=str(cam.get("last_error", "") or ""),
    )
