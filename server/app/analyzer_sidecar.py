"""T-327：Rust sidecar（ls-analyzer.exe）的 Python 客户端——AnalyzerBackend 实现。

生命周期：spawn（一次性令牌）→ 等 READY 行 → gRPC 通道 → ApplyConfig/Start/Stop/Status。
崩溃守护由上层（CameraManager supervisor 语义）负责；此处只做进程与通道管理。
"""
from __future__ import annotations

import logging
import os
import secrets
import subprocess
import time
from pathlib import Path

from .analyzer import AnalyzerBackend, AnalyzerStatusRow

log = logging.getLogger("app.analyzer_sidecar")

_DEFAULT_EXE = Path("D:/tmp/ls-rs-target/debug/ls-analyzer.exe")


class SidecarAnalyzer(AnalyzerBackend):
    """单个 sidecar 进程 = 全部 rust 摄像头（按 cameras.analyzer='rust' 灰度）。"""

    def __init__(self, exe: Path | None = None, port: int = 0):
        self._exe = Path(exe or os.environ.get("LS_ANALYZER_EXE", _DEFAULT_EXE))
        self._port = port
        self._proc: subprocess.Popen | None = None
        self._token = ""
        self._stub = None
        self._started = False

    # ---- 进程生命周期 ----
    def start(self) -> None:
        if self._started:
            return
        if not self._exe.exists():
            raise FileNotFoundError(f"sidecar 不存在: {self._exe}")
        self._token = secrets.token_urlsafe(24)
        self._port = self._port or self._free_port()
        self._proc = subprocess.Popen(
            [str(self._exe), "--port", str(self._port), "--token", self._token],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        deadline = time.time() + 15
        while time.time() < deadline:
            line = self._proc.stdout.readline().decode("utf-8", "replace")
            if line.startswith("READY"):
                break
            if self._proc.poll() is not None:
                raise RuntimeError(f"sidecar 提前退出: {line}")
        else:
            self.stop()
            raise TimeoutError("sidecar 15s 未就绪")
        import grpc
        channel = grpc.insecure_channel(f"127.0.0.1:{self._port}")
        grpc.channel_ready_future(channel).result(timeout=10)
        from . import analyzer_pb2, analyzer_pb2_grpc
        self._stub = analyzer_pb2_grpc.AnalyzerStub(channel)
        self._started = True

    def stop(self) -> None:
        self._started = False
        self._stub = None
        if self._proc and self._proc.poll() is None:
            self._proc.terminate()
            try:
                self._proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self._proc.kill()
        self._proc = None

    @staticmethod
    def _free_port() -> int:
        import socket
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
        sidecar_port = port
        s.close()
        return sidecar_port

    # ---- AnalyzerBackend 接口 ----
    def start_camera(self, camera: dict, zones: list[dict], rules: list[dict]) -> None:
        self.start()
        from . import analyzer_pb2 as pb
        self._stub.ApplyConfig(pb.Config(version=1, cameras=[], zones=[], rules=[]),
                               metadata=(("x-ls-token", self._token),))
        self._stub.StartCamera(pb.CameraRef(camera_id=camera["id"]),
                               metadata=(("x-ls-token", self._token),))

    def stop_camera(self, camera_id: int) -> None:
        if self._stub is None:
            return
        from . import analyzer_pb2 as pb
        self._stub.StopCamera(pb.CameraRef(camera_id=camera_id),
                              metadata=(("x-ls-token", self._token),))

    def status(self) -> list[AnalyzerStatusRow]:
        if self._stub is None:
            return []
        from . import analyzer_pb2 as pb
        resp = self._stub.Status(pb.Empty(), metadata=(("x-ls-token", self._token),))
        return [AnalyzerStatusRow(camera_id=c.camera_id, state=c.state,
                                  analysis_fps_now=c.analysis_fps_now)
                for c in resp.cameras]
