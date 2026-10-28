"""单元测试：区域判定 + 规则引擎 + 存储回路。运行: pytest tests/ -q"""
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app.config import Settings  # noqa: E402
from app.rules import RuleEngine  # noqa: E402
from app.store import Store  # noqa: E402
from app.zones import ZoneState, boxes_overlap, parse_polygon, point_inside  # noqa: E402

POLY = [[100, 100], [600, 100], [600, 500], [100, 500]]


def _tmp_store(tmp_path):
    return Store(path=tmp_path / "t.db")


def test_boxes_overlap():
    assert boxes_overlap((100, 100, 300, 400), (200, 200, 400, 500)) is True
    assert boxes_overlap((0, 0, 50, 50), (100, 100, 200, 200)) is False
    assert boxes_overlap((100, 100, 300, 400), (290, 390, 500, 600)) is True  # 角相交
    assert boxes_overlap((100, 100, 300, 400), (300, 100, 500, 400)) is False  # 仅边贴边


def test_point_inside():
    poly = parse_polygon(POLY)
    assert point_inside(poly, (300, 300)) is True
    assert point_inside(poly, (50, 50)) is False
    assert point_inside(poly, (100, 100)) is True  # 边界算在内


def test_zone_state_transitions():
    zs = ZoneState()
    assert zs.update(1, True) == (True, False)   # 进入
    assert zs.update(1, True) is None            # 持续在内
    assert zs.update(1, False) == (False, True)  # 离开
    assert zs.update(1, False) is None


def test_store_roundtrip(tmp_path):
    s = _tmp_store(tmp_path)
    cid = s.add_camera("cam1", "x.mp4", "file")
    zid = s.add_zone(cid, "zoneA", POLY)
    rid = s.add_rule({"camera_id": cid, "zone_id": zid, "behavior": "enter"})
    eid = s.add_event(cid, "enter", 2, 7, "进入安全区「zoneA」", None)
    assert s.camera(cid)["name"] == "cam1"
    assert s.zones(cid)[0]["polygon"] == POLY
    assert s.rules(cid)[0]["behavior"] == "enter"
    assert s.events(limit=1)[0]["id"] == eid
    s.delete_zone(zid)
    assert s.zones(cid) == []


def test_rule_engine_match_and_cooldown(tmp_path):
    s = _tmp_store(tmp_path)
    cid = s.add_camera("cam1", "x.mp4", "file")
    zid = s.add_zone(cid, "zoneA", POLY)
    s.add_rule({"camera_id": cid, "zone_id": zid, "behavior": "enter",
                "level": 2, "cooldown_s": 60})
    eng = RuleEngine(s)
    r = eng.match(cid, zid, "enter", track_id=7)
    assert r is not None and r["level"] == 2
    # 冷却期内不再命中
    assert eng.match(cid, zid, "enter", track_id=7) is None
    # 不同 track 不受冷却影响
    assert eng.match(cid, zid, "enter", track_id=8) is not None
    # 其他行为不命中
    assert eng.match(cid, zid, "exit", track_id=7) is None


def test_rule_engine_time_window(tmp_path):
    s = _tmp_store(tmp_path)
    cid = s.add_camera("cam1", "x.mp4", "file")
    now = time.strftime("%H:%M")
    hh = int(now[:2])
    start = f"{(hh + 1) % 24:02d}:00"  # 一小时后开始的窗口 → 现在不在窗口内
    end = f"{(hh + 2) % 24:02d}:00"
    zid = s.add_zone(cid, "zoneA", POLY)
    s.add_rule({"camera_id": cid, "zone_id": zid, "behavior": "enter",
                "time_start": start, "time_end": end, "cooldown_s": 0})
    eng = RuleEngine(s)
    assert eng.match(cid, zid, "enter", track_id=1) is None


def test_accel_run_serializes():
    """根因修复验证（2026-10-26 死机事故）：加速后端(_accel=True)多线程 _run 必须完全串行。

    背景：DirectML 会话并发 run 会挂死 GPU 驱动冻死整机（3× Kernel-Power 41 证据），
    修复为模块级全局锁。本测试用 FakeSess 验证串行性，不加载真实模型/不碰 DML。
    执行留给子代理：pytest tests/test_core.py::test_accel_run_serializes -v
    """
    import threading
    import time as _t
    from app import engine as eng

    e = eng.Engine.__new__(eng.Engine)  # 不走 __init__，不加载真模型
    e.backend = "directml"
    e.input_name = "x"
    e._accel = True
    state = {"active": 0, "max_active": 0, "lock": threading.Lock()}

    class FakeSess:
        def run(self, names, feeds):
            with state["lock"]:
                state["active"] += 1
                state["max_active"] = max(state["max_active"], state["active"])
            _t.sleep(0.02)
            with state["lock"]:
                state["active"] -= 1
            return [None]

    e.sess = FakeSess()
    ts = [threading.Thread(target=e._run, args=(None,)) for _ in range(4)]
    [t.start() for t in ts]
    [t.join() for t in ts]
    assert state["max_active"] == 1, "并发 _run 出现交错，串行锁失效！"

    e._accel = False  # CPU 后端不加锁路径冒烟
    assert e._run(None) == [None]


def test_ppe_class_names_sidecar(tmp_path, monkeypatch):
    """M1.5#2：PPE 词表优先读 <model>.classes.txt 同名文件（换模型零改代码）。"""
    from app import engine as eng

    monkeypatch.setattr(eng.settings, "ppe_class_names", "lab coat")  # 旧配置兜底值
    model = tmp_path / "m.onnx"
    model.write_bytes(b"x")
    # 无 sidecar：回退 LS_PPE_CLASS_NAMES
    assert eng._load_class_names(model) == ["lab coat"]
    # 有 sidecar：读文件、顺序保留、忽略空行与注释
    model.with_suffix(".classes.txt").write_text(
        "Glasses\n\nLab Coat\n# 注释\nNo Lab Coat\n", encoding="utf-8")
    assert eng._load_class_names(model) == ["Glasses", "Lab Coat", "No Lab Coat"]
    # 空 sidecar：警告并回退
    empty_model = tmp_path / "e.onnx"
    empty_model.write_bytes(b"x")
    empty_model.with_suffix(".classes.txt").write_text("\n# 只有注释\n", encoding="utf-8")
    assert eng._load_class_names(empty_model) == ["lab coat"]


def test_capture_source_usb_numeric():
    """USB 源 "0"/"1" 必须转 int（字符串会被 OpenCV 当文件名）；RTSP/文件原样。"""
    from app.pipeline import capture_source
    assert capture_source({"type": "usb", "source": "0"}) == 0
    assert capture_source({"type": "usb", "source": " 1 "}) == 1
    assert capture_source({"type": "usb", "source": "OBS Virtual Camera"}) == "OBS Virtual Camera"
    assert capture_source({"type": "rtsp", "source": "rtsp://127.0.0.1:8554/lab"}) == "rtsp://127.0.0.1:8554/lab"
