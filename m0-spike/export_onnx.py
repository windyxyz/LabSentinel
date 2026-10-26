# M0: 导出 yolo11n 为 ONNX（供 DirectML / NPU 推理使用）
from ultralytics import YOLO
m = YOLO("yolo11n.pt")
p = m.export(format="onnx", imgsz=640, opset=13)
print("exported:", p)
