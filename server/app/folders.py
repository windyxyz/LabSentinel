"""图片文件夹接入（任务121）：外部软件持续截图 → 自动识别关键图片。

生产场景：实验室某软件把画面/结果自动截图为图片文件写入一个目录；本 worker 轮询目录，
对每张图片跑与视频源相同的人体/实验服模型 + 安全区/规则，落库并汇报：

- 每张图片一行 `folder_images`（人数/在区/实验服证据/命中区域/违规/证据副本）；
- `key=1`：图片中出现人员（需要人工关注的候选帧）；
- `violation=1`：出现显式违规证据（未穿实验服/人数超限），会按规则/冷却产生事件与告警；
- 原始截图只读，不移动/不删除；派生的证据副本才受保留策略管理。

与视频源的区别：图片之间没有时间连续性，不做 ByteTrack/进出场状态机，也不产 exit 事件；
人员按"每张图独立判定"处理，规则冷却以 track_id=0 生效（防同一违规连续截图刷屏告警）。
"""
from __future__ import annotations

import os
import threading
import time
import uuid
from datetime import datetime
from pathlib import Path

import cv2

from .alerts import dispatcher
from .config import settings
from .engine import get_engine, get_ppe_engine
from .rules import RuleEngine
from .store import Store
from .vision import normalize_frame, ppe_boxes, ppe_status_from_boxes, read_image

THUMB_MAX_W = 480
_CANDIDATE_CAP = 50000  # 单轮目录枚举上限（防误指超大目录吃内存；超出只处理最新，日志告警）
_PENDING_CAP = 50000  # 待处理队列上限（积压跨轮保留，保证能连续消化）


def image_exts() -> tuple[str, ...]:
    """当前允许的图片扩展名（小写，含点）。"""
    return tuple(e.strip().lower() for e in settings.folder_exts.split(",") if e.strip())


def thumbnail_path(iid: int) -> Path:
    return settings.data_dir / "thumbs" / f"img_{iid}.jpg"


_THUMB_LOCK = threading.Lock()


def thumbnail_bytes(src: str, iid: int, max_w: int = THUMB_MAX_W) -> bytes | None:
    """缩略图（列表页用）：磁盘缓存 data/thumbs/img_<id>.jpg；源文件更新则重生成。"""
    src_p = Path(src)
    dest = thumbnail_path(iid)
    try:
        if dest.is_file() and dest.stat().st_mtime >= src_p.stat().st_mtime:
            return dest.read_bytes()
    except OSError:
        pass
    with _THUMB_LOCK:
        try:  # 双检：并发首屏只生成一次
            if dest.is_file() and dest.stat().st_mtime >= src_p.stat().st_mtime:
                return dest.read_bytes()
        except OSError:
            pass
        img = read_image(src_p)
        if img is None:
            return None
        h, w = img.shape[:2]
        if w > max_w > 0:
            img = cv2.resize(img, (max_w, max(1, round(h * max_w / w))),
                             interpolation=cv2.INTER_AREA)
        ok, buf = cv2.imencode(".jpg", img, [int(cv2.IMWRITE_JPEG_QUALITY), 80])
        if not ok:
            return None
        try:
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(buf.tobytes())
        except OSError:
            pass  # 缓存写失败不影响本次返回
        return buf.tobytes()


