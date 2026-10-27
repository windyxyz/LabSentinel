"""分析管线 worker：每路摄像头一个线程。

取流 → (限频)检测 → ByteTrack 跟踪 → 区域进出判定 → 规则/冷却 → 事件入库+告警+截图+视频存证。
"""
import os
import queue
import threading
import time
from collections import deque

import cv2
import numpy as np
import supervision as sv

from .alerts import dispatcher
from .config import settings
from .engine import get_engine, get_ppe_engine
from .face import FaceGate
from .rules import RuleEngine
from .store import Store
from .zones import ZoneState, boxes_overlap, parse_polygon, point_inside

# 视频存证：前/后各 2 秒；同一时间最多 3 个并发录制（防告警风暴拖垮 IO）
VIDEO_PRE_S = 2.0
VIDEO_POST_S = 2.0
MAX_CONCURRENT_RECORDINGS = 3


class _Recording:
    """一次事件录像：前 2 秒帧已从环形缓冲拷出，等主循环补齐后 2 秒。"""

    def __init__(self, eid: int, pre_frames: list):
        self.eid = eid
        self.frames = list(pre_frames)
        self.t0 = time.monotonic()
        self.done = threading.Event()


class CameraWorker(threading.Thread):
    def __init__(self, camera: dict, store: Store, rules: RuleEngine):
        super().__init__(daemon=True, name=f"cam-{camera['id']}")
        self.camera = camera
        self.store = store
        self.rules = rules
        self.stop_flag = threading.Event()
        self.latest_frame: np.ndarray | None = None
        self.frame_lock = threading.Lock()
        self.analysis_fps_now: float = 0.0
        self._byteTrack = sv.ByteTrack()  # M0 结论：必须 ByteTrack，IOU 跟踪 ID 会跳变
        self._zone_states: dict[int, ZoneState] = {}  # zone_id -> ZoneState
        self._ring: deque = deque(maxlen=1)  # 环形帧缓存，run() 里按实际帧率重建
        self._recs: list[_Recording] = []  # 进行中的录像任务
        self._rec_q: queue.Queue = queue.Queue()  # 已补齐待写盘的任务
        self._face = FaceGate()  # 人脸比对门控：默认关闭时 check() 恒返回 None

    def _init_ring(self, fps: float):
        self._ring = deque(maxlen=int(fps * VIDEO_PRE_S) + 10)

    # ---- 主循环 ----
    def run(self):
        cid = self.camera["id"]
        cap = cv2.VideoCapture(self.camera["source"])
        if not cap.isOpened():
            self.store.set_camera_status(cid, "error")
            print(f"[cam-{cid}] cannot open {self.camera['source']}")
            return
        self.store.set_camera_status(cid, "running")
        engine = get_engine()
        self.ppe = get_ppe_engine()  # 可能为 None（未配置实验服模型）
        interval = 1.0 / settings.analysis_fps
        file_fps = cap.get(cv2.CAP_PROP_FPS) or 30  # 文件源按原速播放（否则读帧瞬间耗尽、分析只抽到几帧）
        self._rec_fps = file_fps
        self._init_ring(file_fps)
        recorder = threading.Thread(target=self._recorder_loop, daemon=True, name=f"cam-{cid}-rec")
        recorder.start()
        next_t = 0.0
        frames = 0
        fps_t0 = time.monotonic()
        try:
            while not self.stop_flag.is_set():
                ok, frame = cap.read()
                if not ok:  # 文件播完 / 流断开
                    if self.camera["type"] == "file":
                        if settings.file_loop:  # 压测/演示：文件源循环播放
                            cap.set(cv2.CAP_PROP_POS_FRAMES, 0)
                            continue
                        break
                    time.sleep(1.0)
                    cap.release()
                    cap = cv2.VideoCapture(self.camera["source"])
                    if not cap.isOpened():
                        self.store.set_camera_status(cid, "error")
                        time.sleep(5)
                    continue
                if self.camera["type"] == "file":
                    time.sleep(1.0 / file_fps)
                frame = cv2.resize(frame, (1280, 720))
                with self.frame_lock:
                    self.latest_frame = frame
                self._feed_recordings(frame)
                now = time.monotonic()
                if now >= next_t:
                    try:
                        self._analyze(engine, frame)
                    except Exception as e:  # noqa: BLE001
                        # 自愈：单帧推理异常（如设备瞬时故障）只丢这一帧，不杀死 worker 线程。
                        # 2026-10-26 多路压测中发现 DML 并发崩溃会让 worker 静默死亡、摄像头永久 stopped，
                        # 根因已在 engine.py 串行化修复，这里是兜底。
                        print(f"[cam-{cid}] analyze error (frame skipped): {e}")
                    next_t = now + interval
                frames += 1
                if now - fps_t0 >= 5:
                    self.analysis_fps_now = frames / (now - fps_t0)
                    frames, fps_t0 = 0, now
        finally:
            self.stop_flag.set()  # 先停 recorder 线程，避免与其争抢队列
            recorder.join(timeout=2.0)
            for rec in self._recs:  # 未补齐的录像：帧数不足也落盘（有比没有强）
                if not rec.done.is_set():
                    rec.done.set()
                    self._rec_q.put(rec)
            while True:  # 同步清空队列，保证视频全部写完再退出
                try:
                    self._write_video(self._rec_q.get_nowait())
                except queue.Empty:
                    break
            cap.release()
            self.store.set_camera_status(cid, "stopped")

    # ---- 视频存证 ----
    def _feed_recordings(self, frame):
        """主循环每帧调用：喂环形缓存与进行中的录像任务。"""
        self._ring.append(frame)
        for rec in self._recs:
            if rec.done.is_set():
                continue
            rec.frames.append(frame)
            if time.monotonic() - rec.t0 >= VIDEO_PRE_S + VIDEO_POST_S:
                rec.done.set()
                self._rec_q.put(rec)
        self._recs = [r for r in self._recs if not r.done.is_set()]

    def _open_writer(self, path: str, fps: float):
        """优先 H.264（浏览器可直接播），失败退回 mp4v（ffplay 可播）。"""
        for fourcc, api in (("avc1", cv2.CAP_MSMF), ("avc1", 0), ("mp4v", 0)):
            try:
                vw = (cv2.VideoWriter(path, api, cv2.VideoWriter_fourcc(*fourcc), fps, (1280, 720))
                      if api else cv2.VideoWriter(path, cv2.VideoWriter_fourcc(*fourcc), fps, (1280, 720)))
                if vw.isOpened():
                    return vw
                vw.release()
            except Exception:  # noqa: BLE001
                continue
        return None

    def _recorder_loop(self):
        """后台写盘线程：逐个把补齐帧数的录像任务写成 MP4。"""
        while not self.stop_flag.is_set():
            try:
                rec = self._rec_q.get(timeout=0.2)
            except queue.Empty:
                continue
            self._write_video(rec)

    def _write_video(self, rec: _Recording, path: str | None = None, fps: float | None = None):
        path = path or str(settings.video_dir / f"ev_{rec.eid}.mp4")
        if not rec.frames:
            return
        vw = self._open_writer(path, fps or getattr(self, "_rec_fps", 30.0))
        if vw is None:
            print(f"[cam-{self.camera['id']}] ev#{rec.eid} 视频编码器不可用，跳过存证")
            return
        for f in rec.frames:
            vw.write(f)
        vw.release()
        self.store.set_event_video(rec.eid, path)

    # ---- 单帧分析 ----
    def _analyze(self, engine, frame):
        dets = engine.detect_persons(frame)
        if not dets:
            return
        # supervision Detections + ByteTrack
        xyxy = np.array([d.xyxy for d in dets], dtype=np.float32)
        conf = np.array([d.conf for d in dets], dtype=np.float32)
        cls = np.zeros(len(dets), dtype=int)
        sdets = sv.Detections(xyxy=xyxy, confidence=conf, class_id=cls)
        sdets = self._byteTrack.update_with_detections(sdets)

        zones = self.store.zones(self.camera["id"])
        zone_polys = {z["id"]: parse_polygon(z["polygon"]) for z in zones}
        for z in zones:
            self._zone_states.setdefault(z["id"], ZoneState())

        occupancy = 0
        for i in range(len(sdets)):
            tid = int(sdets.tracker_id[i])
            x1, y1, x2, y2 = sdets.xyxy[i].tolist()
            anchor = (int((x1 + x2) / 2), int(y2))
            in_any = False
            for zid, poly in zone_polys.items():
                if not point_inside(poly, anchor):
                    continue
                in_any = True
                occupancy += 1
                change = self._zone_states[zid].update(tid, True)
                if change:
                    entered, exited = change
                    if entered:
                        zname = self._zone_name(zid)
                        ppe = self._ppe_status(frame, (x1, y1, x2, y2))
                        # 人脸识别（默认关闭；每 track 冷却期内不重复调 API）
                        face = self._face.check(self.camera["id"], tid, frame, (x1, y1, x2, y2))
                        face_txt = f"，人员：{face[6:]}" if face and face.startswith("known:") else ""
                        if ppe == "wearing":
                            self._fire_event(zid, "enter", tid, f"进入安全区「{zname}」（已穿实验服{face_txt}）")
                        elif ppe == "no":
                            self._fire_event(zid, "enter", tid, f"进入安全区「{zname}」（未穿实验服{face_txt}）")
                            self._fire_event(zid, "enter_no_ppe", tid, f"进入安全区「{zname}」未穿实验服{face_txt}")
                        else:
                            note = "实验服检测未启用" if self.ppe is None else "实验服状态不确定"
                            self._fire_event(zid, "enter", tid, f"进入安全区「{zname}」{face_txt}")
                            self._fire_event(zid, "enter_no_ppe", tid,
                                             f"进入安全区「{zname}」（{note}）", quiet=True)
                        if face == "unknown":
                            self._fire_stranger(tid, zname)
            if not in_any:
                for zid in zone_polys:
                    change = self._zone_states[zid].update(tid, False)
                    if change and change[1]:
                        self._fire_event(zid, "exit", tid, f"离开安全区「{self._zone_name(zid)}」")

        # 在场人数超限（规则级冷却统一由 RuleEngine 管，track 记 0）
        for rule in self.rules.occupancy_rules(self.camera["id"]):
            if rule["threshold"] and occupancy > rule["threshold"]:
                self._fire_event(None, "occupancy_over", 0,
                                 f"在场人数 {occupancy} 超过阈值 {rule['threshold']}", level=rule["level"])

    def _zone_name(self, zid):
        for z in self.store.zones(self.camera["id"]):
            if z["id"] == zid:
                return z["name"]
        return str(zid)

    def _ppe_status(self, frame, person_box) -> str:
        """证据分级判定：'no'（有显式违规证据） | 'wearing'（有穿戴证据） | 'unknown'（证据不足）。

        违规告警只认 No Lab Coat 类的显式检测（AP 0.81）；穿戴证据用低阈值
        （远距离时 Lab Coat 置信度会降到 0.2 附近）；两者皆无时返回 unknown，
        绝不把"证据不足"当"未穿"——防对合规人员误报告警。
        """
        if self.ppe is None:
            return "unknown"
        for det in self.ppe.detect_class(frame, settings.ppe_violation_class, conf=settings.ppe_conf):
            if boxes_overlap(det.xyxy, person_box):
                return "no"
        for name in settings.ppe_class_names.split(","):
            for det in self.ppe.detect_class(frame, name.strip(), conf=settings.ppe_wearing_conf):
                if boxes_overlap(det.xyxy, person_box):
                    return "wearing"
        return "unknown"

    def _save_snapshot(self) -> str | None:
        with self.frame_lock:
            if self.latest_frame is None:
                return None
            path = settings.snapshot_dir / f"ev_{int(time.time()*1000)}.jpg"
            ok, buf = cv2.imencode(".jpg", self.latest_frame)
            if not ok:
                return None
            # Windows 下 cv2.imwrite 对含中文的路径静默返回 False（本项目路径即含「监控识别」），
            # 统一走 imencode + Python 写盘，保证截图证据不丢。
            path.write_bytes(buf.tobytes())
            return str(path)

    def _dispatch_row(self, eid, behavior, level, track_id, detail, snapshot):
        row = self.store.events(limit=1)[0]
        row["has_snapshot"] = bool(snapshot)
        row["has_video"] = False  # 视频异步写盘，事件推送时还未生成
        dispatcher.dispatch(row)

    def _fire_event(self, zone_id, behavior, track_id, detail, level=2, quiet=False):
        rule = self.rules.match(self.camera["id"], zone_id, behavior, track_id, level)
        if rule is None:
            return
        level = rule["level"]
        snapshot = self._save_snapshot()
        eid = self.store.add_event(self.camera["id"], behavior, level, track_id, detail, snapshot)
        self._dispatch_row(eid, behavior, level, track_id, detail, snapshot)
        # 视频存证：环形缓冲拷出前 2 秒，后 2 秒由主循环补齐；超 3 路并发退化为仅截图
        active = sum(1 for r in self._recs if not r.done.is_set())
        if self._ring and active < MAX_CONCURRENT_RECORDINGS:
            self._recs.append(_Recording(eid, list(self._ring)))
        if not quiet:
            print(f"[cam-{self.camera['id']}] event#{eid} {behavior} L{level} {detail}")

    def _fire_stranger(self, track_id, zname):
        """陌生人事件（未注册人员进入）。不走规则匹配：人脸门控冷却已限频，级别固定 L2。"""
        detail = f"未注册人员进入安全区「{zname}」"
        snapshot = self._save_snapshot()
        eid = self.store.add_event(self.camera["id"], "stranger", 2, track_id, detail, snapshot)
        self._dispatch_row(eid, "stranger", 2, track_id, detail, snapshot)
        active = sum(1 for r in self._recs if not r.done.is_set())
        if self._ring and active < MAX_CONCURRENT_RECORDINGS:
            self._recs.append(_Recording(eid, list(self._ring)))
        print(f"[cam-{self.camera['id']}] event#{eid} stranger L2 {detail}")

    # ---- 控制 ----
    def stop(self):
        self.stop_flag.set()


class CameraManager:
    def __init__(self, store: Store):
        self.store = store
        self.rules = RuleEngine(store)
        self.workers: dict[int, CameraWorker] = {}

    def start_camera(self, camera: dict):
        cid = camera["id"]
        if cid in self.workers and self.workers[cid].is_alive():
            return
        w = CameraWorker(camera, self.store, self.rules)
        self.workers[cid] = w
        w.start()

    def stop_camera(self, cid):
        w = self.workers.pop(cid, None)
        if w:
            w.stop()

    def start_all(self):
        for cam in self.store.cameras():
            if cam["type"] in ("rtsp", "file", "usb"):
                self.start_camera(cam)

    def stop_all(self):
        for cid in list(self.workers):
            self.stop_camera(cid)

    def latest_frame(self, cid):
        w = self.workers.get(cid)
        if w is None:
            return None
        with w.frame_lock:
            return w.latest_frame

    def status(self):
        return {
            "cameras": [
                {"id": cid, "fps": round(w.analysis_fps_now, 1),
                 "status": "running" if w.is_alive() else "stopped"}
                for cid, w in self.workers.items()
            ]
        }
