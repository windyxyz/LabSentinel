"""LAB PPE 微调训练脚本（ultralytics YOLO11n 起点）。

流程: m0-spike/yolo11n.pt（COCO 预训练）+ datasets/lab-ppe/data.yaml
      -> runs/detect/<name>/ 训练产物 -> best.pt 复制为 m0-spike/yolo11n_ppe.pt

用法（m0-venv）:
    D:\\LabSentinel\\m0-venv\\Scripts\\python.exe tools/train_ppe.py                      # 正式训练（100 epochs）
    D:\\LabSentinel\\m0-venv\\Scripts\\python.exe tools/train_ppe.py --epochs 1 --name runs/ppe_smoke   # 冒烟
    D:\\LabSentinel\\m0-venv\\Scripts\\python.exe tools/train_ppe.py --help               # 全部参数

数据集先用 tools/download_datasets.py 下载。训练完成后再跑 tools/export_onnx.py 导出 ONNX。
"""
import argparse
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_MODEL = ROOT / "m0-spike" / "yolo11n.pt"
DEFAULT_DATA = ROOT / "Resources" / "datasets" / "lab-ppe" / "data.yaml"
BEST_COPY_DST = ROOT / "m0-spike" / "yolo11n_ppe.pt"


def parse_args():
    ap = argparse.ArgumentParser(
        description="LAB PPE 微调训练（YOLO11n 起点），训练后把 best.pt 复制到 m0-spike/yolo11n_ppe.pt"
    )
    ap.add_argument("--model", default=str(DEFAULT_MODEL),
                    help=f"起点权重（默认 {DEFAULT_MODEL}，COCO 预训练 yolo11n）")
    ap.add_argument("--data", default=str(DEFAULT_DATA),
                    help=f"数据集 data.yaml（默认 {DEFAULT_DATA}）")
    ap.add_argument("--epochs", type=int, default=100,
                    help="训练轮数（默认 100；冒烟验证用 --epochs 1）")
    ap.add_argument("--patience", type=int, default=50,
                    help="早停耐心值：连续 N 个 epoch 无提升即停（默认 50）")
    ap.add_argument("--imgsz", type=int, default=640, help="训练输入尺寸（默认 640）")
    ap.add_argument("--batch", type=int, default=16, help="batch size（默认 16）")
    ap.add_argument("--device", default=None,
                    help="训练设备：默认 auto（本机 CPU）；GPU 训练用 dml（需 gpu-venv，见 tools/README 第八节）")
    ap.add_argument("--workers", type=int, default=4, help="DataLoader 进程数（默认 4）")
    ap.add_argument("--project", default=str(ROOT / "Resources" / "runs" / "detect"),
                    help="结果根目录（默认 <项目>/Resources/runs/detect，与历史 ppe_v1 并列）")
    ap.add_argument("--name", default="runs/ppe_v2",
                    help="本次训练结果子目录名（默认 runs/ppe_v2，位于 Resources/runs/detect 下；勿与已有目录重名，或加 --exist-ok）")
    ap.add_argument("--exist-ok", action="store_true",
                    help="允许覆盖已存在的同名结果目录（默认不允许）")
    return ap.parse_args()


def main():
    args = parse_args()

    model_path = Path(args.model)
    data_path = Path(args.data)
    if not model_path.exists():
        sys.exit(f"[error] 起点权重不存在: {model_path}（可从 GitHub 镜像下载 yolo11n.pt）")
    if not data_path.exists():
        sys.exit(f"[error] data.yaml 不存在: {data_path}（先运行 tools/download_datasets.py 下载数据集）")

    from ultralytics import YOLO

    print(f"[info] 起点: {model_path.name}  数据: {data_path}")
    print(f"[info] epochs={args.epochs} patience={args.patience} imgsz={args.imgsz} "
          f"batch={args.batch} device={args.device or 'auto'} -> {args.project}/{args.name}")
    model = YOLO(str(model_path))
    model.train(
        data=str(data_path),
        epochs=args.epochs,
        patience=args.patience,
        imgsz=args.imgsz,
        batch=args.batch,
        device=args.device,
        workers=args.workers,
        project=args.project,
        name=args.name,
        exist_ok=args.exist_ok,
    )

    # 定位 best.pt：优先 trainer 记录的路径，回退到 project/name 推导
    save_dir = getattr(getattr(model, "trainer", None), "save_dir", None)
    best = Path(save_dir) / "weights" / "best.pt" if save_dir else None
    if best is None or not best.exists():
        best = Path(args.project) / args.name / "weights" / "best.pt"
    if not best.exists():
        sys.exit(f"[error] 未找到 best.pt（预期位置 {best}），请检查训练输出目录。")

    shutil.copy2(best, BEST_COPY_DST)
    print(f"[done] 训练完成，结果目录: {best.parents[2]}")
    print(f"[done] best.pt 已复制: {best} -> {BEST_COPY_DST}")
    print(f"[next] 运行 tools/export_onnx.py 导出 ONNX（--weights {BEST_COPY_DST}）")


if __name__ == "__main__":
    main()
