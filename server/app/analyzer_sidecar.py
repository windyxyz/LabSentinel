"""T-327：Rust sidecar (ls-analyzer.exe) 的 Python 客户端——AnalyzerBackend 实现。

生命周期：spawn（一次性令牌）→ 等 READY 行 → gRPC 通道 → ApplyConfig/Start/Stop/Status。
T-327 验收：sidecar 进程崩溃后 ≤10s 自动恢复——本类自带 watchdog (ensure_running)，
事件桥断流重连与 CameraManager.supervise_tick 双路径触发；恢复时重放全量配置快照
并重拉每路摄像头，Python 服务不中断。
"""
from __future__ import annotations

import logging
import os
import secrets
import subprocess
import threading
import time
from pathlib import Path

from .analyzer import AnalyzerBackend, AnalyzerStatusRow

log = logging.getLogger("app.analyzer_sidecar")

def _default_exe() -> Path:
    """R2-506/N-06: no developer-machine paths. Resolution order:
    LS_ANALYZER_EXE -> portable install bin/ls-analyzer[.exe] -> cargo target dir
    (engine-rs/target, gitignored; CARGO_TARGET_DIR can override).
    R2-304 packages the binary into bin/ and sets LS_ANALYZER_EXE in the launcher.
    """
    env = os.environ.get("LS_ANALYZER_EXE")
    if env:
        return Path(env)
    here = Path(__file__).resolve()
    suffix = ".exe" if os.name == "nt" else ""
    portable = here.parents[2] / "bin" / f"ls-analyzer{suffix}"
    if portable.is_file():
        return portable
    target = Path(os.environ.get("CARGO_TARGET_DIR", str(here.parents[2] / "engine-rs" / "target")))
    return target / "debug" / f"ls-analyzer{suffix}"


_DEFAULT_EXE = _default_exe()


