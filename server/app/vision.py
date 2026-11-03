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

    图片文件夹可能一张图多人，逐人调用 detect_class 会重复跑模型；先按图取框，
    再对每个人做框重叠判定（ppe_status_from_boxes），每人只花几何计算。
    """
    if ppe is None:
        return [], []
    no_boxes = [det.xyxy for det in
                ppe.detect_class(frame, settings.ppe_violation_class, conf=settings.ppe_conf)]
    wear_boxes = []
    for name in settings.ppe_class_names.split(","):
        wear_boxes += [det.xyxy for det in
                       ppe.detect_class(frame, name.strip(), conf=settings.ppe_wearing_conf)]
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

