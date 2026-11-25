"""LS-056 离线单测：dataset_hub 登记表解析 + merge_datasets 清洗/去重/混合。

运行：
    python -m unittest discover -s tools/tests -v
"""
from __future__ import annotations

import json
import io
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import dataset_hub  # noqa: E402
import merge_datasets as md  # noqa: E402
from train_ppe import preflight_dataset, training_dataset_sources  # noqa: E402

from PIL import Image

jpeg_buf = io.BytesIO()
Image.new("RGB", (4, 4), "white").save(jpeg_buf, format="JPEG")
JPEG = jpeg_buf.getvalue()
PNG = b"\x89PNG\r\n\x1a\n" + b"0" * 64


class DatasetHubTests(unittest.TestCase):
    def test_registry_parses_and_has_required_fields(self):
        reg = dataset_hub.load_registry()
        self.assertIn("datasets", reg)
        self.assertIn("weights", reg)
        for e in reg["datasets"]:
            self.assertIn("name", e)
            self.assertIn("license", e)
            self.assertIn("type", e)

    def test_find_entry(self):
        reg = dataset_hub.load_registry()
        self.assertIsNotNone(dataset_hub.find_entry(reg, "lab-ppe"))
        self.assertIsNone(dataset_hub.find_entry(reg, "nope"))

    def test_huggingface_url_uses_mirror(self):
        e = {"name": "x", "type": "huggingface", "repo": "a/b", "file": "d.zip"}
        url = dataset_hub.resolve_url(e, hf_endpoint="https://hf-mirror.com")
        self.assertEqual(url, "https://hf-mirror.com/datasets/a/b/resolve/main/d.zip")

    def test_local_returns_none(self):
        e = {"name": "x", "type": "local", "path": "Resources/datasets/lab-ppe"}
        self.assertIsNone(dataset_hub.resolve_url(e))


