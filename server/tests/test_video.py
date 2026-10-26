"""单元测试：视频存证（环形缓冲 / 旧库迁移 / 编码器）。运行: pytest tests/ -q"""
import os
import sqlite3
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import cv2  # noqa: E402
import numpy as np  # noqa: E402

from app.pipeline import VIDEO_POST_S, VIDEO_PRE_S, CameraWorker, _Recording  # noqa: E402
from app.store import Store, _migrate  # noqa: E402

_CAM = {"id": 1, "name": "cam1", "source": "x.mp4", "type": "file"}


def test_ring_buffer_capacity():
    """环形缓冲长度 = 前2秒帧数 + 10 余量；超长自动丢弃最旧帧。"""
    w = CameraWorker(_CAM, None, None)
    assert VIDEO_PRE_S == 2.0 and VIDEO_POST_S == 2.0
    w._init_ring(30)
    assert w._ring.maxlen == 30 * 2 + 10
    for i in range(200):
        w._ring.append(np.full((720, 1280, 3), i % 255, np.uint8))
    assert len(w._ring) == w._ring.maxlen
    # 低帧率源也有下限
    w._init_ring(5)
    assert w._ring.maxlen == 20


def test_recording_lifecycle():
    """录像任务：补齐前 done 未置位，补齐后入队一次。"""
    w = CameraWorker(_CAM, None, None)
    w._init_ring(30)
    rec = _Recording(1, list(w._ring))
    assert rec.done.is_set() is False
    import time as _t
    rec.t0 = _t.monotonic() - 10  # 模拟已过 2+2 秒
    w._recs = [rec]
    for _ in range(3):  # 喂几帧 → 立即补齐并入队
        w._feed_recordings(np.zeros((720, 1280, 3), np.uint8))
    assert rec.done.is_set()
    assert not w._recs  # 已从活动列表清除
    assert w._rec_q.get_nowait() is rec


def test_store_migration_old_db(tmp_path):
    """旧库（无 video 列）打开后自动迁移，新列可读写。"""
    db = tmp_path / "old.db"
    with sqlite3.connect(db) as c:
        c.execute("""CREATE TABLE events(
          id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT, camera_id INTEGER, type TEXT,
          level INTEGER, track_id INTEGER, detail TEXT, snapshot TEXT)""")
        c.execute("INSERT INTO events(ts,camera_id,type,level,track_id,detail,snapshot)"
                  " VALUES('2026-01-01T08:00:00',1,'enter',2,1,'旧事件',NULL)")
    s = Store(path=db)
    eid = s.events(limit=1)[0]["id"]
    assert s.event_video(eid) is None
    s.set_event_video(eid, "videos/ev_1.mp4")
    assert s.event_video(eid) == "videos/ev_1.mp4"
    # 迁移函数幂等：重复执行不报错
    with sqlite3.connect(db) as c:
        _migrate(c)
        _migrate(c)


def test_write_video_smoke(tmp_path):
    """编码器冒烟：60 帧 @30fps（2 秒）写出的 MP4 可读且帧数正确。"""
    s = Store(path=tmp_path / "t.db")
    w = CameraWorker(_CAM, s, None)
    rec = _Recording(1, [])
    rec.frames = [np.random.randint(0, 255, (720, 1280, 3), np.uint8) for _ in range(60)]
    path = str(tmp_path / "ev_1.mp4")
    w._write_video(rec, path=path, fps=30.0)
    assert os.path.exists(path) and os.path.getsize(path) > 0
    cap = cv2.VideoCapture(path)
    n = int(cap.get(cv2.CAP_PROP_FRAME_COUNT))
    fps = cap.get(cv2.CAP_PROP_FPS)
    cap.release()
    assert n >= 55  # 允许编码器少量丢尾帧
    assert fps > 0
    assert n / fps >= 1.5  # 时长 ≥1.5s（目标 2s）


def test_write_video_codec_fallback(tmp_path):
    """编码优先级：有 OpenH264 时产出 h264（浏览器可播），否则回退 mp4v/FMP4。"""
    w = CameraWorker(_CAM, Store(path=tmp_path / "t.db"), None)
    p = str(tmp_path / "codec.mp4")
    vw = w._open_writer(p, 30)
    assert vw is not None
    for _ in range(10):
        vw.write(np.zeros((720, 1280, 3), np.uint8))
    vw.release()
    cap = cv2.VideoCapture(p)
    tag = int(cap.get(cv2.CAP_PROP_FOURCC)).to_bytes(4, "little").decode(errors="replace")
    cap.release()
    assert tag in ("h264", "FMP4")  # h264=OpenH264；FMP4=mp4v 兜底
