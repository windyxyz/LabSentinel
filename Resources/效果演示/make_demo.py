# 生成检测效果演示视频：人(绿) / PPE 类别按 --draw 选择绘制
# 用法（m0-venv）:
#   python Resources/效果演示/make_demo.py                        # 默认两段演示视频
#   python Resources/效果演示/make_demo.py --ppe m0-spike/yolo11n_ppe.onnx --suffix _v2
#   python Resources/效果演示/make_demo.py --draw "lab coat,no lab coat,hairnet"   # 扩展绘制类别
#   python Resources/效果演示/make_demo.py --samples              # 顺带输出抽样 jpg 到 Resources/logs/demo_samples/
# 类别词表自动从 <ppe模型>.classes.txt 读取（见 server/app/engine.py）；无则回退默认 10 类。
import argparse
import os
import re
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'server'))
import cv2
import numpy as np
from app.engine import Engine, PpeEngine

BASE = os.path.join(os.path.dirname(__file__), '..', '..')  # Resources/效果演示 → 项目根
DEFAULT_NAMES = ['Glasses', 'Gloves', 'Lab Coat', 'Mask', 'No Glasses',
                 'No Gloves', 'No Lab Coat', 'No Mask', 'Person', 'Shoes']
# 任务40-L1：输出命名显式映射（旧逻辑 split('-')[0] 把两段源都截成 "lab" 后写覆盖先写）
NAME_MAP = {"lab-walk-24076": "lab-walk", "lab-equip-21453": "lab-equip"}
# 任务40-L2：11 类颜色表（person 绿；违规类红系；穿戴证据黄/蓝/青系区分）
COLORS = {
    'person': (80, 220, 80),
    'lab coat': (60, 200, 255), 'no lab coat': (60, 60, 255),
    'gloves': (255, 200, 60), 'no gloves': (60, 60, 255),
    'hairnet': (200, 160, 255), 'mask': (255, 160, 200), 'no mask': (60, 60, 255),
    'glasses': (160, 220, 255), 'no glasses': (60, 60, 255),
    'shoes': (120, 180, 255),
}
DEFAULT_DRAW = 'lab coat,no lab coat'


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
    ap.add_argument("--draw", default=DEFAULT_DRAW,
                    help="要绘制的 PPE 类别（逗号分隔，默认 'lab coat,no lab coat'；person 始终绘制）")
    ap.add_argument("--samples", action="store_true",
                    help="顺带输出抽样 jpg 到 Resources/logs/demo_samples/（默认关闭，不写 Resources/效果演示/）")
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


def resolve_draw(names: list[str], draw_arg: str) -> list[str]:
    """--draw 的类别必须在词表/颜色表内（大小写不敏感），未知类名直接报错，避免静默画错。"""
    known = {n.lower() for n in names}
    out = []
    for cls in [c.strip().lower() for c in draw_arg.split(',') if c.strip()]:
        if cls == 'person' or cls not in known or cls not in COLORS:
            raise SystemExit(f"[错误] --draw 含未知/不可绘制类别: '{cls}'"
                             f"（词表={names}；颜色表={sorted(k for k in COLORS if k != 'person')}）")
        out.append(cls)
    return out


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
    names = load_names(args.ppe)
    ppe_eng = PpeEngine(args.ppe, names)
    draw_classes = resolve_draw(names, args.draw)
    print('draw classes:', draw_classes)
    samples_dir = os.path.join(BASE, 'Resources', 'logs', 'demo_samples')
    if args.samples:
        os.makedirs(samples_dir, exist_ok=True)

    def annotate(frame):
        for d in person_eng.detect_persons(frame):
            draw(frame, 'person', d.xyxy, d.conf, COLORS['person'])
        for cls in draw_classes:
            for d in ppe_eng.detect_class(frame, cls, conf=0.20):
                draw(frame, cls, d.xyxy, d.conf, COLORS[cls])
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
            if args.samples and i in (60, 120, 180):
                # imencode+tofile：cv2.imwrite 在 Windows 上对中文路径静默失败
                ok_jpg, buf = cv2.imencode('.jpg', f)
                if ok_jpg:
                    buf.tofile(os.path.join(samples_dir, os.path.basename(sample_jpg.format(i))))
            i += 1
        cap.release()
        vw.release()
        print('done:', out_name, i, 'frames')

    videos = args.videos or [os.path.join(BASE, 'm0-spike/data/lab-walk-24076.mp4'),
                             os.path.join(BASE, 'm0-spike/data/lab-equip-21453.mp4')]
    here = os.path.dirname(__file__)
    for v in videos:
        base = os.path.splitext(os.path.basename(v))[0].split('_')[0]
        stem = NAME_MAP.get(base, re.sub(r"-\d+$", "", base))  # L1：显式映射 + 安全回退
        process(v,
                os.path.join(here, f'{stem}-检测效果{args.suffix}.mp4'),
                os.path.join(here, f'{stem}-帧{{}}{args.suffix}.jpg'))


if __name__ == '__main__':
    main()
