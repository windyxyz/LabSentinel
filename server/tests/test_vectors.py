"""T-323：读取与 Rust（ls-core）共享的测试向量，保证两侧领域行为逐用例一致。

真值来源：point_in_polygon 用 cv2.pointPolygonTest（现状实现）；ZoneState 用
app.zones.ZoneState（T-306 迟滞）。向量文件：testdata/vectors/*.json。
运行: pytest tests/test_vectors.py -q
"""
import json
import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import cv2  # noqa: E402
import numpy as np  # noqa: E402

from app.zones import ZoneState, parse_polygon, point_inside  # noqa: E402

VECTORS = Path(__file__).resolve().parents[2] / "testdata" / "vectors"


def _load(name: str) -> dict:
    return json.loads((VECTORS / name).read_text(encoding="utf-8"))


def test_zone_state_hysteresis_matches_rust_shared_vectors():
    v = _load("zone_state_hysteresis.json")
    assert v["cases"], "向量文件为空"
    for case in v["cases"]:
        st = ZoneState(enter_frames=case["enter_frames"], exit_frames=case["exit_frames"])
        got = []
        for tid, inside in case["observations"]:
            r = st.update(int(tid), bool(inside))
            if r is None:
                got.append(None)
            elif r == (True, False):
                got.append("enter")
            else:
                assert r == (False, True), r
                got.append("exit")
        assert got == case["expected"], f"向量用例 {case['name']} 不一致: {got}"


def test_point_in_polygon_matches_opencv_shared_vectors():
    v = _load("point_in_polygon.json")
    assert v["cases"], "向量文件为空"
    for case in v["cases"]:
        poly = parse_polygon(v["polygons"][case["poly"]])
        p = tuple(int(x) for x in case["point"])
        got = point_inside(poly, p)  # = cv2.pointPolygonTest(...) >= 0（边界算内）
        assert got == case["inside"], f"多边形 {case['poly']} 点 {p} 判定不一致"
