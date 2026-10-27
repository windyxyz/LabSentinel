"""ONNX Runtime 推理引擎：后端自动降级 directml → vitisai → cpu（M0 实测三档均可用）。"""
import threading
import time
from pathlib import Path

import cv2
import numpy as np
import onnxruntime as ort

from .config import settings

# 【根因修复 2026-10-26】DirectML/VitisAI 会话跨线程并发 run 会挂死 GPU 驱动并冻死整机
# （事件日志证据：3× Kernel-Power 41 硬重启、无蓝屏无 WHEA 无资源耗尽）。
# 因此所有 DML/VitisAI 推理必须全进程串行——跨实例共享一把模块级锁（人检引擎与 PPE 引擎也互斥）。
# CPU 后端不加锁（ORT 自身线程安全，串行会白白损失多路吞吐）。
_ACCEL_LOCK = threading.Lock()


class Detection:
    __slots__ = ("xyxy", "conf", "cls")

    def __init__(self, xyxy, conf, cls):
        self.xyxy = xyxy
        self.conf = conf
        self.cls = cls

    @property
    def bottom_center(self):
        x1, _, x2, y2 = self.xyxy
        return (int((x1 + x2) / 2), int(y2))

    @property
    def area(self):
        x1, y1, x2, y2 = self.xyxy
        return max(0, x2 - x1) * max(0, y2 - y1)


def _providers_for(name: str) -> list[str]:
    avail = set(ort.get_available_providers())
    mapping = {
        "directml": ["DmlExecutionProvider", "CPUExecutionProvider"],
        "vitisai": ["VitisAIExecutionProvider", "CPUExecutionProvider"],
        "cpu": ["CPUExecutionProvider"],
    }
    return mapping[name] if set(mapping[name]) & avail else mapping["cpu"]


class Engine:
    """推理引擎。NPU(vitisai) 后端配 XINT8 模型，其余后端配 FP32 模型。"""

    PERSON_CLS = 0

    def __init__(self, model_path=None, prefer=None):
        self.backend = None
        self.sess = None
        self.input_name = None
        self._accel = False  # directml/vitisai 后端为 True，run 走全局串行锁
        self._init_session(model_path, prefer)

    def _run(self, blob):
        """统一的 sess.run 入口：加速后端强制串行（见模块头注释）。"""
        if self._accel:
            with _ACCEL_LOCK:
                return self.sess.run(None, {self.input_name: blob})
        return self.sess.run(None, {self.input_name: blob})

    def _init_session(self, model_path=None, prefer=None):
        attempts = []
        order = [prefer] if prefer else [b.strip() for b in settings.backend_order.split(",")]
        for name in order:
            model = str(model_path) if model_path else str(
                settings.model_int8 if name == "vitisai" else settings.model_fp32)
            try:
                t0 = time.perf_counter()
                sess = ort.InferenceSession(model, providers=_providers_for(name))
                if not sess.get_providers()[0].startswith(("Dml", "VitisAI")) and name != "cpu":
                    continue  # 实际没拿到目标后端
                self.sess, self.backend = sess, name
                self.input_name = sess.get_inputs()[0].name
                self._accel = name in ("directml", "vitisai")
                print(f"[engine] backend={name} model={model} init={time.perf_counter()-t0:.1f}s"
                      + (" (accel: run 串行锁启用)" if self._accel else ""))
                return
            except Exception as e:  # noqa: BLE001
                attempts.append(f"{name}: {e}")
        raise RuntimeError("no backend available: " + "; ".join(attempts))

    def _letterbox(self, img, s=None):
        s = s or settings.imgsz
        h, w = img.shape[:2]
        r = min(s / h, s / w)
        nh, nw = int(h * r), int(w * r)
        canvas = np.full((s, s, 3), 114, np.uint8)
        resized = cv2.resize(img, (nw, nh))
        top, left = (s - nh) // 2, (s - nw) // 2
        canvas[top:top + nh, left:left + nw] = resized
        return canvas, r, left, top

    def _nms(self, boxes, scores, iou_thres):
        x1, y1, x2, y2 = boxes[:, 0], boxes[:, 1], boxes[:, 2], boxes[:, 3]
        areas = (x2 - x1) * (y2 - y1)
        order = scores.argsort()[::-1]
        keep = []
        while order.size:
            i = order[0]
            keep.append(i)
            xx1 = np.maximum(x1[i], x1[order[1:]])
            yy1 = np.maximum(y1[i], y1[order[1:]])
            xx2 = np.minimum(x2[i], x2[order[1:]])
            yy2 = np.minimum(y2[i], y2[order[1:]])
            inter = np.maximum(0, xx2 - xx1) * np.maximum(0, yy2 - yy1)
            ious = inter / (areas[i] + areas[order[1:]] - inter + 1e-9)
            order = order[1:][ious <= iou_thres]
        return np.array(keep, dtype=int)

    def detect_persons(self, frame_bgr) -> list[Detection]:
        canvas, r, dx, dy = self._letterbox(frame_bgr)
        rgb = cv2.cvtColor(canvas, cv2.COLOR_BGR2RGB).astype(np.float32) / 255.0
        blob = np.transpose(rgb, (2, 0, 1))[None]
        out = self._run(blob)[0][0]  # sess.run→[(1,C,N)]，[0][0]→(C,N)=(84,8400)
        pred = out.T  # (8400, 4+80)
        person = pred[:, 4 + self.PERSON_CLS]
        m = person > settings.conf_thres
        if not m.any():
            return []
        boxes = np.stack([
            (pred[m, 0] - pred[m, 2] / 2 - dx) / r,
            (pred[m, 1] - pred[m, 3] / 2 - dy) / r,
            (pred[m, 0] + pred[m, 2] / 2 - dx) / r,
            (pred[m, 1] + pred[m, 3] / 2 - dy) / r,
        ], axis=1)
        scores = person[m]
        keep = self._nms(boxes, scores, settings.iou_thres)
        dets = []
        for i in keep:
            det = Detection(boxes[i].tolist(), float(scores[i]), 0)
            if det.area >= settings.min_box_area:
                dets.append(det)
        return dets


