"""安全区（多边形）判定：目标锚点取检测框底边中点（脚的位置）。"""
import cv2
import numpy as np


def parse_polygon(polygon: list[list[int]]) -> np.ndarray:
    pts = [(int(p[0]), int(p[1])) for p in polygon]
    if len(pts) < 3:
        raise ValueError("polygon needs >= 3 points")
    return np.array(pts, dtype=np.int32)


def point_inside(poly: np.ndarray, point: tuple[int, int]) -> bool:
    return cv2.pointPolygonTest(poly, point, False) >= 0


def boxes_overlap(a: tuple, b: tuple) -> bool:
    """两个 xyxy 框是否有交集（用于人框↔实验服框的穿戴判定）。"""
    ax1, ay1, ax2, ay2 = a
    bx1, by1, bx2, by2 = b
    return ax1 < bx2 and bx1 < ax2 and ay1 < by2 and by1 < ay2


class ZoneState:
    """按 track_id 跟踪的区域进出状态（T-306 / B-12：进出迟滞）。

    连续 zone_enter_frames 帧观测在内才判进场、连续 zone_exit_frames 帧观测
    在外/轨迹消失才判离场——单帧漏检/闪现不再产生 enter/exit 抖动与重复告警。
    轨迹消失（空帧/部分消失路径反复 update(tid, False)）同样按 exit_frames 去抖。
    """

    MAX_TRACKS = 5000  # 兜底上限：正常画面不会同时有这么多轨迹（异常/长跑防内存缓涨）

    def __init__(self, enter_frames: int | None = None, exit_frames: int | None = None):
        from .config import settings  # 局部导入（避免 import 环）

        ef = settings.zone_enter_frames if enter_frames is None else enter_frames
        xf = settings.zone_exit_frames if exit_frames is None else exit_frames
        self.enter_frames = max(1, int(ef))
        self.exit_frames = max(1, int(xf))
        # tid -> {"inside": bool, "streak": int, "pending": bool | None}
        self._state: dict[int, dict] = {}

    @property
    def inside_ids(self) -> list[int]:
        """当前判定为在区域内的 track（空帧批量离场路径使用）。"""
        return [tid for tid, s in self._state.items() if s["inside"]]

    @property
    def size(self) -> int:
        return len(self._state)

    def update(self, track_id: int, inside: bool):
        """返回 (entered, exited)；无变化返回 None。"""
        s = self._state.get(track_id)
        if s is None:
            s = {"inside": False, "streak": 0, "pending": None}
            self._state[track_id] = s
        if inside == s["inside"]:
            s["streak"] = 0
            s["pending"] = None
            return None
        if s["pending"] is not inside:  # 观测方向翻转：连击重计
            s["pending"] = inside
            s["streak"] = 1
        else:
            s["streak"] += 1
        need = self.enter_frames if inside else self.exit_frames
        if s["streak"] >= need:
            s["inside"] = inside
            s["streak"] = 0
            s["pending"] = None
            return (inside, not inside)  # (True, False)=进场 / (False, True)=离场
        return None

    def retain(self, active_tids: set[int], max_size: int | None = None) -> int:
        """内存回收：删掉"非活跃且已判离场"的轨迹。

        与迟滞语义配合：已判 inside 的轨迹即使本帧消失也保留（离场需 exit_frames
        连击，消失路径逐帧累积）；连击完成后（inside=False, streak=0）自然回收。
        active 集合通常来自 ByteTrack 本帧输出；active 项绝不淘汰。
        """
        active = set(active_tids)
        removed = 0
        for tid in list(self._state):
            if tid not in active and not self._state[tid]["inside"]:
                del self._state[tid]
                removed += 1
        cap = self.MAX_TRACKS if max_size is None else max(0, int(max_size))
        if len(self._state) > cap:
            for tid in list(self._state):
                if len(self._state) <= cap:
                    break
                if tid not in active:
                    del self._state[tid]
                    removed += 1
        return removed
