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
import hashlib
import json
import os
import shutil
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_MODEL = ROOT / "m0-spike" / "yolo11n.pt"
DEFAULT_DATA = ROOT / "Resources" / "datasets" / "lab-ppe" / "data.yaml"
BEST_COPY_DST = ROOT / "m0-spike" / "yolo11n_ppe.pt"
IMAGE_TYPES = {".avif", ".bmp", ".dng", ".heic", ".heif", ".jp2", ".jpeg",
               ".jpg", ".mpo", ".png", ".tif", ".tiff", ".webp"}


def dataset_root(yaml_path: Path, configured: str | None) -> Path:
    if not configured or configured == ".":
        return yaml_path.parent
    path = Path(configured).expanduser()
    if path.is_absolute() or path.is_dir():
        return path.resolve()
    default_config = (Path(os.environ["APPDATA"]) if "APPDATA" in os.environ else
                      Path.home() / ("Library/Application Support" if sys.platform == "darwin" else ".config"))
    config_dir = Path(os.environ.get("YOLO_CONFIG_DIR") or default_config / "Ultralytics")
    try:
        value = json.loads((config_dir / "settings.json").read_text(encoding="utf-8"))
        if isinstance(value.get("datasets_dir"), str):
            root = Path(value["datasets_dir"]) / path
            if root.is_dir():
                return root.resolve()
    except (OSError, ValueError, AttributeError):
        pass
    return (yaml_path.parent / path).resolve()


def training_dataset_sources(yaml_path: Path, root: Path) -> dict:
    from merge_datasets import source_record
    from dataset_hub import load_registry

    manifest = root / "dataset-sources.json"
    if manifest.is_file():
        recorded = json.loads(manifest.read_text(encoding="utf-8"))
        if not isinstance(recorded, dict) or not isinstance(recorded.get("sources"), list):
            raise ValueError("dataset-sources.json 缺少来源列表")
        sources = recorded["sources"]
    else:
        source = source_record(root, root.name, load_registry())
        source.pop("sample_count")
        sources = [source]
    return {
        "format_version": 1,
        "data_yaml_sha256": hashlib.sha256(yaml_path.read_bytes()).hexdigest(),
        "sources": sources,
    }


def preflight_dataset(yaml_path: Path) -> tuple[dict, dict[str, int]]:
    import yaml
    from PIL import Image

    yaml_path = yaml_path.resolve()
    if not yaml_path.is_file():
        if (yaml_path.parent / "classes.txt").is_file():
            raise ValueError("缺少可训练的 data.yaml，请补充或合并样本，确保训练集和验证集各有图片")
        raise ValueError(f"找不到数据集配置：{yaml_path}")
    try:
        data = yaml.safe_load(yaml_path.read_text(encoding="utf-8-sig"))
    except yaml.YAMLError as exc:
        raise ValueError(f"data.yaml 格式有误：{exc}") from exc
    if not isinstance(data, dict):
        raise ValueError("data.yaml 必须包含 train 和 val 配置")
    configured = data.get("path")
    if configured is not None and not isinstance(configured, str):
        raise ValueError("data.yaml 的 path 必须是目录路径")
    root = dataset_root(yaml_path, configured)
    data["path"] = str(root)
    counts = {}
    for split in ("train", "val"):
        value = data.get(split)
        sources = [value] if isinstance(value, str) else value
        if not isinstance(sources, list) or not sources or not all(isinstance(s, str) and s for s in sources):
            raise ValueError(f"{split} 缺少图片目录或清单")
        resolved = []
        images = set()
        for source in sources:
            path = (root / source).resolve()
            if not path.exists() and isinstance(value, str) and source.startswith("../"):
                path = (root / source[3:]).resolve()
            if path.is_dir():
                images.update(p.resolve() for p in path.rglob("*") if p.is_file() and p.suffix.lower() in IMAGE_TYPES)
            elif path.is_file() and path.suffix.lower() == ".txt":
                for line in path.read_text(encoding="utf-8").splitlines():
                    if line.strip():
                        images.add((path.parent / line[2:]).resolve() if line.startswith("./") else Path(line).resolve())
            elif path.is_file() and path.suffix.lower() in IMAGE_TYPES:
                images.add(path)
            else:
                raise ValueError(f"{split} 找不到图片源：{path}")
            resolved.append(str(path))
        if not images:
            raise ValueError(f"{split} 没有图片，请补充样本后再训练")
        for path in sorted(images):
            try:
                with Image.open(path) as img:
                    img.verify()
            except FileNotFoundError as exc:
                raise ValueError(f"{split} 清单中缺少图片：{path}") from exc
            except (OSError, ValueError, Image.DecompressionBombError) as exc:
                raise ValueError(f"{split} 图片不可读：{path}") from exc
        data[split] = resolved[0] if isinstance(value, str) else resolved
        counts[split] = len(images)
    return data, counts


