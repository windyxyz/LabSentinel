"""Run the LS-086 native Rust CPU, DirectML and ncnn comparisons sequentially."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess


def overlap(a, b):
    def area(box):
        return max(0, box[2] - box[0]) * max(0, box[3] - box[1])

    intersection = area((max(a[0], b[0]), max(a[1], b[1]), min(a[2], b[2]), min(a[3], b[3])))
    union = area(a) + area(b) - intersection
    return intersection / union if union > 0 else 0


def compare_boxes(expected, actual):
    edges = [
        [i for i, other in enumerate(actual) if box["class_id"] == other["class_id"]
         and abs(box["score"] - other["score"]) <= 0.02 and overlap(box["xyxy"], other["xyxy"]) >= 0.95]
        for box in expected
    ]
    associations = {}

    def extend(index, visited):
        for target in edges[index]:
            if target in visited:
                continue
            visited.add(target)
            if target not in associations or extend(associations[target], visited):
                associations[target] = index
                return True
        return False

    for index in range(len(expected)):
        extend(index, set())
    return {"reference": len(expected), "candidate": len(actual), "matched": len(associations)}


def run(args, classes, mode, model, environment):
    command = [str(args.exe), "--classes", str(classes), mode, str(model), str(args.iterations),
               *map(str, args.frames)]
    result = subprocess.run(command, env=environment, capture_output=True, text=True,
                            encoding="utf-8", errors="replace", timeout=180,
                            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    if result.returncode:
        raise RuntimeError(f"{mode} exited {result.returncode}: {result.stderr}")
    samples = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
    if [sample["streams"] for sample in samples] != [1, 4] or any(
        sample["frames"] != sample["streams"] * args.iterations
        or sample["comparison_classes"] != classes
        or len(sample["predictions"]) != len(args.frames) for sample in samples
    ):
        raise RuntimeError(f"{mode} returned an incomplete benchmark")
    return {"samples": samples, "diagnostics": result.stderr.splitlines()}


def fingerprint(path):
    with path.open("rb") as f:
        return {"name": path.name, "sha256": hashlib.file_digest(f, "sha256").hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ("exe", "ort", "payload", "people", "ppe", "out"):
        parser.add_argument("--" + flag, type=lambda value: Path(value).resolve(), required=True)
    parser.add_argument("--ppe-classes", type=int, required=True)
    parser.add_argument("--adapter", type=int, default=0, help="index in the ncnn Vulkan device list")
    parser.add_argument("--iterations", type=int, default=80, help="timed frames per stream")
    parser.add_argument("--candidate-mode", choices=("ncnn", "ncnn-fp16-storage", "ncnn-fp16"),
                        default="ncnn-fp16")
    parser.add_argument("frames", metavar="1280x720.bgr", nargs="+", type=Path)
    args = parser.parse_args()
    if not 1 <= args.ppe_classes <= 80 or args.iterations < 1 or args.adapter < 0:
        parser.error("invalid class count, iteration count or adapter index")
    if args.out.exists():
        parser.error("report path already exists")

    dll = args.payload / "ncnn.dll"
    env = {**os.environ, "ORT_DYLIB_PATH": str(args.ort), "LS_NCNN_DLL": str(dll),
           "LS_NCNN_ADAPTER": str(args.adapter)}
    report = {"platform": platform.platform(), "adapter_requested": args.adapter,
              "iterations": args.iterations, "candidate_mode": args.candidate_mode, "runner": fingerprint(args.exe),
              "frames": [fingerprint(path) for path in args.frames],
              "ncnn": fingerprint(dll), "ort": fingerprint(args.ort), "models": {}}
    for tag, onnx, base_name, classes in (
        ("people", args.people, "spike-yolo11n-fp16.ncnn", 80),
        ("ppe", args.ppe, "spike-yolo11n_ppe-fp16.ncnn", args.ppe_classes),
    ):
        ncnn = args.payload / base_name
        runs = {mode: run(args, classes, mode, input_path, env) for mode, input_path in (
            ("cpu", onnx), ("dml", onnx), (args.candidate_mode, ncnn),
        )}
        comparison = [
            compare_boxes(expected, actual) for expected, actual in zip(
                runs["cpu"]["samples"][0]["predictions"],
                runs[args.candidate_mode]["samples"][0]["predictions"], strict=True,
            )
        ]
        opportunities = sum(max(row["reference"], row["candidate"]) for row in comparison)
        agreement = sum(row["matched"] for row in comparison) / opportunities if opportunities else None
        report["models"][tag] = {"onnx": fingerprint(onnx), "classes": classes,
                                 "converted": [fingerprint(Path(str(ncnn) + suffix)) for suffix in (".param", ".bin")],
                                 "comparison": comparison, "agreement": agreement, "runs": runs}
        print(f"{tag}: {sum(row['matched'] for row in comparison)}/{opportunities} boxes, "
              f"4-stream DirectML {runs['dml']['samples'][1]['fps']:.2f} fps, "
              f"Vulkan {runs[args.candidate_mode]['samples'][1]['fps']:.2f} fps", flush=True)
    with args.out.open("x", encoding="utf-8") as stream:
        json.dump(report, stream, indent=2, ensure_ascii=False)
        stream.write("\n")
    return 0 if all((model["agreement"] or 0) >= 0.98 for model in report["models"].values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
