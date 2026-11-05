"""视觉公共工具：中文安全读图、分析画布归一化、实验服证据分级。

视频 worker（pipeline）与图片文件夹 worker（folders）共用本模块，避免两套判定口径漂移。
"""
from pathlib import Path

import cv2
import numpy as np

from .config import settings
from .zones import boxes_overlap

# 安全区坐标以 1280x720 为约定画布（前端绘制/存储口径，见 models.Zone）
ANALYSIS_W = 1280
ANALYSIS_H = 720


def read_image(path) -> np.ndarray | None:
    """读取图片，Windows 中文/特殊字符路径安全（cv2.imread 会静默失败）。

    返回 BGR 三通道；空文件/损坏/非图片返回 None。
    """
    try:
        data = np.fromfile(str(Path(path)), dtype=np.uint8)
    except OSError:
        return None
    if data.size == 0:
        return None
    try:
        return cv2.imdecode(data, cv2.IMREAD_COLOR)
    except cv2.error:
        return None


def normalize_frame(img: np.ndarray) -> np.ndarray:
    """缩放到约定分析画布（与视频源一致；区域坐标才能通用）。"""
    if img.shape[1] == ANALYSIS_W and img.shape[0] == ANALYSIS_H:
        return img
    return cv2.resize(img, (ANALYSIS_W, ANALYSIS_H))


def ppe_boxes(ppe, frame: np.ndarray):
    """一次推理拿到全图实验服证据框：([违规框], [穿戴框])。

    T-301（PF-01）：违规类 + 全部穿戴类共享同一次整图推理（旧实现逐类别各跑一次，
    3 人同帧进场 ×2 类 = 6 次整图推理）；视频 worker 另做帧级缓存（每分析帧最多一次）。
    """
    if ppe is None:
        return [], []
    out, geom = ppe.detect_all_raw(frame)
    no_boxes = [det.xyxy for det in
                ppe.detect_class_from_raw(out, geom, settings.ppe_violation_class,
                                          conf=settings.ppe_conf)]
    wear_boxes = []
    for name in settings.ppe_class_names.split(","):
        wear_boxes += [det.xyxy for det in
                       ppe.detect_class_from_raw(out, geom, name.strip(),
                                                 conf=settings.ppe_wearing_conf)]
    return no_boxes, wear_boxes


def ppe_status_from_boxes(no_boxes, wear_boxes, person_box) -> str:
    """按已算好的证据框判定单人：'no' | 'wearing' | 'unknown'。"""
    for box in no_boxes:
        if boxes_overlap(box, person_box):
            return "no"
    for box in wear_boxes:
        if boxes_overlap(box, person_box):
            return "wearing"
    return "unknown"


def ppe_status(ppe, frame: np.ndarray, person_box) -> str:
    """实验服证据分级：'no'（显式违规证据） | 'wearing'（穿戴证据） | 'unknown'。

    违规告警只认 No Lab Coat 类的显式检测（AP 0.81）；穿戴证据用低阈值；
    两者皆无时返回 unknown，绝不把"证据不足"当"未穿"——防对合规人员误告警。
    """
    if ppe is None:
        return "unknown"
    no_boxes, wear_boxes = ppe_boxes(ppe, frame)
    return ppe_status_from_boxes(no_boxes, wear_boxes, person_box)

