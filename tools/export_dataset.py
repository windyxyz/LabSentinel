"""数据飞轮：把一轮标注数据打包为版本化数据集目录（v1/v2/...）。

职责（90-data-flywheel.md 工具清单）：
  1. 汇总图片+标签 → 目标版本目录 Resources/datasets/lab-ppe-real/v{N}/
  2. 生成 VERSION.md：来源清单（manifest.csv 汇总）、数量统计、许可与脱敏声明
  3. 脱敏检查（辅助）：提示待人工确认项（本工具不做自动人脸打码——脱敏必须人工过目）

用法:
    python tools/export_dataset.py --images Resources/datasets/frames --labels Resources/datasets/labels \
        --version 1 --source-manifest Resources/datasets/frames/manifest.csv
    python tools/export_dataset.py --help
"""
import argparse
import csv
import shutil
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IMG_EXTS = {".jpg", ".jpeg", ".png", ".bmp", ".webp"}


def count_classes(labels_dir: Path) -> dict[int, int]:
    counts: dict[int, int] = {}
    for f in labels_dir.glob("*.txt"):
        for ln in f.read_text(encoding="utf-8").splitlines():
            p = ln.split()
            if p:
                ci = int(p[0])
                counts[ci] = counts.get(ci, 0) + 1
    return counts


def main():
    ap = argparse.ArgumentParser(description="打包一轮标注数据为版本化数据集（含 VERSION.md）")
    ap.add_argument("--images", required=True, help="图片目录（含 manifest.csv 更佳）")
    ap.add_argument("--labels", required=True, help="YOLO 标签目录（与图片同名 .txt）")
    ap.add_argument("--version", type=int, required=True, help="版本号 N → 输出 v{N}/")
    ap.add_argument("--out-root", default=None,
                    help="版本根目录（默认 <项目>/datasets/lab-ppe-real）")
    ap.add_argument("--source-manifest", default=None,
                    help="来源清单 CSV（frame_sampler 的 manifest.csv，写入 VERSION.md）")
    ap.add_argument("--license", default="CC BY 4.0", help="本轮数据许可（默认 CC BY 4.0）")
    ap.add_argument("--note", default="", help="备注（机位/时段/采集说明）")
    args = ap.parse_args()

    img_dir, lbl_dir = Path(args.images), Path(args.labels)
    if not img_dir.is_dir() or not lbl_dir.is_dir():
        sys.exit(f"[error] 目录不存在: {img_dir} / {lbl_dir}")
    images = sorted(p for p in img_dir.iterdir() if p.suffix.lower() in IMG_EXTS)
    if not images:
        sys.exit(f"[error] 图片目录为空: {img_dir}")

    root = Path(args.out_root) if args.out_root else ROOT / "Resources" / "datasets" / "lab-ppe-real"
    vdir = root / f"v{args.version}"
    if vdir.exists():
        sys.exit(f"[error] 版本目录已存在（勿覆盖）: {vdir}")
    (vdir / "images").mkdir(parents=True)
    (vdir / "labels").mkdir()

    copied, missing_lbl, empty_lbl = 0, 0, 0
    for img in images:
        lbl = lbl_dir / (img.stem + ".txt")
        if not lbl.exists():
            missing_lbl += 1
            continue
        text = lbl.read_text(encoding="utf-8").strip()
        if not text:
            empty_lbl += 1  # 空标签（纯背景帧）也保留——负样本对训练有用
        shutil.copy2(img, vdir / "images" / img.name)
        shutil.copy2(lbl, vdir / "labels" / lbl.name)
        copied += 1

    # 来源清单（若有）
    src_rows = []
    manifest = Path(args.source_manifest) if args.source_manifest else img_dir / "manifest.csv"
    if manifest.exists():
        with open(manifest, encoding="utf-8") as f:
            src_rows = list(csv.DictReader(f))
    src_videos = sorted({r.get("source", "?") for r in src_rows}) if src_rows else []

    # 类别统计（映射到 data.yaml 的 names 若能找到）
    counts = count_classes(vdir / "labels")
    data_yaml = ROOT / "Resources" / "datasets" / "merged" / "data.yaml"
    names = {}
    if data_yaml.exists():
        import yaml
        names = {i: n for i, n in enumerate(
            yaml.safe_load(data_yaml.read_text(encoding="utf-8")).get("names", []))}

    stats_lines = [f"- {names.get(ci, ci)}: {n} 框" for ci, n in sorted(counts.items())]
    version_md = f"""# 数据集版本 v{args.version}

- 打包时间: {time.strftime('%Y-%m-%d %H:%M')}
- 图片数: {copied}（空标签/纯背景 {empty_lbl}，缺标签跳过 {missing_lbl}）
- 许可: {args.license}
- 备注: {args.note or '（无）'}

## 来源清单（{len(src_videos)} 个来源视频/目录）

""" + ("\n".join(f"- {s}" for s in src_videos) if src_videos else "- （未提供 manifest，请人工补充来源说明）") + f"""

## 类别框数统计

{chr(10).join(stats_lines) if stats_lines else "- （无标签框）"}

## 发布前脱敏声明（人工逐项确认后打勾）

- [ ] 已逐张检查并模糊/裁剪所有人脸
- [ ] 已移除画面中的可识别信息（工牌、屏幕内容等）
- [ ] 已取得画面中出现人员的授权（或素材本身允许发布）
- [ ] 来源与许可记录完整（本文件上方清单）

> 依据 90-data-flywheel.md：未完成脱敏确认前，本版本**不得**对社区发布。
"""
    (vdir / "VERSION.md").write_text(version_md, encoding="utf-8")
    print(f"[done] v{args.version}: {copied} 图 + 标签 → {vdir}")
    print(f"       缺标签跳过 {missing_lbl}，空标签(背景) {empty_lbl}")
    print("       VERSION.md 已生成（含脱敏确认清单，发布前必须人工勾完）")


if __name__ == "__main__":
    sys.exit(main())