def parse_args():
    ap = argparse.ArgumentParser(
        description="LAB PPE 微调训练（YOLO11n 起点），训练后把 best.pt 复制到 m0-spike/yolo11n_ppe.pt"
    )
    ap.add_argument("--model", default=str(DEFAULT_MODEL),
                    help=f"起点权重（默认 {DEFAULT_MODEL}，COCO 预训练 yolo11n）")
    ap.add_argument("--data", default=str(DEFAULT_DATA),
                    help=f"数据集 data.yaml（默认 {DEFAULT_DATA}）")
    ap.add_argument("--preflight", action="store_true", help="只检查训练集和验证集的文件并显示图片数")
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
    # LS-056：多架构 / 数据飞轮混合 / 灰度增强 / 线程限流
    ap.add_argument("--arch", default="yolo11n",
                    choices=["yolo11n", "yolo11s", "yolo12n", "yolo12s", "rtdetr-l"],
                    help="模型架构（默认 yolo11n；非 yolo11n 时自动用 ultralytics 下载预训练权重）")
    ap.add_argument("--user-zip", default="",
                    help="网页端导出的 YOLO 训练集 ZIP（与公开数据混合）")
    ap.add_argument("--mix-public", default="1:3",
                    help="user:public 混合比例（默认 1:3）")
    ap.add_argument("--public-src", action="append", default=[],
                    help="公开数据集 name=path（可多次；与 --user-zip 一起使用）")
    ap.add_argument("--threads", type=int, default=4,
                    help="torch 线程上限（默认 4，避免训练拖垮本机）")
    ap.add_argument("--aug", default="lab",
                    choices=["lab", "none"],
                    help="增强预设：lab=灰度/低光/远景增强（默认），none=ultralytics 默认")
    return ap.parse_args()


