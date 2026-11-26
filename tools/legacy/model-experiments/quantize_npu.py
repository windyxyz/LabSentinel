# M0 / S2c: 用 AMD Quark 把 yolo11n.onnx 量化为 NPU 可用的 INT8
# 用法: python quantize_npu.py [XINT8|A8W8]
# 依赖: npu-venv (py3.12) + amd_quark wheel + calibration 图 (calib/*.jpg)
import sys, os
import numpy as np
import cv2
import onnxruntime as ort
from quark.onnx import CalibMethod, Int8Spec, ModelQuantizer, QConfig, QLayerConfig

MODE = sys.argv[1] if len(sys.argv) > 1 else "XINT8"
INPUT_MODEL = "yolo11n.onnx"
OUTPUT_MODEL = f"yolo11n_{MODE.lower()}.onnx"
CALIB_DIR = "calib"

class ImageDataReader:
    """按 Quark 约定: get_next() 返回 {input_name: ndarray} 或 None; 含 rewind/reset"""
    def __init__(self, model_path, calib_dir):
        names = [f for f in os.listdir(calib_dir) if f.lower().endswith((".jpg", ".png"))]
        self.data_list = []
        for n in names:
            img = cv2.imread(os.path.join(calib_dir, n))
            img = cv2.resize(img, (640, 640)).astype(np.float32)  # BGR
            img = img.transpose(2, 0, 1)[None] / 255.0            # NCHW RGB 无关紧要(校准统计用)
            self.data_list.append(img)
        session = ort.InferenceSession(model_path, providers=["CPUExecutionProvider"])
        self.input_name = session.get_inputs()[0].name
        self.datasize = len(self.data_list)
        self.enum_data = None

    def get_next(self):
        if self.enum_data is None:
            self.enum_data = iter([{self.input_name: d} for d in self.data_list])
        return next(self.enum_data, None)

    def __len__(self):
        return self.datasize

    def rewind(self):
        self.enum_data = None

    def reset(self):
        self.enum_data = None

def build_config(mode):
    if mode == "XINT8":
        from quark.onnx import XInt8Spec
        act = wgt = XInt8Spec()
        extra = {"EnableNPUCnn": True, "AlignSlice": False, "FoldRelu": True, "AlignConcat": True}
    else:
        act = wgt = Int8Spec()
        extra = {"EnableNPUCnn": False, "AlignSlice": False, "FoldRelu": True, "AlignConcat": True}
    act.set_calibration_method(CalibMethod.MinMax)
    wgt.set_calibration_method(CalibMethod.MinMax)
    return QConfig(
        global_config=QLayerConfig(activation=act, weight=wgt),
        algo_config=[],
        use_external_data_format=False,
        exclude=[],
        **extra,
    )

print(f"quantizing {INPUT_MODEL} -> {OUTPUT_MODEL} (mode={MODE})")
reader = ImageDataReader(INPUT_MODEL, CALIB_DIR)
print(f"calibration images: {len(reader)}")
quantizer = ModelQuantizer(build_config(MODE))
quantizer.quantize_model(INPUT_MODEL, OUTPUT_MODEL, reader)
print("done:", OUTPUT_MODEL, os.path.getsize(OUTPUT_MODEL), "bytes")