_engine: Engine | None = None


def get_engine() -> Engine:
    global _engine
    if _engine is None:
        _engine = Engine()
    return _engine


class PpeEngine(Engine):
    """自定义类别检测引擎（实验服等）。class_names 按模型输出顺序，名称小写匹配。"""

    def __init__(self, model_path, class_names: list[str]):
        super().__init__(model_path=model_path)
        self.class_names = [c.lower() for c in class_names]

    def detect_class(self, frame_bgr, class_name: str, conf: float | None = None):
        """返回指定类别的检测框。精确匹配优先，避免 'lab coat' 子串误命中 'no lab coat'。"""
        want = class_name.lower()
        idx = [i for i, n in enumerate(self.class_names) if n == want]
        if not idx:
            idx = [i for i, n in enumerate(self.class_names) if want in n]
        if not idx:
            return []
        conf = conf if conf is not None else settings.conf_thres
        canvas, r, dx, dy = self._letterbox(frame_bgr)
        rgb = cv2.cvtColor(canvas, cv2.COLOR_BGR2RGB).astype(np.float32) / 255.0
        blob = np.transpose(rgb, (2, 0, 1))[None]
        out = self._run(blob)[0][0]  # sess.run→[(1,C,N)]，[0][0]→(C,N)=(4+num_cls,N)
        pred = out.T
        best = np.zeros(pred.shape[0], dtype=np.float32)
        for i in idx:
            best = np.maximum(best, pred[:, 4 + i])
        m = best > conf
        if not m.any():
            return []
        boxes = np.stack([
            (pred[m, 0] - pred[m, 2] / 2 - dx) / r,
            (pred[m, 1] - pred[m, 3] / 2 - dy) / r,
            (pred[m, 0] + pred[m, 2] / 2 - dx) / r,
            (pred[m, 1] + pred[m, 3] / 2 - dy) / r,
        ], axis=1)
        keep = self._nms(boxes, best[m], settings.iou_thres)
        return [Detection(boxes[i].tolist(), float(best[m][i]), 0) for i in keep]


_ppe_engine: PpeEngine | None = None
_ppe_engine_tried = False


def _load_class_names(model_path: Path) -> list[str]:
    """PPE 类别词表：优先读模型同名 classes 文件（如 yolo11n_ppe.onnx → yolo11n_ppe.classes.txt），
    一行一个类名、顺序即输出通道顺序——换模型零改代码（M1.5 #2）。
    未提供时回退 settings.ppe_class_names（逗号分隔，兼容旧配置）。"""
    sidecar = model_path.with_suffix(".classes.txt")
    if sidecar.exists():
        names = [ln.strip() for ln in sidecar.read_text(encoding="utf-8").splitlines()
                 if ln.strip() and not ln.strip().startswith("#")]
        if names:
            print(f"[engine] ppe classes from {sidecar.name}: {names}")
            return names
        print(f"[warn] {sidecar.name} 为空，回退 LS_PPE_CLASS_NAMES")
    elif not sidecar.exists():
        print(f"[warn] 未找到 {sidecar.name}，使用 LS_PPE_CLASS_NAMES（建议随模型提供词表文件）")
    return [n for n in settings.ppe_class_names.split(",") if n.strip()]


def get_ppe_engine() -> PpeEngine | None:
    """实验服模型可选：settings.ppe_model 未配置时返回 None（匿名/未知模式）。"""
    global _ppe_engine, _ppe_engine_tried
    if _ppe_engine_tried:
        return _ppe_engine
    _ppe_engine_tried = True
    if not settings.ppe_model or not Path(str(settings.ppe_model)).exists():
        return None
    try:
        _ppe_engine = PpeEngine(settings.ppe_model, _load_class_names(Path(str(settings.ppe_model))))
        print(f"[engine] ppe model loaded: {settings.ppe_model}")
    except Exception as e:  # noqa: BLE001
        print(f"[engine] ppe model load failed: {e}")
    return _ppe_engine

