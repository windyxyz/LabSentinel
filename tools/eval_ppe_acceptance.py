"""Prepare Lab Coat Detection 3 labels and score Rust PPE predictions offline."""

import argparse
from collections import Counter
from hashlib import sha256
import json
from math import isfinite
from pathlib import Path


CLASSES = {"0": "wearing", "1": "no_ppe"}
OUTCOMES = (*CLASSES.values(), "unknown", "missed_person")
MATCH_IOU = 0.50


def read_labels(path):
    people = []
    for fields in (line.split() for line in path.read_text(encoding="utf-8").splitlines()):
        if not fields:
            continue
        class_id, *coords = fields
        points = tuple(map(float, coords))
        if len(points) == 4:
            cx, cy, width, height = points
            bbox = (cx - width / 2, cy - height / 2, cx + width / 2, cy + height / 2)
        elif len(points) >= 6 and len(points) % 2 == 0:
            xs, ys = points[::2], points[1::2]
            bbox = (min(xs), min(ys), max(xs), max(ys))
        else:
            raise ValueError(f"{path}: expected a YOLO box or polygon")
        if not all(isfinite(v) and 0 <= v <= 1 for v in bbox) or not (
            bbox[0] < bbox[2] and bbox[1] < bbox[3]
        ):
            raise ValueError(f"{path}: invalid normalized box {bbox}")
        people.append({"label": CLASSES[class_id], "bbox": bbox})
    return people


def prepare(dataset, manifest):
    images = sorted((dataset / "images").glob("*.jpg"))
    labels = {p.stem: p for p in (dataset / "labels").glob("*.txt")}
    if not images or {p.stem for p in images} != set(labels):
        raise ValueError("Expected matching image and label files")
    digests = set()
    with manifest.open("x", encoding="utf-8") as output:
        for image in images:
            digest = sha256(image.read_bytes()).hexdigest()
            if digest in digests:
                raise ValueError(f"Duplicate image bytes: {image.name}")
            digests.add(digest)
            people = read_labels(labels[image.stem])
            sample = {
                "image": str(image.resolve()),
                "sha256": digest,
                "boxes": [person["bbox"] for person in people],
                "labels": [person["label"] for person in people],
            }
            print(json.dumps(sample), file=output)


def iou(a, b):
    width = max(0, min(a[2], b[2]) - max(a[0], b[0]))
    height = max(0, min(a[3], b[3]) - max(a[1], b[1]))
    overlap = width * height
    if not overlap:
        return 0.0
    return overlap / (
        (a[2] - a[0]) * (a[3] - a[1])
        + (b[2] - b[0]) * (b[3] - b[1])
        - overlap
    )


def match(boxes, detections):
    """Find one-to-one matches without using the prediction's clothing class."""
    candidates = [
        [(score, j) for j, det in enumerate(detections) if (score := iou(box, det)) >= MATCH_IOU]
        for box in boxes
    ]
    owners = {}

    def claim(label, visited):
        for _, det in sorted(candidates[label], reverse=True):
            if det in visited:
                continue
            visited.add(det)
            if det not in owners or claim(owners[det], visited):
                owners[det] = label
                return True
        return False

    for label in sorted(range(len(boxes)), key=lambda i: max(candidates[i], default=(0, 0)),
                        reverse=True):
        claim(label, set())
    return {label: det for det, label in owners.items()}


def by_image(path):
    rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]
    by_path = {row["image"]: row for row in rows}
    if len(by_path) != len(rows):
        raise ValueError(f"Duplicate image entries in {path}")
    return by_path


def totals(matrix):
    people = sum(sum(row.values()) for row in matrix.values())
    correct = sum(matrix[label][label] for label in CLASSES.values())
    missed = sum(row["missed_person"] for row in matrix.values())
    unknown = sum(row["unknown"] for row in matrix.values())
    resolved = people - missed - unknown
    recall = {label: row[label] / sum(row.values()) if sum(row.values()) else None
              for label, row in matrix.items()}
    recalls = [value for value in recall.values() if value is not None]
    return {
        "people": people,
        "matrix": matrix,
        "missed_person": missed,
        "unknown": unknown,
        "unknown_per_matched_person": unknown / (people - missed) if people > missed else None,
        "accuracy_when_decided": correct / resolved if resolved else None,
        "recall": recall,
        "balanced_recall": sum(recalls) / len(recalls) if len(recalls) == 2 else None,
    }


def score(manifest, predictions, output):
    samples, results = by_image(manifest), by_image(predictions)
    if samples.keys() != results.keys():
        raise ValueError("Reference images and prediction images differ")
    summary = {"images": len(samples), "match_iou": MATCH_IOU, "conditions": {}}
    for condition in ("original", "dimmed"):
        modes = {
            mode: {label: Counter(dict.fromkeys(OUTCOMES, 0)) for label in CLASSES.values()}
            for mode in ("detected", "reference")
        }
        reasons = {mode: Counter() for mode in modes}
        unmatched = Counter()
        for image, sample in samples.items():
            result = results[image]
            data = result[condition]
            w, h = result["width"], result["height"]
            positions = match(
                [[x1 * w, y1 * h, x2 * w, y2 * h] for x1, y1, x2, y2 in sample["boxes"]],
                [item["bbox"] for item in data["detected"]],
            )
            if len(data["reference"]) != len(sample["labels"]):
                raise ValueError(f"Reference result count differs for {image}")
            matched = set(positions.values())
            unmatched.update(item["ppe"]["coat"] for index, item in enumerate(data["detected"])
                             if index not in matched)
            for index, label in enumerate(sample["labels"]):
                target = positions.get(index)
                matched_person = data["detected"][target]["ppe"] if target is not None else None
                for mode, observation in (("detected", matched_person),
                                          ("reference", data["reference"][index])):
                    outcome = observation["coat"] if observation else "missed_person"
                    if outcome not in OUTCOMES:
                        raise ValueError(f"Unexpected PPE outcome {outcome}")
                    modes[mode][label][outcome] += 1
                    if observation and outcome != label:
                        reasons[mode][observation["diagnostic"]["result"]] += 1
        summary["conditions"][condition] = {
            **{mode: {**totals(matrix), "incorrect_or_unknown_reasons": reasons[mode]}
               for mode, matrix in modes.items()},
            "unmatched_detections": unmatched,
        }
    summary["devices"] = sorted({(r["person_device"], r["ppe_device"]) for r in results.values()})
    with output.open("x", encoding="utf-8") as handle:
        json.dump(summary, handle, ensure_ascii=False, indent=2)
        handle.write("\n")
    print(json.dumps(summary, ensure_ascii=False))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest="action", required=True)
    build = actions.add_parser("prepare")
    build.add_argument("dataset", type=Path)
    build.add_argument("manifest", type=Path)
    evaluate = actions.add_parser("score")
    evaluate.add_argument("manifest", type=Path)
    evaluate.add_argument("predictions", type=Path)
    evaluate.add_argument("output", type=Path)
    args = parser.parse_args()
    if args.action == "prepare":
        prepare(args.dataset, args.manifest)
    else:
        score(args.manifest, args.predictions, args.output)


if __name__ == "__main__":
    main()