class FolderWorker(threading.Thread):
    """一个图片文件夹 = 一路"摄像头"（type=folder），接口与 CameraWorker 对齐供 Manager 复用。"""

    kind = "folder"

    def __init__(self, camera: dict, store: Store, rules: RuleEngine):
        super().__init__(daemon=True, name=f"folder-{camera['id']}")
        self.camera = camera
        self.store = store
        self.rules = rules
        self.stop_flag = threading.Event()
        self._wake = threading.Event()
        # 与 CameraWorker/Manager 对齐的指标字段
        self.latest_frame = None
        self.frame_lock = threading.Lock()
        self.analysis_fps_now = 0.0
        self.last_frame_ts: float | None = None
        self.last_error: str | None = None
        self.last_error_ts: float | None = None
        self.reconnect_count = 0
        self.opened_at: float | None = None
        self.frames_total = 0
        self.ppe = None
        self._status: str | None = None
        self._exit_status = "stopped"
        # 文件夹专有指标
        self.processed_total = 0
        self.key_total = 0
        self.violation_total = 0
        self.error_total = 0
        self.backlog = 0
        self.last_scan_ts: float | None = None
        self.current_file: str | None = None
        self._checked: dict[str, tuple[int, int]] = {}  # path -> (size, mtime_ns) 本次会话已核
        # 目录级增量扫描：目录 mtime 未变则不重复枚举其文件（大目录/长跑省 IO）
        self._dir_sig: dict[str, int] = {}
        self._dir_children: dict[str, list[str]] = {}
        self._last_full_scan = 0.0
        self._pending: list[tuple[Path, os.stat_result]] = []  # 已发现未处理（跨轮保留）

    # ---- 状态/错误 ----
    def _set_status(self, status: str) -> None:
        if self._status == status:
            return
        self._status = status
        try:
            self.store.set_camera_status(self.camera["id"], status)
        except Exception as e:  # noqa: BLE001  状态写库失败不应杀死采集线程，但必须留痕
            print(f"[folder-{self.camera['id']}] status write failed: {e}")

    def _set_error(self, msg: str) -> None:
        self.last_error = str(msg)[:300]
        self.last_error_ts = time.time()

    def _clear_error(self) -> None:
        self.last_error = None
        self.last_error_ts = None

    def _set_camera_error_status(self) -> None:
        self._set_status("error")

    # ---- 主循环 ----
    def run(self):
        cid = self.camera["id"]
        root = Path(str(self.camera.get("source") or "")).expanduser()
        engine = None
        interval = max(0.5, float(settings.folder_poll_s))
        while not self.stop_flag.is_set():
            if not root.is_dir():
                self._set_error(f"文件夹不存在或不可访问: {root}")
                self._set_camera_error_status()
                if self.stop_flag.wait(max(5.0, interval * 2)):
                    break
                continue
            if engine is None:
                try:
                    engine = get_engine()
                    self.ppe = get_ppe_engine()
                except Exception as e:  # noqa: BLE001  模型加载失败可重试（比如临时显存不足）
                    self._set_error(f"模型加载失败: {e}")
                    self._set_camera_error_status()
                    print(f"[folder-{cid}] engine init failed: {e}")
                    if self.stop_flag.wait(10.0):
                        break
                    continue
            self._clear_error()
            if self.opened_at is None:
                self.opened_at = time.time()
            self._set_status("running")
            try:
                self._scan_once(engine, root)
            except Exception as e:  # noqa: BLE001  单轮扫描异常只跳过本轮
                self._set_error(f"扫描失败: {e}")
                print(f"[folder-{cid}] scan error: {e}")
            self._wake.wait(interval)
            self._wake.clear()
        self._set_status(self._exit_status)

    def _scan_once(self, engine, root: Path) -> int:
        """枚举/合并待处理队列 → 限量识别。返回本轮识别张数（测试直接调用）。

        待处理队列跨轮保留：目录 mtime 不变时增量扫描发现不了旧积压，但队列仍会继续消化，
        不会出现"每 5 分钟全量一次才处理 20 张"的饥饿。
        """
        self.last_scan_ts = time.time()
        candidates = self._candidates(root)
        merged = {str(p): (p, st) for p, st in self._pending}
        for p, st in candidates:
            merged[str(p)] = (p, st)  # 同路径更新为最新 stat
        self._pending = sorted(merged.values(),
                               key=lambda item: (-item[1].st_mtime, str(item[0])))[:_PENDING_CAP]
        self.backlog = len(self._pending)
        interval = 1.0 / max(0.1, float(settings.analysis_fps))
        limit = max(1, int(settings.folder_max_per_tick))
        batch, rest = self._pending[:limit], self._pending[limit:]
        keep: list[tuple[Path, os.stat_result]] = []
        done = 0
        t_scan = time.monotonic()
        for idx, (path, _st) in enumerate(batch):
            if self.stop_flag.is_set():
                keep.extend(batch[idx:])  # 未处理的留到下一轮
                break
            if self._already_done(path, _st):
                continue
            t0 = time.monotonic()
            try:
                processed = self._process_path(engine, path)
            except Exception as e:  # noqa: BLE001  单张失败不影响后续；下轮重试
                self._set_error(f"识别失败 {path.name}: {e}")
                print(f"[folder-{self.camera['id']}] {path.name} analyze error: {e}")
                processed = False
            if processed:
                done += 1
            else:
                keep.append((path, _st))  # 文件暂不稳定/读取瞬间消失：保留重试
            dt = time.monotonic() - t0
            if dt < interval:
                self.stop_flag.wait(interval - dt)
        self._pending = keep + rest
        self.backlog = len(self._pending)
        elapsed = time.monotonic() - t_scan
        if elapsed > 0:
            self.analysis_fps_now = round(done / elapsed, 2)
        return done

    def _candidates(self, root: Path) -> list[tuple[Path, os.stat_result]]:
        """目录扫描：扩展名/大小/静置时间过滤，按文件修改时间降序（新图优先，保证告警时效）。

        递归模式做目录级增量：目录 mtime 未变（没有新增/删除/改名）时跳过其文件枚举；
        每 LS_FOLDER_FULL_RESCAN_S 秒强制全量一次，补抓"原地覆盖但不改目录时间"的文件。
        单目录超过 5 万张时只处理最新部分并在状态里告警（建议归档或按日期分子目录）。
        """
        exts = image_exts()
        max_bytes = max(1, int(settings.folder_max_file_mb)) * 1024 * 1024
        stable_s = max(0.0, float(settings.folder_stable_s))
        now = time.time()
        internal = os.path.normcase(str(settings.data_dir.resolve()))
        full_s = max(0, int(settings.folder_full_rescan_s))
        if now - self._last_full_scan >= full_s:
            self._dir_sig.clear()
            self._dir_children.clear()
            self._last_full_scan = now
        out: list[tuple[Path, os.stat_result]] = []
        try:
            if settings.folder_recursive:
                dirs = self._iter_dirs(str(root), internal)
            else:
                with os.scandir(root) as it:
                    names = [e.name for e in it if e.is_file()]
                dirs = iter([(str(root), names)])
            for dirpath, filenames in dirs:
                for name in filenames:
                    if Path(name).suffix.lower() not in exts:
                        continue
                    p = Path(dirpath) / name
                    try:
                        st = p.stat()
                    except OSError:
                        self._dir_sig[dirpath] = -1  # 文件被占用/瞬移：强制下轮重列该目录
                        continue  # 枚举与读取之间被移走/占用
                    if st.st_size <= 0:
                        self._dir_sig[dirpath] = -1  # 可能刚创建还没写完：下轮再看
                        continue
                    if st.st_size > max_bytes:
                        continue
                    age = now - st.st_mtime
                    if age < stable_s:  # 刚写入未静置：下轮重列该目录
                        self._dir_sig[dirpath] = -1
                        continue
                    if age < -300:  # 时钟异常的未来文件：跳过
                        continue
                    out.append((p, st))
        except OSError as e:
            self._set_error(f"目录读取失败: {e}")
            return []
        out.sort(key=lambda item: (-item[1].st_mtime, str(item[0])))  # 新图优先（告警时效）
        if len(out) > _CANDIDATE_CAP:
            self._set_error(f"目录图片数超过 {_CANDIDATE_CAP}，仅处理最新部分；"
                            "建议按日期分子目录或归档旧图")
            out = out[:_CANDIDATE_CAP]
        return out

    def _iter_dirs(self, root: str, internal: str):
        """递归枚举"有变化"的目录；返回 (dirpath, filenames)。内部数据目录一律跳过。"""
        stack = [root]
        seen: set[str] = set()
        while stack:
            d = stack.pop()
            key = os.path.normcase(os.path.abspath(d))
            if key in seen:
                continue
            seen.add(key)
            if key == internal or key.startswith(internal + os.sep):
                continue  # 防把数据目录指成输入目录造成"证据图→再识别"的自激循环
            try:
                sig = os.stat(d).st_mtime_ns
            except OSError:
                self._dir_sig.pop(d, None)
                self._dir_children.pop(d, None)
                continue
            children = self._dir_children.get(d)
            if self._dir_sig.get(d) == sig and children is not None:
                stack.extend(children)  # 本目录无变化：只继续检查子目录
                continue
            files: list[str] = []
            new_children: list[str] = []
            try:
                with os.scandir(d) as it:
                    for e in it:
                        try:
                            if e.is_dir(follow_symlinks=False):
                                if not e.name.startswith("."):
                                    new_children.append(e.path)
                            elif e.is_file(follow_symlinks=False):
                                files.append(e.name)
                        except OSError:
                            continue
            except OSError:
                continue
            self._dir_sig[d] = sig
            self._dir_children[d] = sorted(new_children)
            yield d, files
            stack.extend(new_children)

    @staticmethod
    def _sig(st: os.stat_result) -> tuple[int, int]:
        return int(st.st_size), int(st.st_mtime_ns)

    def _remember(self, path: str, sig: tuple[int, int]) -> None:
        self._checked[path] = sig
        if len(self._checked) > 50000:  # 有界内存：挤掉最旧 10k（而非清空，减少下一轮 DB 核对量）
            for _ in range(10000):
                k = next(iter(self._checked), None)
                if k is None:
                    break
                self._checked.pop(k, None)

    def _already_done(self, path: Path, st: os.stat_result) -> bool:
        key = str(path)
        sig = self._sig(st)
        if self._checked.get(key) == sig:
            return True
        try:
            done = self.store.folder_image_done(
                self.camera["id"], key, sig[1], sig[0])
        except Exception as e:  # noqa: BLE001  去重查询失败时宁可重试识别，也不漏图
            self._set_error(f"去重查询失败: {e}")
            return False
        if done:
            self._remember(key, sig)
        return done

    # ---- 单张图片 ----
    def _process_path(self, engine, path: Path) -> bool:
        """识别单张；返回 False 表示暂不稳定/需保留重试，True 表示已处理或可丢弃。"""
        cid = self.camera["id"]
        try:
            st = path.stat()  # 以处理时最新 stat 为准（队列里的可能已过期/被重写）
        except OSError:
            return True  # 文件已消失：丢弃该项
        if st.st_size <= 0:
            return True  # 空文件：丢弃，等外部软件写完产生新的目录 mtime/全量扫描再发现
        if time.time() - st.st_mtime < max(0.0, float(settings.folder_stable_s)):
            return False  # 仍在写入：保留到下一轮
        key = str(path)
        self.current_file = key
        img = read_image(path)
        ts = datetime.fromtimestamp(st.st_mtime).isoformat(timespec="seconds")
        processed_ts = datetime.now().isoformat(timespec="seconds")
        if img is None:
            iid = self.store.add_folder_image({
                "camera_id": cid, "path": key, "name": path.name,
                "mtime_ns": int(st.st_mtime_ns), "size": int(st.st_size),
                "ts": ts, "processed_ts": processed_ts,
                "error": "无法解码（损坏或非图片文件）"})
            self._remember(key, self._sig(st))
            self.frames_total += 1
            self.error_total += 1
            self._set_error(f"图片解码失败: {path.name}")
            print(f"[folder-{cid}] decode failed: {path.name} (image#{iid})")
            return True

        h, w = img.shape[:2]
        frame = normalize_frame(img)
        with self.frame_lock:
            self.latest_frame = frame
        result = self._analyze(engine, frame, path.name)
        snapshot = self._save_evidence(img, path) if result["violation"] else None
        iid = self.store.add_folder_image({
            "camera_id": cid, "path": key, "name": path.name,
            "mtime_ns": int(st.st_mtime_ns), "size": int(st.st_size),
            "ts": ts, "processed_ts": processed_ts, "width": w, "height": h,
            "persons": result["persons"], "in_zone": result["in_zone"],
            "ppe_no": result["ppe_no"], "ppe_wearing": result["ppe_wearing"],
            "zones": result["zones"], "violations": result["violations"],
            "key": result["key"], "violation": result["violation"], "snapshot": snapshot})
        first_eid = None
        for ev in result["events"]:
            eid = self._fire_event(ev, snapshot)
            if eid and first_eid is None:
                first_eid = eid
        if first_eid:
            self.store.set_folder_image_event(iid, first_eid)
        self._remember(key, self._sig(st))
        self.frames_total += 1
        self.processed_total += 1
        if result["key"]:
            self.key_total += 1
        if result["violation"]:
            self.violation_total += 1
        self.last_frame_ts = time.time()
        return True

    def _analyze(self, engine, frame, name: str) -> dict:
        """单张判定：区域命中 + 实验服证据 + 人数超限；返回落库字段与待发事件。"""
        cid = self.camera["id"]
        dets = engine.detect_persons(frame) or []
        polys = [(z["id"], z["name"], z["_poly"])
                 for z in self._zones_with_polygons()]
        no_boxes, wear_boxes = ppe_boxes(self.ppe, frame) if dets else ([], [])
        zone_stat: dict[int, dict] = {}
        in_zone = 0
        for det in dets:
            x1, y1, x2, y2 = det.xyxy
            anchor = (int((x1 + x2) / 2), int(y2))
            hits = [(zid, zname) for zid, zname, poly in polys
                    if _point_inside(poly, anchor)]
            if not hits:
                continue
            in_zone += 1
            status = ppe_status_from_boxes(no_boxes, wear_boxes, det.xyxy)
            for zid, zname in hits:
                st = zone_stat.setdefault(zid, {"name": zname, "persons": 0, "no": 0, "wearing": 0})
                st["persons"] += 1
                if status == "no":
                    st["no"] += 1
                elif status == "wearing":
                    st["wearing"] += 1

        events: list[dict] = []
        violations: list[dict] = []
        zones_hit: list[str] = []
        for zid, st in zone_stat.items():
            zname = st["name"]
            zones_hit.append(zname)
            # 图片之间没有连续性，默认不为"有人"造 enter 事件（否则每次截图都像新进入，
            # 告警语义错误且易刷屏）；需要时用 LS_FOLDER_ENTER_EVENTS=1 打开并按冷却限频。
            if settings.folder_enter_events:
                events.append({"zone_id": zid, "behavior": "enter",
                               "detail": f"图片「{name}」：进入安全区「{zname}」"})
            if st["no"]:
                violations.append({"type": "enter_no_ppe", "zone": zname, "count": st["no"]})
                events.append({"zone_id": zid, "behavior": "enter_no_ppe",
                               "detail": f"图片「{name}」：进入安全区「{zname}」未穿实验服"})

        for rule in self.rules.occupancy_rules(cid):
            threshold = int(rule.get("threshold") or 0)
            if threshold and in_zone > threshold:
                violations.append({"type": "occupancy_over", "zone": None,
                                   "count": in_zone, "threshold": threshold})
                events.append({"zone_id": None, "behavior": "occupancy_over",
                               "level": rule["level"],
                               "detail": f"图片「{name}」：在场人数 {in_zone} 超过阈值 {threshold}"})

        return {
            "persons": len(dets),
            "in_zone": in_zone,
            "ppe_no": sum(s["no"] for s in zone_stat.values()),
            "ppe_wearing": sum(s["wearing"] for s in zone_stat.values()),
            "zones": zones_hit,
            "violations": violations,
            # key=画面中出现人员：即使没有规则/实验服模型，也把"有人"的图筛出来汇报
            "key": len(dets) > 0,
            "violation": bool(violations),
            "events": events,
        }

    def _zones_with_polygons(self) -> list[dict]:
        from .zones import parse_polygon
        zones = self.store.zones(self.camera["id"])
        for z in zones:
            z["_poly"] = parse_polygon(z["polygon"])
        return zones

    def _fire_event(self, ev: dict, snapshot: str | None) -> int | None:
        rule = self.rules.match(self.camera["id"], ev.get("zone_id"), ev["behavior"],
                                0, ev.get("level"))
        if rule is None:
            return None
        detail = str(ev.get("detail") or "")[:500]
        eid = self.store.add_event(self.camera["id"], ev["behavior"], rule["level"],
                                   0, detail, snapshot)
        row = self.store.event_by_id(eid)
        if row is not None:
            row.pop("snapshot", None)
            row.pop("video", None)
            row["has_snapshot"] = bool(snapshot)
            row["has_video"] = False
            try:
                dispatcher.dispatch(row)
            except Exception as e:  # noqa: BLE001  告警派发异常不能反过来丢识别结果
                print(f"[folder-{self.camera['id']}] dispatch error: {e}")
        if not ev.get("quiet"):
            print(f"[folder-{self.camera['id']}] event#{eid} {ev['behavior']} "
                  f"L{rule['level']} {detail}")
        return eid

    def _save_evidence(self, img, source: Path) -> str | None:
        """违规证据副本（原始截图不动）。JPEG 直接复制保真，其它格式转高质量 JPEG。"""
        try:
            settings.snapshot_dir.mkdir(parents=True, exist_ok=True)
            dest = settings.snapshot_dir / (
                f"img_{self.camera['id']}_{datetime.now().strftime('%Y%m%d_%H%M%S')}_"
                f"{uuid.uuid4().hex[:8]}.jpg")
            if source.suffix.lower() in (".jpg", ".jpeg"):
                dest.write_bytes(source.read_bytes())
            else:
                ok, buf = cv2.imencode(".jpg", img, [int(cv2.IMWRITE_JPEG_QUALITY), 88])
                if not ok:
                    return None
                dest.write_bytes(buf.tobytes())
            return str(dest)
        except OSError as e:
            self._set_error(f"证据保存失败: {e}")
            return None

    # ---- 控制/状态 ----
    def wake(self) -> None:
        """外部触发立即扫描（POST /api/folders/{id}/scan）。"""
        self._wake.set()

    def stop(self) -> None:
        self.stop_flag.set()
        self._wake.set()

    def folder_status(self) -> dict:
        return {
            "kind": "folder",
            "folder": str(self.camera.get("source") or ""),
            "processed_total": self.processed_total,
            "key_total": self.key_total,
            "violation_total": self.violation_total,
            "error_total": self.error_total,
            "backlog": self.backlog,
            "last_scan_ts": self.last_scan_ts,
            "current_file": self.current_file,
        }


def _point_inside(poly, point) -> bool:
    """复用 zones.point_inside（保留懒导入，便于测试替换/避免循环依赖）。"""
    from .zones import point_inside
    return point_inside(poly, point)