class MergeDatasetTests(unittest.TestCase):
    def test_weak_validation_images_are_not_reused_for_training(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            for split in ("train", "val_weak"):
                path = root / "images" / split
                path.mkdir(parents=True)
                (path / "image.jpg").write_bytes(JPEG)
            self.assertEqual([sample.image.parent.name for sample in md.load_yolo_dir(root)], ["train"])

    def test_training_sources_follow_the_data_file(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            data = root / "data.yaml"
            data.write_text("train: images/train\n", encoding="utf-8")
            (root / "dataset-sources.json").write_text(
                json.dumps({"format_version": 1, "sources": [
                    {"name": "example", "source": "https://example.test/data",
                     "license": "CC BY-NC-SA 4.0", "sample_count": 2},
                ]}), encoding="utf-8",
            )
            manifest = training_dataset_sources(data, root)
            self.assertEqual(manifest["sources"][0]["license"], "CC BY-NC-SA 4.0")
            self.assertEqual(len(manifest["data_yaml_sha256"]), 64)

    def test_merge_records_source_of_retained_samples(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            for name in ("first", "duplicate"):
                self._make_dataset(root / name, ["lab coat", "person"])
                (root / name / "dataset-source.json").write_text(
                    json.dumps({"source": f"https://example.test/{name}", "license": "CC BY-NC-SA 4.0"}),
                    encoding="utf-8",
                )
            out = root / "merged"
            self.assertEqual(md.main([
                "--src", f"first={root / 'first'}",
                "--src", f"duplicate={root / 'duplicate'}",
                "--out", str(out),
            ]), 0)
            manifest = json.loads((out / "dataset-sources.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["sources"], [{
                "name": "first", "source": "https://example.test/first",
                "license": "CC BY-NC-SA 4.0", "sample_count": 1,
            }])

    def test_normalize_class_aliases(self):
        self.assertEqual(md.normalize_class("Lab Coat"), "lab_coat")
        self.assertEqual(md.normalize_class("no-lab-coat"), "no_lab_coat")
        self.assertEqual(md.normalize_class("no goggles"), "no_goggles")
        self.assertIsNone(md.normalize_class("unknown-thing"))

    def test_valid_box(self):
        self.assertTrue(md.valid_box(0.5, 0.5, 0.2, 0.2))
        self.assertFalse(md.valid_box(1.5, 0.5, 0.2, 0.2))
        self.assertFalse(md.valid_box(0.5, 0.5, 0.0, 0.2))

    def test_parse_label_line(self):
        self.assertEqual(md.parse_label_line("0 0.5 0.5 0.2 0.2"), (0, 0.5, 0.5, 0.2, 0.2))
        self.assertIsNone(md.parse_label_line("bad"))
        self.assertIsNone(md.parse_label_line("0 0.5 0.5 0.2"))

    def _make_dataset(self, root: Path, names: list[str]) -> None:
        (root / "images").mkdir(parents=True)
        (root / "labels").mkdir(parents=True)
        (root / "classes.txt").write_text("\n".join(names), encoding="utf-8")
        # 两张有效图 + 一张坏图
        for i, blob in enumerate([JPEG, JPEG, b"not-an-image"]):
            (root / "images" / f"img{i}.jpg").write_bytes(blob)
            (root / "labels" / f"img{i}.txt").write_text(
                "0 0.5 0.5 0.2 0.2\n1 2.0 0.5 0.1 0.1\n", encoding="utf-8"
            )

    def test_load_dedup_and_remap(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td) / "src"
            self._make_dataset(root, ["lab coat", "no lab coat"])
            samples = md.remap_class_ids(md.load_yolo_dir(root), ["lab coat", "no lab coat"])
            # 坏图被剔除 → 2 张；越界框被剔除 → 每张仅 1 行
            self.assertEqual(len(samples), 2)
            self.assertTrue(all(len(s.lines) == 1 for s in samples))
            # lab_coat → 标准索引 0
            self.assertTrue(samples[0].lines[0].startswith("0 "))
            # 同内容图片 MD5 去重 → 1 张
            deduped = md.dedupe(samples)
            self.assertEqual(len(deduped), 1)

    def test_mix_ratio(self):
        user = [md.Sample(Path(f"u{i}.jpg")) for i in range(2)]
        public = [md.Sample(Path(f"p{i}.jpg")) for i in range(10)]
        merged = md.mix(user, public, (1, 3))
        # 2 用户 → 取 6 公开
        self.assertEqual(len(merged), 8)

    def test_write_dataset_layout_and_yaml(self):
        with tempfile.TemporaryDirectory() as td:
            src = Path(td) / "src"
            self._make_dataset(src, ["lab coat", "no lab coat"])
            samples = md.remap_class_ids(md.load_yolo_dir(src), ["lab coat", "no lab coat"])
            out = Path(td) / "out"
            stats = md.write_dataset(samples, out)
            self.assertEqual(stats["total"], 2)
            self.assertTrue((out / "images" / "train").is_dir())
            self.assertTrue((out / "labels" / "train").is_dir())
            self.assertIn("names:", (out / "data.yaml").read_text(encoding="utf-8"))
            self.assertEqual(
                (out / "classes.txt").read_text(encoding="utf-8").splitlines(),
                md.CANONICAL,
            )

    def test_small_splits_skip_missing_images_and_preflight(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            samples = [md.Sample(root / f"source-{i}.jpg") for i in range(5)]
            for sample in samples:
                sample.image.write_bytes(JPEG)
            missing = md.Sample(root / "missing.jpg")
            for count in [1, 2, 4, 5]:
                with self.subTest(count=count):
                    out = root / f"output-{count}"
                    stats = md.write_dataset([missing] + samples[:count], out)
                    val = int(count > 1)
                    self.assertEqual(stats, {"total": count, "train": count - val, "val": val, "missing": 1})
                    data = out / "data.yaml"
                    if count == 1:
                        self.assertFalse(data.exists())
                        self.assertTrue((out / "README.txt").is_file())
                        with self.assertRaisesRegex(ValueError, "缺少可训练"):
                            preflight_dataset(data)
                        continue
                    self.assertEqual(preflight_dataset(data)[1], {"train": count - val, "val": val})
                    for img in (out / "images" / "val").iterdir():
                        img.unlink()
                    with self.assertRaisesRegex(ValueError, "val 没有图片"):
                        preflight_dataset(data)

    def test_preflight_rejects_missing_image_list_entry(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "present.jpg").write_bytes(JPEG)
            (root / "val.txt").write_text("./missing.jpg\n", encoding="utf-8")
            yaml_path = root / "data.yaml"
            yaml_path.write_text("path: .\ntrain: present.jpg\nval: [val.txt]\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "val 清单中缺少图片"):
                preflight_dataset(yaml_path)

    def test_user_zip_extract(self):
        import zipfile

        with tempfile.TemporaryDirectory() as td:
            z = Path(td) / "u.zip"
            with zipfile.ZipFile(z, "w") as zf:
                zf.writestr("classes.txt", "lab_coat\nperson\n")
                zf.writestr("images/a.jpg", JPEG)
                zf.writestr("labels/a.txt", "0 0.5 0.5 0.2 0.2\n")
            out = Path(td) / "extract"
            md.extract_user_zip(z, out)
            self.assertTrue((out / "classes.txt").is_file())
            self.assertTrue((out / "images" / "a.jpg").is_file())


if __name__ == "__main__":
    unittest.main(verbosity=2)
