# M0 / S2c 备选: 用 onnxruntime 内置 quantize_static 生成 INT8（不依赖 quark/MSVC）
# 用法: python quantize_ort.py
import os
import numpy as np
import cv2
import onnxruntime as ort
from onnxruntime.quantization import CalibrationDataReader, QuantFormat, QuantType, quantize_static

INPUT_MODEL = "yolo11n.onnx"
OUTPUT_MODEL = "yolo11n_ort_int8.onnx"
CALIB_DIR = "calib"

session = ort.InferenceSession(INPUT_MODEL, providers=["CPUExecutionProvider"])
input_name = session.get_inputs()[0].name

def load_calib():
    data = []
    for f in sorted(os.listdir(CALIB_DIR)):
        if not f.lower().endswith((".jpg", ".png")):
            continue
        img = cv2.imread(os.path.join(CALIB_DIR, f))
        img = cv2.resize(img, (640, 640)).astype(np.float32)[:, :, ::-1] / 255.0  # BGR->RGB
        data.append(img.transpose(2, 0, 1)[None])
    return data

class Reader(CalibrationDataReader):
    def __init__(self):
        self.iter = iter([{input_name: d} for d in load_calib()])
    def get_next(self):
        return next(self.iter, None)

print("calibrating with", len(os.listdir(CALIB_DIR)), "images")
quantize_static(
    model_input=INPUT_MODEL,
    model_output=OUTPUT_MODEL,
    calibration_data_reader=Reader(),
    quant_format=QuantFormat.QDQ,
    activation_type=QuantType.QUInt8,
    weight_type=QuantType.QInt8,
    per_channel=True,
)
print("done:", OUTPUT_MODEL, os.path.getsize(OUTPUT_MODEL), "bytes")
