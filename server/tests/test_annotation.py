"""单元测试：标注工具链的非模型部分（manifest / 文件名规则 / 空目录）。运行: pytest tests/ -q"""
import csv
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "tools"))

import numpy as np  # noqa: E402

import frame_sampler  # noqa: E402
import preannotate  # noqa: E402


def _make_video(path, frames=60, fps=20):
    """用 cv2 写一个测试视频（内容随时间变化，避免帧差去重全丢）。"""
    import cv2
    vw = cv2.VideoWriter(str(path), cv2.VideoWriter_fourcc(*"mp4v"), fps, (320, 180))
    for i in range(frames):
        img = np.zeros((180, 320, 3), np.uint8)
        cv2.rectangle(img, (i * 3, 40), (i * 3 + 40, 140), (255, 255, 255), -1)
        vw.write(img)
    vw.release()


def test_sampler_manifest_and_naming(tmp_path):
    video = tmp_path / "camA.mp4"
    _make_video(video)
    out = tmp_path / "frames"
    out.mkdir()
    rows = []
    kept = frame_sampler.sample_video(video, out, interval=0.5, max_frames=10,
                                      diff_thresh=1.0, manifest_rows=rows)
    assert kept == len(rows) and 1 <= kept <= 10
    for row in rows:
        # 文件名规则：{stem}_{秒:06.1f}.jpg
        assert row["frame_file"].startswith("camA_")
        assert row["frame_file"].endswith(".jpg")
        sec = float(row["timestamp"])
        assert row["frame_file"] == f"camA_{sec:06.1f}.jpg"
        assert row["resolution"] == "320x180"
        assert (out / row["frame_file"]).exists()
    # manifest.csv 可写可读
    manifest = out / "manifest.csv"
    with manifest.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=["source", "timestamp", "frame_file", "resolution"])
        w.writeheader()
        w.writerows(rows)
    with open(manifest, encoding="utf-8") as f:
        assert len(list(csv.DictReader(f))) == kept


def test_sampler_bad_video_and_empty_dir(tmp_path):
    out = tmp_path / "frames"
    out.mkdir()
    rows = []
    assert frame_sampler.sample_video(tmp_path / "missing.mp4", out, 1.0, 5, 8.0, rows) == 0
    assert rows == []


def test_load_classes_txt_and_yaml(tmp_path):
    txt = tmp_path / "classes.txt"
    txt.write_text("person\nlab coat\n# 注释\nno lab coat\n", encoding="utf-8")
    assert preannotate.load_classes(txt) == ["person", "lab coat", "no lab coat"]
    yml = tmp_path / "data.yaml"
    yml.write_text("names:\n- Glasses\n- Gloves\n- Lab Coat\nnc: 3\n", encoding="utf-8")
    assert preannotate.load_classes(yml) == ["Glasses", "Gloves", "Lab Coat"]


def test_preannotate_empty_images(tmp_path, capsys):
    """空目录：打印提示、不加载模型（用不存在的模型路径也不应报错）。"""
    img_dir = tmp_path / "imgs"
    img_dir.mkdir()
    sys.argv = ["preannotate.py", "--images", str(img_dir), "--out", str(tmp_path / "labels"),
                "--model", str(tmp_path / "nope.onnx"), "--classes-file", str(tmp_path / "c.txt")]
    (tmp_path / "c.txt").write_text("person\n", encoding="utf-8")
    assert preannotate.main() == 0
    assert "为空" in capsys.readouterr().out
