"""Export the checked-in white-coat confirmation graph from local Ultralytics assets."""

import argparse
import contextlib
import shutil
import tempfile
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("checkpoint", type=Path, help="Local yoloe-26n-seg.pt")
    parser.add_argument("text_encoder", type=Path, help="Local mobileclip2_b.ts")
    parser.add_argument("output", type=Path, help="Output ONNX file")
    args = parser.parse_args()
    checkpoint = args.checkpoint.resolve(strict=True)
    encoder = args.text_encoder.resolve(strict=True)
    output = args.output.resolve()
    if output.exists():
        parser.error("output already exists")

    from ultralytics import YOLOE

    prompts = ["white lab coat", "white sweater", "casual jacket", "surgical face mask"]
    with tempfile.TemporaryDirectory(prefix="ls-coat-export-") as work, contextlib.chdir(work):
        shutil.copyfile(checkpoint, "yoloe-26n-seg.pt")
        shutil.copyfile(encoder, "mobileclip2_b.ts")
        source = YOLOE("yoloe-26n-seg.pt")
        detector = YOLOE("yoloe-26n.yaml").load("yoloe-26n-seg.pt")
        detector.set_classes(prompts, source.get_text_pe(prompts))
        exported = detector.export(format="onnx", imgsz=640, batch=1, dynamic=False,
                                   opset=17, simplify=False, nms=False, device="cpu")
        shutil.copyfile(exported, output)


if __name__ == "__main__":
    main()
