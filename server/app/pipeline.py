"""分析管线 worker：每路摄像头一个线程。

取流 → (限频)检测 → ByteTrack 跟踪 → 区域进出判定 → 规则/冷却 → 事件入库+告警+截图+视频存证。
"""
import os
import queue
import re
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

# 任务65：rtsp/usb 打开失败/断流的有限重试（file 源坏路径直接退出，不反复刷）
MAX_OPEN_RETRIES = 5
RETRY_INTERVAL_S = 3.0

_CRED_RE = re.compile(r"(?i)(\w[\w+.-]*://)([^/@\s:]+):([^@\s]+)@")


def redact_creds(msg: str, limit: int = 200) -> str:
    """错误文案脱敏：URL 凭证 user:pass@ → ***:***@，截断 200 字。"""
    return _CRED_RE.sub(r"\1***:***@", str(msg))[:limit]


class _Recording:
    """一次事件录像：前 2 秒帧已从环形缓冲拷出，等主循环补齐后 2 秒。"""

    def __init__(self, eid: int, pre_frames: list):
        self.eid = eid
        self.frames = list(pre_frames)
        self.t0 = time.monotonic()
        self.done = threading.Event()


def capture_source(camera: dict):
    """OpenCV 打开源：USB 允许 "0"/"1" 这类设备号（字符串会被当文件名），其余原样。"""
    src = camera.get("source")
    if camera.get("type") == "usb":
        s = str(src).strip()
        if s.isdigit():
            return int(s)
    return src


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
        # 任务65 运行指标（简单赋值；读取允许轻微竞态）
        self.last_frame_ts: float | None = None
        self.last_error: str | None = None
        self.last_error_ts: float | None = None
        self.reconnect_count: int = 0
        self.opened_at: float | None = None
        self.frames_total: int = 0
        self._exit_status: str = "stopped"  # finally 写库用：重试耗尽/打开失败保持 error

    # ---- 任务65 指标辅助 ----
    def _set_error(self, msg: str) -> None:
        self.last_error = redact_creds(msg)
        self.last_error_ts = time.time()

    def _clear_error(self) -> None:
        self.last_error = None
        self.last_error_ts = None

    def _init_ring(self, fps: float):
        self._ring = deque(maxlen=int(fps * VIDEO_PRE_S) + 10)

    # ---- 主循环 ----
    def run(self):
        cid = self.camera["id"]
        cap = cv2.VideoCapture(capture_source(self.camera))
        retries = 0
        while not cap.isOpened():
            self._set_error(f"打开失败: {self.camera.get('source', '')}")
            self.store.set_camera_status(cid, "error")
            if self.camera["type"] == "file" or retries >= MAX_OPEN_RETRIES:
                # file 坏路径直接退出；rtsp/usb 重试 5 次（间隔 3s）后放弃
                print(f"[cam-{cid}] cannot open {self.camera['source']}"
                      + (f"（重试 {retries} 次后放弃）" if retries else ""))
                self._exit_status = "error"
                return
            retries += 1
            self.reconnect_count += 1
            if self.stop_flag.wait(RETRY_INTERVAL_S):
                return
            cap = cv2.VideoCapture(capture_source(self.camera))
        self._clear_error()
        self.opened_at = time.time()
        self.store.set_camera_status(cid, "running")
        engine = get_engine()
        self.ppe = get_ppe_engine()  # 可能为 None（未配置实验服模型）
        interval = 1.0 / settings.analysis_fps
        # USB 摄像头常返回 fps=-1；为录像 writer/环形缓冲兜底到 30
        # 2026-10-30 审阅：CAP_PROP_FPS 可能返回 NaN（`or 30` 挡不住），NaN 会让 int() 抛错
        # 且此时还没进 try——状态卡 running、cap 泄漏，故用 isfinite 严格护栏
        import math
        file_fps = cap.get(cv2.CAP_PROP_FPS)
        if not isinstance(file_fps, (int, float)) or not math.isfinite(file_fps) \
                or file_fps <= 0 or file_fps > 240:
            file_fps = 30.0
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
                    # rtsp/usb 断流：有限重连（任务65）
                    self.reconnect_count += 1
                    self._set_error("读帧中断，重连中")
                    cap.release()
                    reopened = False
                    for attempt in range(MAX_OPEN_RETRIES):
                        if self.stop_flag.wait(RETRY_INTERVAL_S):
                            return
                        cap = cv2.VideoCapture(capture_source(self.camera))
                        if cap.isOpened():
                            reopened = True
                            break
                        self._set_error(f"重连失败（{attempt + 1}/{MAX_OPEN_RETRIES}）")
                    if not reopened:
                        self.store.set_camera_status(cid, "error")
                        self._exit_status = "error"
                        print(f"[cam-{cid}] 重连 {MAX_OPEN_RETRIES} 次失败，退出")
                        return
                    self._clear_error()
                    self.opened_at = time.time()
                    self.store.set_camera_status(cid, "running")
                    continue
                self.last_frame_ts = time.time()
                self.frames_total += 1
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
                        self._set_error(f"analyze: {e}")  # 任务65：只记指标，不改 running 状态
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
            self.store.set_camera_status(cid, self._exit_status)

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
        zones = self.store.zones(self.camera["id"])
        zone_polys = {z["id"]: parse_polygon(z["polygon"]) for z in zones}
        for z in zones:
            self._zone_states.setdefault(z["id"], ZoneState())

        if not dets:
            # 本帧无检出（2026-10-30 审阅修复）：所有在场 track 视为已离开。
            # 105 回归补丁 P0-1：必须调 update(tid, False) 清状态——原补丁只发 exit 不更新，
            # 空帧重复发 exit（靠冷却压制）、人再进入时 was=True 永不再触发 enter（事件丢失）。
            for zid, st in self._zone_states.items():
                for tid in list(st.inside_ids):
                    change = st.update(tid, False)
                    if change and change[1]:
                        self._fire_event(zid, "exit", tid, f"离开安全区「{self._zone_name(zid)}」")
            return

        # supervision Detections + ByteTrack
        xyxy = np.array([d.xyxy for d in dets], dtype=np.float32)
        conf = np.array([d.conf for d in dets], dtype=np.float32)
        cls = np.zeros(len(dets), dtype=int)
        sdets = sv.Detections(xyxy=xyxy, confidence=conf, class_id=cls)
        sdets = self._byteTrack.update_with_detections(sdets)
        active_tids = {int(sdets.tracker_id[i]) for i in range(len(sdets))}

        # 105 回归补丁 P0-1（部分消失）：画面里还有别人、但某个 track 已从检出中消失——
        # 该 track 在各区的 inside 状态永真，exit 永不触发。对 inside_ids - active_tids 补离场。
        for zid, st in self._zone_states.items():
            for tid in set(st.inside_ids) - active_tids:
                change = st.update(tid, False)
                if change and change[1]:
                    self._fire_event(zid, "exit", tid, f"离开安全区「{self._zone_name(zid)}」")

        occupancy = 0
        for i in range(len(sdets)):
            tid = int(sdets.tracker_id[i])
            x1, y1, x2, y2 = sdets.xyxy[i].tolist()
            anchor = (int((x1 + x2) / 2), int(y2))
            inside_zones = {zid for zid, poly in zone_polys.items() if point_inside(poly, anchor)}
            occupancy += len(inside_zones)
            # 每帧对每个 zone 更新 inside 状态（2026-10-30 审阅修复：原实现只在"不在任何区域"
            # 时才补 exit——A→B 直移会漏掉 A 的 exit，且离开重叠区时产生滞后重复 exit）
            for zid in zone_polys:
                change = self._zone_states[zid].update(tid, zid in inside_zones)
                if not change:
                    continue
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
                if exited:
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
        # 2026-10-30 审阅修复：原实现用 events(limit=1) 取"最新一行当刚插入的事件"——
        # 多摄像头并发插入时 A 会错拿 B 的事件行（A 告警静默丢失、B 收到两条），必须按 id 单查。
        row = self.store.event_by_id(eid)
        if row is None:  # 行刚被保留清理删掉等极端情况：跳过推送，不阻塞分析线程
            print(f"[cam-{self.camera['id']}] event#{eid} 行缺失，跳过告警派发")
            return
        row.pop("snapshot", None)  # 服务器绝对路径不外泄（WS 广播/推送渠道只给布尔）
        row.pop("video", None)
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
        self._mgmt_lock = threading.Lock()  # 2026-10-30 审阅：start/stop/status 与 API 线程互斥

    def start_camera(self, camera: dict):
        cid = camera["id"]
        with self._mgmt_lock:
            old = self.workers.get(cid)
            if old is not None and old.is_alive():
                return
            if old is not None:
                # 2026-10-30 审阅：旧线程刚 stop 尚未退出时，USB/RTSP 独占设备还没释放——
                # 只在"重启同一路"这个真正需要句柄释放的场景同步等待（stop 本身保持异步不拖关停）
                old.join(timeout=3.0)
            w = CameraWorker(camera, self.store, self.rules)
            self.workers[cid] = w
            w.start()

    def stop_camera(self, cid):
        with self._mgmt_lock:
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
        now = time.time()
        cams = []
        with self._mgmt_lock:  # 2026-10-30 审阅：API 线程遍历时 stop_camera 可能 pop，先快照
            items = list(self.workers.items())
        for cid, w in items:
            lfw = w.last_frame_ts
            cams.append({
                "id": cid, "fps": round(w.analysis_fps_now, 1),
                "status": "running" if w.is_alive() else "stopped",
                "last_frame_ts": lfw,
                "frame_age_s": round(now - lfw, 1) if lfw else None,
                "last_error": w.last_error,
                "last_error_ts": w.last_error_ts,
                "reconnect_count": w.reconnect_count,
                "opened_at": w.opened_at,
                "frames_total": w.frames_total,
            })
        return {"cameras": cams}
