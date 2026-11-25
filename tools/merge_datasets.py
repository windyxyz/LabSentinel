"""LS-056：多源 YOLO 数据集统一清洗与合并（开发支具，不进发行包）。

能力：
- 统一类别映射到标准词表（lab_coat / no_lab_coat / gloves / no_gloves /
  goggles / no_goggles / mask / person），支持 --map 自定义 JSON；
- 校验图片可读性（JPEG/PNG 头）、剔除坏图与越界/退化框；
- 按图片 MD5 去重；
- 支持 --user-zip（网页端导出的 YOLO ZIP）与公开数据集按 --mix-ratio 1:3 混合；
- 输出标准 YOLO 目录（images/train|val、labels/train|val）+ data.yaml + classes.txt。

用法：
    python tools/merge_datasets.py --src A=data/a --src B=data/b --out Resources/datasets/merged
    python tools/merge_datasets.py --src pub=data/pub --user-zip user.zip --mix-ratio 1:3 --out merged
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import sys
import zipfile
from dataclasses import dataclass, field
from pathlib import Path

from dataset_hub import find_entry, load_registry

ROOT = Path(__file__).resolve().parent.parent

# 标准词表（与 Rust yolo_label_line / tools/export_onnx.py 对齐）
CANONICAL = [
    "lab_coat",
    "no_lab_coat",
    "gloves",
    "no_gloves",
    "goggles",
    "no_goggles",
    "mask",
    "person",
    "no_mask",
]

# 常见别名 → 标准词
DEFAULT_MAP = {
    "lab coat": "lab_coat",
    "labcoat": "lab_coat",
    "coat": "lab_coat",
    "no lab coat": "no_lab_coat",
    "no_labcoat": "no_lab_coat",
    "no coat": "no_lab_coat",
    "glove": "gloves",
    "gloves": "gloves",
    "no glove": "no_gloves",
    "no gloves": "no_gloves",
    "goggle": "goggles",
    "goggles": "goggles",
    "no goggle": "no_goggles",
    "no goggles": "no_goggles",
    "mask": "mask",
    "face mask": "mask",
    "no mask": "no_mask",
    "without mask": "no_mask",
    "person": "person",
    "people": "person",
    "worker": "person",
}


def normalize_class(name: str, mapping: dict[str, str] | None = None) -> str | None:
    """别名 → 标准词；跳过项目词表之外的类别。"""
    key = name.strip().lower().replace("-", "_")
    custom = {k.strip().lower(): v for k, v in (mapping or {}).items()}
    table = {**DEFAULT_MAP, **custom}
    # 先按原样、再按下划线/空格归一化查
    for k in (name.strip().lower(), key, key.replace("_", " ")):
        if k in table:
            return table[k]
    if key in CANONICAL:
        return key
    return None


def parse_label_line(line: str) -> tuple[int, float, float, float, float] | None:
    parts = line.split()
    if len(parts) != 5:
        return None
    try:
        cls = int(float(parts[0]))
        cx, cy, w, h = (float(x) for x in parts[1:])
    except ValueError:
        return None
    return cls, cx, cy, w, h


def valid_box(cx: float, cy: float, w: float, h: float) -> bool:
    """归一化框合法性：中心/宽高在 0..1 且面积>0（容忍微小越界）。"""
    eps = 1e-3
    if not (-eps <= cx <= 1 + eps and -eps <= cy <= 1 + eps):
        return False
    if not (0 < w <= 1 + eps and 0 < h <= 1 + eps):
        return False
    return (cx - w / 2 >= -eps and cy - h / 2 >= -eps
            and cx + w / 2 <= 1 + eps and cy + h / 2 <= 1 + eps)


def is_readable_image(path: Path) -> bool:
    """解码整个图像，剔除首部正常但像素损坏的样本。"""
    from PIL import Image
    try:
        with Image.open(path) as img:
            img.load()
    except (OSError, ValueError, Image.DecompressionBombError):
        return False
    return True


def md5_file(path: Path) -> str:
    h = hashlib.md5()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


@dataclass
class Sample:
    image: Path
    lines: list[str] = field(default_factory=list)


def load_yolo_dir(root: Path, mapping: dict[str, str] | None = None) -> list[Sample]:
    """读取一个 YOLO 数据集目录：优先 images/+labels/，其次平铺同目录。"""
    out: list[Sample] = []
    pairs: list[tuple[Path, Path | None]] = []
    images_dir = root / "images"
    labels_dir = root / "labels"
    if images_dir.is_dir():
        for img in sorted(images_dir.rglob("*")):
            if img.suffix.lower() not in (".jpg", ".jpeg", ".png", ".bmp"):
                continue
            rel = img.relative_to(images_dir)
            if rel.parts[0] == "val_weak":
                continue
            lab = labels_dir / rel.with_suffix(".txt")
            pairs.append((img, lab if lab.is_file() else None))
    else:
        splits = [root / split for split in ("train", "val", "valid", "test") if (root / split / "images").is_dir()]
        if splits:
            return [sample for split in splits for sample in load_yolo_dir(split)]
        pairs = [(img, img.with_suffix(".txt")) for img in sorted(root.glob("*"))
                 if img.suffix.lower() in (".jpg", ".jpeg", ".png", ".bmp")]
    for img, lab in pairs:
        if not is_readable_image(img):
            continue
        s = Sample(image=img)
        if lab and lab.is_file():
            for line in lab.read_text(encoding="utf-8", errors="replace").splitlines():
                parsed = parse_label_line(line)
                if not parsed:
                    continue
                cls, cx, cy, w, h = parsed
                if not valid_box(cx, cy, w, h):
                    continue
                # 类别索引按源数据集 classes.txt（若存在）解析为名字再映射
                s.lines.append(f"{cls} {cx:.6f} {cy:.6f} {w:.6f} {h:.6f}")
        out.append(s)
    return out


def remap_class_ids(samples: list[Sample], class_names: list[str], mapping: dict[str, str] | None = None) -> list[Sample]:
    """把源类别索引 → 标准词表索引；未知/缺失类回退 person。"""
    std_index = {name: i for i, name in enumerate(CANONICAL)}
    for s in samples:
        new_lines: list[str] = []
        for line in s.lines:
            parts = line.split()
            try:
                src = int(float(parts[0]))
            except (IndexError, ValueError):
                continue
            if not 0 <= src < len(class_names):
                continue
            name = normalize_class(class_names[src], mapping)
            if name in std_index:
                new_lines.append(" ".join([str(std_index[name])] + parts[1:]))
        s.lines = new_lines
    return samples


def dedupe(samples: list[Sample]) -> list[Sample]:
    seen: set[str] = set()
    out: list[Sample] = []
    for s in samples:
        try:
            h = md5_file(s.image)
        except OSError:
            continue
        if h in seen:
            continue
        seen.add(h)
        out.append(s)
    return out


def mix(user: list[Sample], public: list[Sample], ratio: tuple[int, int]) -> list[Sample]:
    """按 user:public 比例混合（如 1:3）；不足时按可用数量取。"""
    u, p = ratio
    if u <= 0 or p <= 0:
        return user + public
    want_u = len(user)
    want_p = int(want_u * p / u) if want_u else len(public)
    return user + public[: max(0, min(want_p, len(public)))]


def write_dataset(samples: list[Sample], out: Path, val_ratio: float = 0.2) -> dict:
    """写 YOLO 样本目录。有独立验证集时才写训练配置。"""
    available = [s for s in samples if s.image.is_file()]
    if not available:
        raise ValueError("没有可用图片，请检查样本文件")
    if not 0 < val_ratio < 1:
        raise ValueError("验证集比例须介于 0 与 1 之间")
    n_train = n_val = 0
    stride = max(2, round(1 / val_ratio))
    for i, s in enumerate(available):
        split = "val" if len(available) > 1 and i % stride == 0 else "train"
        stem = f"{i:06d}_{s.image.stem}"
        dst_img = out / "images" / split / (stem + s.image.suffix)
        dst_lab = out / "labels" / split / (stem + ".txt")
        dst_img.parent.mkdir(parents=True, exist_ok=True)
        dst_lab.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(s.image, dst_img)
        dst_lab.write_text("\n".join(s.lines), encoding="utf-8")
        if split == "val":
            n_val += 1
        else:
            n_train += 1
    (out / "classes.txt").write_text("\n".join(CANONICAL), encoding="utf-8")
    if n_val:
        (out / "data.yaml").write_text(
            "train: images/train\nval: images/val\nnames:\n"
            + "\n".join(f"  {i}: {c}" for i, c in enumerate(CANONICAL)) + "\n",
            encoding="utf-8",
        )
    else:
        (out / "README.txt").write_text(
            "只有 1 张可用图片。继续补充或合并样本，生成独立的训练集和验证集后再训练。\n",
            encoding="utf-8",
        )
    return {"total": len(available), "train": n_train, "val": n_val,
            "missing": len(samples) - len(available)}


def extract_user_zip(zip_path: Path, out_dir: Path) -> Path:
    """安全解压用户标注 ZIP（防 Zip-Slip 路径穿越与符号链接逃逸）。"""
    out_dir.mkdir(parents=True, exist_ok=True)
    root = out_dir.resolve()
    with zipfile.ZipFile(zip_path) as z:
        for info in z.infolist():
            # 拒绝符号链接（Unix mode 0o120000）
            if (info.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError(f"ZIP 含符号链接，已拒绝: {info.filename}")
            rel = Path(info.filename)
            if rel.is_absolute() or rel.drive or ".." in rel.parts:
                raise ValueError(f"ZIP 含非法路径，已拒绝: {info.filename}")
            target = (root / rel).resolve()
            if not target.is_relative_to(root):
                raise ValueError(f"ZIP 路径越出目标目录，已拒绝: {info.filename}")
            if info.is_dir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with z.open(info) as src, open(target, "wb") as dst:
                    shutil.copyfileobj(src, dst)
    return out_dir


def class_names(root: Path) -> list[str]:
    classes = root / "classes.txt"
    if classes.is_file():
        return classes.read_text(encoding="utf-8-sig").splitlines()
    yaml_file = next(iter(sorted(root.glob("*.yaml"))), None)
    if yaml_file is None:
        raise ValueError(f"{root}: 缺少 classes.txt 或 YOLO YAML 类表")
    import yaml
    names = yaml.safe_load(yaml_file.read_text(encoding="utf-8-sig"))["names"]
    if isinstance(names, dict):
        int_map = {int(k): str(v) for k, v in names.items()}
        return [int_map.get(i, "") for i in range(max(int_map, default=-1) + 1)]
    return list(names)


def source_record(root: Path, name: str, registry: dict) -> dict:
    sidecar = root / "dataset-source.json"
    if sidecar.is_file():
        metadata = json.loads(sidecar.read_text(encoding="utf-8-sig"))
        if not isinstance(metadata, dict) or not all(
            isinstance(metadata.get(key), str) and metadata[key].strip()
            for key in ("source", "license")
        ):
            raise ValueError(f"{sidecar}: source 和 license 必须是非空文字")
    else:
        entry = find_entry({"datasets": registry["datasets"]}, name) or {}
        metadata = {"source": entry.get("source") or entry.get("url"), "license": entry.get("license")}
    return {"name": name, "source": metadata["source"], "license": metadata["license"], "sample_count": 0}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="多源 YOLO 数据集清洗与合并")
    ap.add_argument("--src", action="append", default=[], help="name=path（可多次）")
    ap.add_argument("--user-zip", help="网页端导出的 YOLO ZIP")
    ap.add_argument("--mix-ratio", default="1:3", help="user:public 比例（默认 1:3）")
    ap.add_argument("--map", help="自定义类别映射 JSON 文件")
    ap.add_argument("--out", required=True)
    args = ap.parse_args(argv)

    mapping = json.loads(Path(args.map).read_text(encoding="utf-8")) if args.map else None
    out = Path(args.out)
    if (out / "data.yaml").exists() or (out / "classes.txt").exists():
        ap.error("--out 已包含训练集，请指定新的输出目录")
    all_public: list[Sample] = []
    registry = load_registry()
    sources: list[dict] = []
    sample_sources: dict[int, dict] = {}
    for spec in args.src:
        if "=" not in spec:
            print(f"[跳过] --src 需 name=path: {spec}", file=sys.stderr)
            continue
        name, path = spec.split("=", 1)
        root = Path(path)
        source = source_record(root, name, registry)
        names = class_names(root)
        samples = remap_class_ids(load_yolo_dir(root), names, mapping)
        print(f"[load] {root}: {len(samples)} 张")
        all_public.extend(samples)
        sources.append(source)
        sample_sources.update((id(sample), source) for sample in samples)
    import tempfile
    user_samples: list[Sample] = []
    tmp_ctx = tempfile.TemporaryDirectory(prefix="ls_user_zip_") if args.user_zip else None
    try:
        if tmp_ctx is not None:
            tmp = Path(tmp_ctx.name)
            extract_user_zip(Path(args.user_zip), tmp)
            try:
                names = class_names(tmp)
            except ValueError:
                names = CANONICAL
            user_samples = remap_class_ids(load_yolo_dir(tmp), names, mapping)
            source = {"name": "user-zip", "source": None, "license": None, "sample_count": 0}
            sources.append(source)
            sample_sources.update((id(sample), source) for sample in user_samples)
            print(f"[load] user-zip: {len(user_samples)} 张")
        try:
            u, p = (int(x) for x in args.mix_ratio.split(":"))
        except ValueError:
            u, p = 1, 3
        merged = [sample for sample in dedupe(mix(user_samples, dedupe(all_public), (u, p)))
                  if sample.image.is_file()]
        if not merged:
            print("[失败] 没有可用样本", file=sys.stderr)
            return 2
        try:
            stats = write_dataset(merged, out)
            for sample in merged:
                sample_sources[id(sample)]["sample_count"] += 1
            manifest = {"format_version": 1, "sources": [source for source in sources if source["sample_count"]]}
            (out / "dataset-sources.json").write_text(
                json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8",
            )
        except (OSError, ValueError) as exc:
            print(f"[失败] {exc}", file=sys.stderr)
            return 2
        print(f"[ok] {out}: {stats}")
    finally:
        if tmp_ctx is not None:
            tmp_ctx.cleanup()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
