# 生成检测效果演示视频：人(绿) / Lab Coat(黄) / No Lab Coat(红)
# 用法（m0-venv）:
#   python 效果演示/make_demo.py                        # 默认两段演示视频
#   python 效果演示/make_demo.py --ppe m0-spike/yolo11n_ppe_v2.onnx --suffix _v2
# 类别词表自动从 <ppe模型>.classes.txt 读取（见 server/app/engine.py）；无则回退默认 10 类。
import argparse
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'server'))
import cv2
import numpy as np
from app.engine import Engine, PpeEngine

BASE = os.path.join(os.path.dirname(__file__), '..')
DEFAULT_NAMES = ['Glasses', 'Gloves', 'Lab Coat', 'Mask', 'No Glasses',
                 'No Gloves', 'No Lab Coat', 'No Mask', 'Person', 'Shoes']
COLOR = {'person': (80, 220, 80), 'lab coat': (60, 200, 255), 'no lab coat': (60, 60, 255)}


def parse_args():
    ap = argparse.ArgumentParser(description="检测效果演示视频生成（v1/v2 对比时用 --suffix 区分输出）")
    ap.add_argument("--ppe", default=os.path.join(BASE, 'm0-spike', 'yolo11n_ppe.onnx'),
                    help="PPE 模型路径（默认 m0-spike/yolo11n_ppe.onnx）")
    ap.add_argument("--suffix", default="",
                    help="输出文件名后缀（如 _v2 → lab-walk-检测效果_v2.mp4）")
    ap.add_argument("--videos", nargs="*", default=[],
                    help="指定视频路径（默认跑 m0-spike/data 下两段演示视频）")
    ap.add_argument("--device", default="directml", choices=["directml", "cpu"],
                    help="推理后端：directml（默认，快）/ cpu（慢但结果一致，用于避免与其它推理抢 GPU）")
    return ap.parse_args()


def load_names(ppe_path: str) -> list[str]:
    """与 engine._load_class_names 同源：优先 <model>.classes.txt。"""
    sidecar = str(ppe_path)
    for cand in (sidecar.rsplit('.', 1)[0] + '.classes.txt', sidecar + '.classes.txt'):
        if os.path.exists(cand):
            names = [ln.strip() for ln in open(cand, encoding='utf-8') if ln.strip()]
            if names:
                print('classes from', os.path.basename(cand), ':', names)
                return names
    print('classes fallback: DEFAULT_NAMES')
    return DEFAULT_NAMES


def draw(frame, label, box, conf, color):
    x1, y1, x2, y2 = map(int, box)
    cv2.rectangle(frame, (x1, y1), (x2, y2), color, 2)
    txt = f"{label} {conf:.2f}"
    (tw, th), _ = cv2.getTextSize(txt, cv2.FONT_HERSHEY_SIMPLEX, 0.55, 2)
    cv2.rectangle(frame, (x1, y1 - th - 8), (x1 + tw + 6, y1), color, -1)
    cv2.putText(frame, txt, (x1 + 3, y1 - 5), cv2.FONT_HERSHEY_SIMPLEX, 0.55, (0, 0, 0), 2)


def main():
    args = parse_args()
    person_eng = Engine(model_path=os.path.join(BASE, 'm0-spike', 'yolo11n.onnx'), prefer=args.device)
    # PpeEngine 不支持 prefer 参数，用 backend_order 控制（Engine 构造时读取一次）
    from app.config import settings
    settings.backend_order = args.device
    ppe_eng = PpeEngine(args.ppe, load_names(args.ppe))

    def annotate(frame):
        for d in person_eng.detect_persons(frame):
            draw(frame, 'person', d.xyxy, d.conf, COLOR['person'])
        for cls in ('lab coat', 'no lab coat'):
            for d in ppe_eng.detect_class(frame, cls, conf=0.20):
                draw(frame, cls, d.xyxy, d.conf, COLOR[cls])
        return frame

    def process(video, out_name, sample_jpg):
        cap = cv2.VideoCapture(video)
        fps = cap.get(cv2.CAP_PROP_FPS) or 24
        vw = cv2.VideoWriter(out_name, cv2.VideoWriter_fourcc(*'mp4v'), fps, (1280, 720))
        i = 0
        while True:
            ok, f = cap.read()
            if not ok:
                break
            f = cv2.resize(f, (1280, 720))
            if i % 2 == 0:  # 每 2 帧标注一次（肉眼流畅且省时）
                f = annotate(f)
            vw.write(f)
            if i in (60, 120, 180):
                cv2.imwrite(sample_jpg.format(i), f)
            i += 1
        cap.release()
        vw.release()
        print('done:', out_name, i, 'frames')

    videos = args.videos or [os.path.join(BASE, 'm0-spike/data/lab-walk-24076.mp4'),
                             os.path.join(BASE, 'm0-spike/data/lab-equip-21453.mp4')]
    here = os.path.dirname(__file__)
    for v in videos:
        stem = os.path.splitext(os.path.basename(v))[0].split('-')[0].split('_')[0]
        process(v,
                os.path.join(here, f'{stem}-检测效果{args.suffix}.mp4'),
                os.path.join(here, f'{stem}-帧{{}}{args.suffix}.jpg'))


if __name__ == '__main__':
    main()
