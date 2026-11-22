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
    "person": "person",
    "people": "person",
    "worker": "person",
}


def normalize_class(name: str, mapping: dict[str, str] | None = None) -> str:
    """别名 → 标准词；未知类回退 person（避免训练集出现非法类）。"""
    key = name.strip().lower().replace("-", "_")
    table = {**DEFAULT_MAP, **(mapping or {})}
    # 先按原样、再按下划线/空格归一化查
    for k in (name.strip().lower(), key, key.replace("_", " ")):
        if k in table:
            return table[k]
    if key in CANONICAL:
        return key
    return "person"


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
    return True


def is_readable_image(path: Path) -> bool:
    """按文件头判断 JPEG/PNG（避免引入 Pillow 依赖）。"""
    try:
        with path.open("rb") as f:
            head = f.read(8)
    except OSError:
        return False
    return head.startswith(b"\xff\xd8") or head.startswith(b"\x89PNG")


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
            lab = labels_dir / rel.with_suffix(".txt")
            pairs.append((img, lab if lab.is_file() else None))
    else:
        for img in sorted(root.glob("*")):
            if img.suffix.lower() in (".jpg", ".jpeg", ".png", ".bmp"):
                pairs.append((img, img.with_suffix(".txt")))
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
            name = class_names[src] if 0 <= src < len(class_names) else "person"
            dst = std_index[normalize_class(name, mapping)]
            new_lines.append(" ".join([str(dst)] + parts[1:]))
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
    """写标准 YOLO 目录（8:2），返回统计。"""
    for sub in ("images/train", "images/val", "labels/train", "labels/val"):
        (out / sub).mkdir(parents=True, exist_ok=True)
    n_train = n_val = 0
    for i, s in enumerate(samples):
        split = "val" if i % max(1, round(1 / val_ratio)) == 0 else "train"
        dst_img = out / "images" / split / s.image.name
        dst_lab = out / "labels" / split / (s.image.stem + ".txt")
        shutil.copy2(s.image, dst_img)
        dst_lab.write_text("\n".join(s.lines), encoding="utf-8")
        if split == "val":
            n_val += 1
        else:
            n_train += 1
    (out / "classes.txt").write_text("\n".join(CANONICAL), encoding="utf-8")
    (out / "data.yaml").write_text(
        "path: .\ntrain: images/train\nval: images/val\nnames:\n"
        + "\n".join(f"  {i}: {c}" for i, c in enumerate(CANONICAL))
        + "\n",
        encoding="utf-8",
    )
    return {"total": len(samples), "train": n_train, "val": n_val}


def extract_user_zip(zip_path: Path, out_dir: Path) -> Path:
    out_dir.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(zip_path) as z:
        z.extractall(out_dir)
    return out_dir


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
    all_public: list[Sample] = []
    for spec in args.src:
        if "=" not in spec:
            print(f"[跳过] --src 需 name=path: {spec}", file=sys.stderr)
            continue
        _, path = spec.split("=", 1)
        root = Path(path)
        classes_file = root / "classes.txt"
        names = (
            classes_file.read_text(encoding="utf-8").splitlines()
            if classes_file.is_file()
            else CANONICAL
        )
        samples = remap_class_ids(load_yolo_dir(root), names, mapping)
        print(f"[load] {root}: {len(samples)} 张")
        all_public.extend(samples)
    user_samples: list[Sample] = []
    if args.user_zip:
        tmp = out.parent / "_user_zip"
        extract_user_zip(Path(args.user_zip), tmp)
        classes_file = tmp / "classes.txt"
        names = classes_file.read_text(encoding="utf-8").splitlines() if classes_file.is_file() else CANONICAL
        user_samples = remap_class_ids(load_yolo_dir(tmp), names, mapping)
        print(f"[load] user-zip: {len(user_samples)} 张")
    try:
        u, p = (int(x) for x in args.mix_ratio.split(":"))
    except ValueError:
        u, p = 1, 3
    merged = dedupe(mix(user_samples, dedupe(all_public), (u, p)))
    if not merged:
        print("[失败] 没有可用样本", file=sys.stderr)
        return 2
    stats = write_dataset(merged, out)
    print(f"[ok] {out}: {stats}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
