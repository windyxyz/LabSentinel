"""单元测试：任务105 回归补丁 P0-1（空帧/部分消失离场的状态清理 + A→B 直移语义）。

_analyze 用假 engine（detect_persons 返回带 .xyxy 的对象）驱动真实 worker，不加载 ONNX。
运行: pytest tests/test_zone_exit_state.py -q
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import numpy as np  # noqa: E402
import pytest  # noqa: E402

from app.pipeline import CameraWorker  # noqa: E402
from app.rules import RuleEngine  # noqa: E402
from app.store import Store  # noqa: E402
from app.zones import ZoneState  # noqa: E402

POLY = "[[100, 50], [1100, 50], [1100, 650], [100, 650]]"


def _det(x1, y1, x2, y2):
    class D:  # 假检出：.xyxy 供 ByteTrack，.conf 供置信度数组
        conf = 0.9
    d = D()
    d.xyxy = [float(x1), float(y1), float(x2), float(y2)]
    return d


class FakeEngine:
    def __init__(self, boxes):
        self.boxes = boxes

    def detect_persons(self, frame):
        return self.boxes


@pytest.fixture()
def worker(tmp_path):
    from app.config import settings
    settings.data_dir = tmp_path
    s = Store(tmp_path / "s.db")
    cid = s.add_camera("cam105", "x.mp4", "file")
    # store.add_zone 会 json.dumps 存库、读回时 json.loads——必须传列表而非字符串
    zid = s.add_zone(cid, "z105", [[100, 50], [1100, 50], [1100, 650], [100, 650]])
    s.add_rule({"camera_id": cid, "zone_id": zid, "behavior": "enter", "level": 2,
                "cooldown_s": 0, "time_start": "00:00", "time_end": "23:59",
                "threshold": 0, "enabled": True})
    s.add_rule({"camera_id": cid, "zone_id": zid, "behavior": "exit", "level": 2,
                "cooldown_s": 0, "time_start": "00:00", "time_end": "23:59",
                "threshold": 0, "enabled": True})  # exit 也是规则事件，需对应规则才会入库
    w = CameraWorker({"id": cid, "name": "cam105", "source": "x.mp4", "type": "file"},
                     s, RuleEngine(s))
    w.ppe = None  # run() 之外手动测试 _analyze，需自行补字段
    frame = np.zeros((720, 720, 3), dtype=np.uint8)
    return w, frame, s, zid


def event_details(s, cid):
    return [(e["type"], e["detail"]) for e in s.events(camera_id=cid, limit=100)]


def test_zone_state_semantics():
    """进入→离场→再进入：第二次必须返回 entered（105 补丁 P0-1 的状态语义核心）。"""
    st = ZoneState()
    assert st.update(1, True) == (True, False)
    assert st.update(1, False) == (False, True)
    assert st.update(1, False) is None  # 空帧重复离场：无变化
    assert st.update(1, True) == (True, False)  # 再进入必须再次 enter


def test_empty_frame_exit_once_then_reenter(worker):
    """空帧：exit 只发一次；人再进入必须重新 enter（原回归：was=True 永不 enter）。"""
    w, frame, s, zid = worker
    w._analyze(FakeEngine([_det(300, 200, 500, 600)]), frame)
    assert sum(1 for t, _ in event_details(s, w.camera["id"]) if t == "enter") == 1
    w._analyze(FakeEngine([]), frame)
    assert sum(1 for t, _ in event_details(s, w.camera["id"]) if t == "exit") == 1
    w._analyze(FakeEngine([]), frame)  # 第二个空帧：不得重复 exit
    assert sum(1 for t, _ in event_details(s, w.camera["id"]) if t == "exit") == 1
    w._analyze(FakeEngine([_det(300, 200, 500, 600)]), frame)  # 再进入必须重新 enter
    assert sum(1 for t, _ in event_details(s, w.camera["id"]) if t == "enter") == 2


def test_partial_disappear_exit_once(worker):
    """两人同区、一人消失：消失者 exit 一次，在场者不受影响。"""
    w, frame, s, zid = worker
    w._analyze(FakeEngine([_det(300, 200, 500, 600), _det(600, 200, 800, 600)]), frame)
    assert sum(1 for t, _ in event_details(s, w.camera["id"]) if t == "enter") == 2
    w._analyze(FakeEngine([_det(300, 200, 500, 600)]), frame)  # 只剩一人
    exits = [d for t, d in event_details(s, w.camera["id"]) if t == "exit"]
    assert len(exits) == 1
    w._analyze(FakeEngine([_det(300, 200, 500, 600)]), frame)  # 后续帧不重复 exit
    assert len([d for t, d in event_details(s, w.camera["id"]) if t == "exit"]) == 1


def test_direct_move_a_to_b(worker):
    """A→B 连续步行（帧间 IoU 重叠）：A exit + B enter 各一次（保留审阅的语义修复）。

    注意：ByteTrack 对异位新检出要等一帧才激活（f2 输出 0 个）——瞬移式测试数据不现实，
    必须用连续重叠的框模拟真实步行。
    """
    w, frame, s, zid = worker
    zid2 = s.add_zone(w.camera["id"], "z105b",
                      [[900, 50], [1279, 50], [1279, 650], [900, 650]])
    s.add_rule({"camera_id": w.camera["id"], "zone_id": zid2, "behavior": "enter",
                "level": 2, "cooldown_s": 0, "time_start": "00:00", "time_end": "23:59",
                "threshold": 0, "enabled": True})  # B 区自己的 enter 规则（规则按 zone 绑定）
    # 锚点 x：400(A) →20px 步进→ 1150(仅 B)。步长必须小：supervision ByteTrack 匹配要求
    # IoU≥0.8，大步长会丢轨（新轨还要两帧激活），真实视频里是连续小位移。
    boxes = []
    x = 300
    while x <= 1030:
        boxes.append((x, 200, x + 200, 600))
        x += 20
    boxes.append((1050, 200, 1250, 600))  # 锚点 1150：仅 B
    for box in boxes:
        w._analyze(FakeEngine([_det(*box)]), frame)
    types = [(t, d) for t, d in event_details(s, w.camera["id"])]
    assert sum(1 for t, _ in types if t == "enter" and "z105b" in _) == 1, types
    assert sum(1 for t, _ in types if t == "exit" and "z105」" in _) == 1, types
    assert sum(1 for t, _ in types if t == "enter" and "「z105」" in _) == 1, types
