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
        # T-331：同一 sidecar 托管多路 → 本地累积全量配置快照，按版本整体下发
        self._cams: dict[int, dict] = {}
        self._zones: dict[int, dict] = {}
        self._rules: dict[int, dict] = {}
        self._cfg_version = 0

    # ---- 进程生命周期 ----
    def start(self) -> None:
        if self._started:
            return
        if not self._exe.exists():
            raise FileNotFoundError(f"sidecar 不存在: {self._exe}")
        self._token = secrets.token_urlsafe(24)
        self._port = self._port or self._free_port()
        from .config import settings
        # T-325：Rust 检测层继承同一模型/阈值/迟滞参数（pydantic settings 不导出到
        # os.environ，必须显式传给子进程）
        child_env = {
            **os.environ,
            "LS_MODEL_FP32": str(settings.model_fp32),
            "LS_IMGSZ": str(settings.imgsz),
            "LS_CONF_THRES": str(settings.conf_thres),
            "LS_IOU_THRES": str(settings.iou_thres),
            "LS_MIN_BOX_AREA": str(settings.min_box_area),
            "LS_ZONE_ENTER_FRAMES": str(settings.zone_enter_frames),
            "LS_ZONE_EXIT_FRAMES": str(settings.zone_exit_frames),
            # T-326：存证写盘目录/抽帧率（relpath 相对 data_dir 回传）
            "LS_VIDEO_DIR": str(settings.video_dir),
            "LS_DATA_DIR": str(settings.data_dir),
            "LS_EVIDENCE_FPS": str(settings.evidence_fps),
        }
        self._proc = subprocess.Popen(
            [str(self._exe), "--port", str(self._port), "--token", self._token],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
            env=child_env,
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
        # stdout 持续排水：READY 后若无人读管道，sidecar 的后续打印（模型加载/
        # 存证落盘）会写满管道缓冲 → 子进程整进程阻塞 → gRPC 流全部断连。
        # 排水线程保留最近日志行供诊断（有界）。
        import collections
        import threading as _th
        self._out_log: collections.deque[str] = collections.deque(maxlen=50)
        self._out_thread = _th.Thread(target=self._drain_stdout, daemon=True,
                                      name="sidecar-stdout")
        self._out_thread.start()
        import grpc
        channel = grpc.insecure_channel(f"127.0.0.1:{self._port}")
        grpc.channel_ready_future(channel).result(timeout=10)
        from . import analyzer_pb2_grpc
        self._stub = analyzer_pb2_grpc.AnalyzerStub(channel)
        self._started = True

    def _drain_stdout(self) -> None:
        """sidecar stdout 排水线程（进程存活期间持续读取）。"""
        proc = self._proc
        try:
            for raw in proc.stdout:
                line = raw.decode("utf-8", "replace").rstrip()
                self._out_log.append(line)
                if line.startswith("[ls-analyzer]"):
                    print(f"[sidecar-out] {line}")
        except (ValueError, OSError):
            pass  # 进程退出/管道关闭

    def out_log(self) -> list[str]:
        """最近 sidecar 输出（诊断用）。"""
        return list(getattr(self, "_out_log", ()))

    def stop(self) -> None:
        self._started = False
        self._stub = None
        self._cams.clear()
        self._zones.clear()
        self._rules.clear()
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
    def _proto_config(self):
        """把当前累积快照转成 proto Config（T-331：Rust 数据面需要源码/区域/规则）。"""
        from . import analyzer_pb2 as pb
        from .config import settings
        cams = [
            pb.CameraConfig(id=c["id"], name=c.get("name") or "",
                            type=c.get("type") or "file",
                            source=c.get("source") or "",
                            analysis_fps=float(settings.analysis_fps),
                            file_loop=bool(settings.file_loop))
            for c in self._cams.values()
        ]
        zones = []
        for z in self._zones.values():
            pts = [pb.Point(x=int(p[0]), y=int(p[1]))
                   for p in (z.get("polygon") or []) if len(p) >= 2]
            if len(pts) >= 3:
                zones.append(pb.ZoneConfig(id=z["id"], camera_id=z["camera_id"],
                                           name=z.get("name") or "", points=pts))
        rules = [
            pb.RuleConfig(
                id=r["id"], camera_id=r["camera_id"],
                zone_id=int(r.get("zone_id") or 0),
                behavior=r.get("behavior") or "",
                level=int(r.get("level") or 2),
                cooldown_s=int(r.get("cooldown_s") or 0),
                time_start=r.get("time_start") or "00:00",
                time_end=r.get("time_end") or "23:59",
                weekdays=str(r.get("days_of_week") or ""),
                threshold=int(r.get("threshold") or 0),
                enabled=bool(r.get("enabled", True)),
            )
            for r in self._rules.values()
        ]
        return pb.Config(version=self._cfg_version, cameras=cams, zones=zones, rules=rules)

    def start_camera(self, camera: dict, zones: list[dict], rules: list[dict]) -> None:
        self.start()
        self._cams[int(camera["id"])] = camera
        for z in zones or []:
            self._zones[int(z["id"])] = z
        for r in rules or []:
            self._rules[int(r["id"])] = r
        self._cfg_version += 1
        from . import analyzer_pb2 as pb
        self._stub.ApplyConfig(self._proto_config(),
                               metadata=(("x-ls-token", self._token),))
        self._stub.StartCamera(pb.CameraRef(camera_id=camera["id"]),
                               metadata=(("x-ls-token", self._token),))

    def stop_camera(self, camera_id: int) -> None:
        if self._stub is None:
            return
        from . import analyzer_pb2 as pb
        self._stub.StopCamera(pb.CameraRef(camera_id=camera_id),
                              metadata=(("x-ls-token", self._token),))

    def finalize_evidence(self, event_id: int, camera_id: int, evidence_key: str,
                          pre_seconds: int = 2, post_seconds: int = 2,
                          timeout: float = 15.0) -> dict | None:
        """T-326：事件 id 回传 → Rust 补齐事件前后片段视频（阻塞至写盘完成）。

        返回 EvidenceResult 字段 dict（ok / video_relpath / message）；
        sidecar 不可用或 RPC 失败返回 None（事件不受影响，仅无视频存证）。"""
        if self._stub is None:
            return None
        from . import analyzer_pb2 as pb
        try:
            resp = self._stub.FinalizeEvidence(
                pb.EvidenceRequest(event_id=event_id, camera_id=camera_id,
                                   evidence_key=evidence_key,
                                   pre_seconds=pre_seconds, post_seconds=post_seconds),
                timeout=timeout, metadata=(("x-ls-token", self._token),))
            return {"ok": resp.ok, "video_relpath": resp.video_relpath,
                    "message": resp.message}
        except Exception as e:  # noqa: BLE001  存证失败不拖累事件链
            log.warning("finalize_evidence(ev=%s) 失败: %s", event_id, e)
            return None

    def status(self) -> list[AnalyzerStatusRow]:
        if self._stub is None:
            return []
        from . import analyzer_pb2 as pb
        resp = self._stub.Status(pb.Empty(), metadata=(("x-ls-token", self._token),))
        return [AnalyzerStatusRow(camera_id=c.camera_id, state=c.state,
                                  analysis_fps_now=c.analysis_fps_now)
                for c in resp.cameras]
