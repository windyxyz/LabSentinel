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
    """按 track_id 跟踪的区域进出状态。"""

    MAX_TRACKS = 5000  # 兜底上限：正常画面不会同时有这么多轨迹（异常/长跑防内存缓涨）

    def __init__(self):
        self._inside: dict[int, bool] = {}

    @property
    def inside_ids(self) -> list[int]:
        """当前判定为在区域内的 track（2026-10-30 审阅：供空帧批量离场用）。"""
        return [tid for tid, inside in self._inside.items() if inside]

    @property
    def size(self) -> int:
        return len(self._inside)

    def update(self, track_id: int, inside: bool):
        """返回 (entered, exited)；无变化返回 None。"""
        was = self._inside.get(track_id, False)
        self._inside[track_id] = inside
        if inside and not was:
            return (True, False)
        if was and not inside:
            return (False, True)
        return None

    def retain(self, active_tids: set[int], max_size: int | None = None) -> int:
        """只保留当前画面仍活跃的轨迹（106：离场状态已由 pipeline 处理，这里做内存回收）。

        返回删除条数。active 集合通常来自 ByteTrack 本帧输出；超过上限时再按插入顺序
        淘汰非 active 项（active 项绝不淘汰，避免破坏在场/离场语义）。
        """
        active = set(active_tids)
        removed = 0
        for tid in list(self._inside):
            if tid not in active:
                del self._inside[tid]
                removed += 1
        cap = self.MAX_TRACKS if max_size is None else max(0, int(max_size))
        if len(self._inside) > cap:
            for tid in list(self._inside):
                if len(self._inside) <= cap:
                    break
                if tid not in active:
                    del self._inside[tid]
                    removed += 1
        return removed
