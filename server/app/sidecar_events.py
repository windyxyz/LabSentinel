"""T-331（L3 灰度）：Rust sidecar 事件流的 Python 落地桥。

`analyzer=rust` 的摄像头不走 Python CameraWorker，由 ls-analyzer 取流/推理/规则后
经 gRPC Events 流推送 EventCandidate；本模块负责：

- 起一个后台线程订阅流（阻塞迭代，失败自动重连，停止标志可退出）；
- 候选 → store.add_event（含 ts_iso 视频时间）→ 写快照 JPEG（如有）→
  dispatcher.dispatch 完整派发链（静默/渠道过滤/投递记录/WS/MQTT）；
- 去重：evidence_key（camera:ts:frame）+ behavior，防流重连重复入库；
- 状态：最近错误/已入库计数，供 /api/system 观测。

这是"Rust 完全替代 Python worker"的最后一环：Python 只剩入库/告警/控制面，
不再持有采集/推理/跟踪/规则逻辑（tools/ 训练链按 Q-16 长期保留）。
"""
from __future__ import annotations

import datetime as dt
import logging
import queue
import threading
from pathlib import Path

log = logging.getLogger("app.sidecar_events")


class SidecarEventBridge:
    """订阅一个 SidecarAnalyzer 的事件流并落库（每进程一个，随 lifespan 启停）。"""

    def __init__(self, store, dispatcher, analyzer):
        self.store = store
        self.dispatcher = dispatcher
        self.analyzer = analyzer
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._seen: set[str] = set()
        self._seen_order: list[str] = []
        self._lock = threading.Lock()
        self._ingested = 0
        self._last_error = ""
        self._last_ts = ""
        # T-326：存证收尾独立线程（finalize 阻塞 ~post 秒，不能拖慢事件流循环的派发）
        self._ev_q: queue.Queue[tuple[int, int, str]] = queue.Queue()
        self._ev_thread: threading.Thread | None = None
        self._last_ts = ""

    def start(self) -> bool:
        """启动订阅线程（sidecar 不可用时返回 False，由调用方回退 Python worker）。"""
        if self._thread is not None and self._thread.is_alive():
            return True
        try:
            self.analyzer.start()
        except (FileNotFoundError, RuntimeError, TimeoutError, OSError) as e:
            self._last_error = f"sidecar 不可用: {e}"
            log.warning("%s（该路回退 Python worker）", self._last_error)
            return False
        self._stop.clear()
        self._thread = threading.Thread(target=self._loop, name="sidecar-events", daemon=True)
        self._thread.start()
        self._ev_thread = threading.Thread(target=self._evidence_loop, name="sidecar-evidence",
                                           daemon=True)
        self._ev_thread.start()
        return True

    def stop(self) -> None:
        self._stop.set()
        t = self._thread
        if t is not None and t.is_alive():
            t.join(timeout=3.0)
        self._thread = None
        t2 = self._ev_thread
        if t2 is not None and t2.is_alive():
            t2.join(timeout=1.0)  # finalize 阻塞最长 ~15s，只等 1s 让进程退出不被拖住
        self._ev_thread = None

    def status(self) -> dict:
        with self._lock:
            return {"ingested": self._ingested, "last_error": self._last_error,
                    "last_ts": self._last_ts, "running": self._thread is not None
                    and self._thread.is_alive()}

    def _loop(self) -> None:
        from . import analyzer_pb2 as pb
        while not self._stop.is_set():
            try:
                stub = self.analyzer._stub
                if stub is None:
                    self._stop.wait(1.0)
                    continue
                stream = stub.Events(pb.EventsRequest(),
                                     metadata=(("x-ls-token", self.analyzer._token),))
                for cand in stream:
                    if self._stop.is_set():
                        return
                    self._ingest(cand)
            except Exception as e:  # noqa: BLE001  流断开自动重连，绝不让事件桥死掉
                self._last_error = f"事件流中断: {e}"
                log.warning("%s（1s 后重连）", self._last_error)
                self._stop.wait(1.0)

    def _dedup(self, cand) -> bool:
        """True=新事件；evidence_key+behavior 去重（流重连重放时保护事件表）。"""
        key = f"{cand.evidence_key}|{cand.behavior}|{cand.track_id}"
        with self._lock:
            if key in self._seen:
                return False
            self._seen.add(key)
            self._seen_order.append(key)
            if len(self._seen_order) > 4096:  # 有界，防长跑内存增长
                old = self._seen_order.pop(0)
                self._seen.discard(old)
        return True

    def _ingest(self, cand) -> None:
        """候选 → 事件行 → 快照 → 完整告警派发链。"""
        if not self._dedup(cand):
            return
        ts = cand.ts_iso or dt.datetime.now().isoformat(timespec="seconds")
        snapshot = None
        if cand.snapshot_jpeg:
            try:
                from .config import settings
                snap_dir = Path(settings.snapshot_dir)
                snap_dir.mkdir(parents=True, exist_ok=True)
                import uuid
                from datetime import datetime
                name = (f"ev_{cand.camera_id}_"
                        f"{datetime.now().strftime('%Y%m%d-%H%M%S-%f')[:-3]}_"
                        f"{uuid.uuid4().hex[:6]}.jpg")
                path = snap_dir / name
                path.write_bytes(cand.snapshot_jpeg)
                snapshot = str(path)
            except OSError as e:
                log.warning("sidecar 快照写入失败: %s", e)
        try:
            eid = self.store.add_event(cand.camera_id, cand.behavior, cand.level,
                                       cand.track_id, cand.detail, snapshot, ts=ts)
            row = self.store.event_by_id(eid)
            if row is None:
                return
            row.pop("snapshot", None)  # 绝对路径不外泄（与 Python worker 同口径）
            row.pop("video", None)
            row["has_snapshot"] = bool(snapshot)
            row["has_video"] = False
            self.dispatcher.dispatch(row)
            with self._lock:
                self._ingested += 1
                self._last_ts = ts
                self._last_error = ""
            print(f"[sidecar] event#{eid} cam-{cand.camera_id} {cand.behavior} L{cand.level}")
            # T-326：Rust 侧补齐事件前后片段视频——入队由存证线程处理
            #（与 Python worker「先入库后写视频」同口径；失败只影响存证不影响事件）
            self._ev_q.put((eid, cand.camera_id, cand.evidence_key))
        except Exception as e:  # noqa: BLE001  单事件失败不影响后续
            self._last_error = f"入库/派发失败: {e}"
            log.warning("%s", self._last_error)

    def _evidence_loop(self) -> None:
        """存证收尾线程：finalize（阻塞至 Rust 写盘完成）→ 回填 events.video。"""
        while not self._stop.is_set():
            try:
                eid, camera_id, key = self._ev_q.get(timeout=0.2)
            except queue.Empty:
                continue
            try:
                res = self.analyzer.finalize_evidence(eid, camera_id, key)
                if res and res.get("ok") and res.get("video_relpath"):
                    from .config import settings as _settings
                    self.store.set_event_video(
                        eid, str(Path(_settings.data_dir) / res["video_relpath"]))
            except Exception as e:  # noqa: BLE001  单事件失败不影响后续
                log.warning("存证收尾失败(ev=%s): %s", eid, e)