class SidecarAnalyzer(AnalyzerBackend):
    """单个 sidecar 进程 = 全部 rust 摄像头（按 cameras.analyzer='rust' 灰度）。"""

    def __init__(self, exe: Path | None = None, port: int = 0):
        self._exe = Path(exe or _DEFAULT_EXE)
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
        # T-327 watchdog：进程守护锁 + 恢复计数 + Store 配置版本（ApplyConfig 推送用）
        self._watch_lock = threading.Lock()
        self._restarts = 0
        self._last_restart_ts = ""
        self._store_config_version = -1
        self._on_recovered = None  # 由事件桥注入：恢复后重建订阅

    def set_recovery_hook(self, hook) -> None:
        """T-327：注入事件桥恢复回调（新 stub 建立后重启 Events 订阅）。"""
        self._on_recovered = hook

    def version(self) -> str:
        """T-336：返回 sidecar 版本（LS_ANALYZER_VERSION 环境变量优先，
        未设置时标 unknown——不写死版本串，避免与 Rust 构建元数据漂移）。"""
        return os.environ.get("LS_ANALYZER_VERSION") or "unknown"

    def watchdog_status(self) -> dict:
        """T-327：供 /api/system 与 soak 演练观测（不依赖默认 True 的宽松判定）。
        pid 供 soak 崩溃演练按 PID 精确 kill（2026-11-13 P3：按映像名全机杀会
        误伤同机其他安装的 ls-analyzer 实例）。"""
        alive = self._proc is not None and self._proc.poll() is None
        return {"running": bool(self._started and alive),
                "restarts": self._restarts,
                "last_restart_ts": self._last_restart_ts or None,
                "config_version": self._cfg_version,
                # getattr：测试桩 FakeProc 无 pid 属性（2026-11-13）
                "pid": getattr(self._proc, "pid", None) if alive else None}

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
            # R2-109/N-16: 一次性令牌不再出现在命令行（本机其它用户可从进程
            # 列表读到），改由环境变量传给子进程（同用户可见、其它用户不可见）。
            "LS_ANALYZER_TOKEN": self._token,
        }
        self._proc = subprocess.Popen(
            [str(self._exe), "--port", str(self._port)],
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
        try:
            grpc.channel_ready_future(channel).result(timeout=10)
            from . import analyzer_pb2_grpc
            self._stub = analyzer_pb2_grpc.AnalyzerStub(channel)
        except Exception:
            # 2026-11-13 P3（审查实证）：通道就绪超时不回收已 spawn 的 sidecar
            # （watchdog 兜底要等下一轮 ensure_running），且 _port 不复位会让
            # 直调路径复用被占端口连环失败——就地停进程+复位再抛。
            channel.close()
            try:
                self.stop()
            except Exception:  # noqa: BLE001 清理失败不掩盖原始异常
                pass
            self._port = 0
            raise
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

    def stop(self, keep_snapshot: bool = False) -> None:
        """停止进程；keep_snapshot=True 保留配置快照供 ensure_running 恢复重放。"""
        self._started = False
        self._stub = None
        if not keep_snapshot:
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

    def _replay(self) -> None:
        """T-327：把累积的全量快照重新下发并重拉每路摄像头（恢复/版本变化共用）。"""
        if self._stub is None:
            return
        from . import analyzer_pb2 as pb
        self._cfg_version += 1
        self._stub.ApplyConfig(self._proto_config(),
                               metadata=(("x-ls-token", self._token),))
        for cid in sorted(self._cams):
            self._stub.StartCamera(pb.CameraRef(camera_id=cid),
                                   metadata=(("x-ls-token", self._token),))

    def sync_config(self, store) -> bool:
        """T-327：Store.config_version 变化 → 重建区域/规则/相机快照 → ApplyConfig。

        返回是否真的推送了新版本（无变化时零开销，不触碰运行中的数据面）。
        """
        try:
            ver = store.config_version()
        except Exception:  # noqa: BLE001  旧 Store/测试桩无该方法时跳过
            return False
        if ver == self._store_config_version:
            return False
        self._store_config_version = ver
        if not self._started or self._stub is None:
            return False
        from . import configcache
        try:
            rust_cams = [c for c in store.cameras()
                         if (c.get("analyzer") or "python") == "rust"]
            self._cams = {int(c["id"]): dict(c) for c in rust_cams}
            self._zones = {}
            self._rules = {}
            for c in rust_cams:
                for z in configcache.zones(store, c["id"]):
                    self._zones[int(z["id"])] = dict(z)
            for r in configcache.rules(store):
                if int(r["camera_id"]) in self._cams:
                    self._rules[int(r["id"])] = dict(r)
        except Exception as e:  # noqa: BLE001  读配置失败不动存量快照
            log.warning("sidecar 配置快照刷新失败（沿用旧快照）: %s", e)
            return False
        self._replay()
        return True

    def ensure_running(self, store=None) -> bool:
        """T-327 watchdog：sidecar 崩溃后 ≤10s 自动恢复；顺带做配置版本推送。

        双触发：事件桥断流（1s 重连轮）与 CameraManager.supervise_tick（10s 巡检），
        任一路径发现进程死亡都会重建进程 + 重放全量快照 + 重拉每路摄像头。
        """
        alive = self._started and self._proc is not None and self._proc.poll() is None
        if alive:
            if store is not None:
                self.sync_config(store)
            return True
        with self._watch_lock:
            # 并发触发时只有一个线程执行恢复；其它线程复查后直接返回
            alive = self._started and self._proc is not None and self._proc.poll() is None
            if alive:
                return True
            log.warning("sidecar 进程已退出，开始自动恢复（第 %s 次）",
                        self._restarts + 1)
            try:
                self.stop(keep_snapshot=True)
                self._port = 0  # 重新分配端口（避免旧进程 TIME_WAIT 占端口）
                self.start()
                self._replay()
                self._restarts += 1
                from datetime import datetime
                self._last_restart_ts = datetime.now().isoformat(timespec="seconds")
                if self._on_recovered is not None:
                    self._on_recovered()
                print(f"[sidecar] 已恢复（第 {self._restarts} 次重启，"
                      f"重放 {len(self._cams)} 路配置）")
                return True
            except Exception as e:  # noqa: BLE001  恢复失败不影响 Python 主服务
                log.warning("sidecar 自动恢复失败（下一轮继续重试）: %s", e)
                return False

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
        """把当前累积快照转成 proto Config（T-331：Rust 数据面需要源码/区域/规则；
        R2-305：附带已解密的告警渠道快照——仅 127.0.0.1 gRPC + 令牌传输，明文只进
        数据面内存）。"""
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
        cfg = pb.Config(version=self._cfg_version, cameras=cams, zones=zones, rules=rules)
        # R2-305：渠道快照（settings 已含 alert_config.load 的 DB 解密覆盖；env 本来明文）
        ac = pb.AlertChannels()
        _s = str(getattr(settings, "wecom_webhook", "") or "")
        if _s.strip():
            ac.channels.append(pb.AlertChannel(channel="wecom", url=_s))
        _s = str(getattr(settings, "dingtalk_webhook", "") or "")
        if _s.strip():
            ac.channels.append(pb.AlertChannel(channel="dingtalk", url=_s))
        _s = str(getattr(settings, "feishu_webhook", "") or "")
        if _s.strip():
            ac.channels.append(pb.AlertChannel(channel="feishu", url=_s))
        _s = str(getattr(settings, "serverchan_sendkey", "") or "")
        if _s.strip():
            ac.channels.append(pb.AlertChannel(
                channel="serverchan",
                url=f"https://sctapi.ftqq.com/{_s.strip()}.send"))
        _s = str(getattr(settings, "pushplus_token", "") or "")
        if _s.strip():
            ac.channels.append(pb.AlertChannel(channel="pushplus", token=_s))
        _k = str(getattr(settings, "qmsg_key", "") or "")
        _q = str(getattr(settings, "qmsg_qq", "") or "")
        if _k.strip() and _q.strip():
            ac.channels.append(pb.AlertChannel(channel="qmsg", key=_k, qq=_q))
        _u = str(getattr(settings, "webhook_url", "") or "")
        if _u.strip():
            ac.channels.append(pb.AlertChannel(
                channel="webhook", url=_u,
                secret=str(getattr(settings, "webhook_secret", "") or ""),
                allow_http=bool(getattr(settings, "webhook_allow_http", False))))
        _h = str(getattr(settings, "smtp_host", "") or "")
        _t = str(getattr(settings, "alert_email_to", "") or "")
        if _h.strip() and _t.strip():
            ac.channels.append(pb.AlertChannel(
                channel="email",
                email=pb.EmailChannel(
                    host=_h,
                    port=int(getattr(settings, "smtp_port", 465) or 465),
                    user=str(getattr(settings, "smtp_user", "") or ""),
                    password=str(getattr(settings, "smtp_pass", "") or ""),
                    to=_t,
                    security=str(getattr(settings, "smtp_security", "ssl") or "ssl"))))
        if len(ac.channels):
            cfg.alert_channels.CopyFrom(ac)
        return cfg

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