def main():
    args = parse_args()

    model_path = Path(args.model)
    data_path = Path(args.data)
    # LS-056：--user-zip 与公开数据混合（复用 merge_datasets 清洗/去重/比例）
    if args.user_zip:
        import subprocess
        if not args.public_src and data_path.is_file():
            args.public_src = [f"default={data_path.parent}"]
        mixed_dir = tempfile.TemporaryDirectory(prefix="labsentinel-mix-")
        merged = Path(mixed_dir.name) / "mixed"
        cmd = [sys.executable, str(ROOT / "tools" / "merge_datasets.py"),
               "--user-zip", args.user_zip, "--mix-ratio", args.mix_public,
               "--out", str(merged)]
        for spec in args.public_src:
            cmd += ["--src", spec]
        print(f"[info] 混合数据集: {' '.join(cmd)}")
        rc = subprocess.run(cmd, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0)).returncode
        if rc != 0:
            sys.exit(f"[error] merge_datasets 失败 rc={rc}")
        data_path = merged / "data.yaml"

    try:
        dataset, counts = preflight_dataset(data_path)
        sources = training_dataset_sources(data_path, Path(dataset["path"]))
    except (OSError, ValueError, ImportError) as exc:
        sys.exit(f"[error] 训练预检失败：{exc}")
    print(f"[ok] 训练集 {counts['train']} 张，验证集 {counts['val']} 张")
    print(f"[info] 数据来源: {json.dumps(sources, ensure_ascii=False)}")
    if args.preflight:
        return
    if args.arch == "yolo11n" and not model_path.exists():
        sys.exit(f"[error] 起点权重不存在: {model_path}（可从 GitHub 镜像下载 yolo11n.pt）")

    import torch
    try:
        torch.set_num_threads(max(1, args.threads))
        print(f"[info] torch 线程上限 = {args.threads}")
    except Exception:
        pass

    from ultralytics import RTDETR, YOLO

    print(f"[info] 起点: {model_path.name}  数据: {data_path}")
    print(f"[info] epochs={args.epochs} patience={args.patience} imgsz={args.imgsz} "
          f"batch={args.batch} device={args.device or 'auto'} -> {args.project}/{args.name}")
    # 架构选择：默认沿用 --model；指定非 yolo11n 时用 ultralytics 预训练权重
    weights = str(model_path) if args.arch == "yolo11n" else f"{args.arch}.pt"
    model = RTDETR(weights) if args.arch == "rtdetr-l" else YOLO(weights)
    aug = {}
    if args.aug == "lab":
        # 实验室灰度/低光/远景增强预设（albumentations 为可选依赖，缺失时降级为内置增强）
        aug = dict(scale=0.9, hsv_v=0.5, hsv_s=0.5, degrees=5.0,
                   mosaic=1.0, close_mosaic=10)
        try:
            from albumentations import RandomBrightnessContrast, ToGray
            aug["augmentations"] = [
                ToGray(p=0.25),
                RandomBrightnessContrast(brightness_limit=(-0.45, 0.10), contrast_limit=0.2, p=0.35),
            ]
        except ImportError:
            print("[warn] 未安装 albumentations，降级使用 Ultralytics 内置 HSV/scale/mosaic 增强")
    import yaml
    with tempfile.TemporaryDirectory(prefix="labsentinel-train-") as temp:
        checked_yaml = Path(temp) / data_path.name
        checked_yaml.write_text(yaml.safe_dump(dataset, allow_unicode=True), encoding="utf-8")
        model.train(
            data=str(checked_yaml),
            epochs=args.epochs,
            patience=args.patience,
            imgsz=args.imgsz,
            batch=args.batch,
            device=args.device,
            workers=args.workers,
            project=args.project,
            name=args.name,
            **aug,
            exist_ok=args.exist_ok,
        )

    # 定位 best.pt：优先 trainer 记录的路径，回退到 project/name 推导
    save_dir = getattr(getattr(model, "trainer", None), "save_dir", None)
    best = Path(save_dir) / "weights" / "best.pt" if save_dir else None
    if best is None or not best.exists():
        best = Path(args.project) / args.name / "weights" / "best.pt"
    if not best.exists():
        sys.exit(f"[error] 未找到 best.pt（预期位置 {best}），请检查训练输出目录。")

    copy_dst = BEST_COPY_DST.parent / f"{args.arch}_ppe.pt"
    copy_dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(best, copy_dst)
    source_record = json.dumps(sources, ensure_ascii=False, indent=2) + "\n"
    (best.parent.parent / "dataset-sources.json").write_text(source_record, encoding="utf-8")
    copy_dst.with_suffix(".sources.json").write_text(source_record, encoding="utf-8")
    print(f"[done] 训练完成，结果目录: {best.parents[2]}")
    print(f"[done] best.pt 已复制: {best} -> {copy_dst}")
    print(f"[next] 运行 tools/export_onnx.py 导出 ONNX（--weights {copy_dst}）")


if __name__ == "__main__":
    main()
