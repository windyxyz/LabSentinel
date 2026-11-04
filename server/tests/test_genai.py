"""单元测试：任务124 本地 VLM 误报预分类（fake transport 零外网；默认 none 零行为）。
运行: pytest tests/test_genai.py -q"""
import json
import threading
import time

import pytest

from app import genai
from app.config import settings
from app.store import Store

GOOD = json.dumps({"verdict": "likely_false", "reason": "画面无人", "confidence": 0.9})


@pytest.fixture(autouse=True)
def reset(monkeypatch):
    genai._q.queue.clear()
    genai._stats.update({"enqueued": 0, "dropped": 0, "reviewed": 0, "failed": 0,
                         "inflight": 0, "max_inflight": 0})
    monkeypatch.setattr(settings, "genai_provider", "none")
    monkeypatch.setattr(settings, "genai_url", "http://127.0.0.1:11434")
    monkeypatch.setattr(settings, "genai_model", "test-vlm")
    monkeypatch.setattr(settings, "genai_min_level", 2)
    monkeypatch.setattr(settings, "genai_allow_remote", False)
    genai.stop_worker()
    yield
    genai.stop_worker()


# ---------- 解析 ----------

def test_parse_review_ok_and_fence():
    r = genai.parse_review(GOOD)
    assert r.verdict == "likely_false" and r.confidence == 0.9
    r2 = genai.parse_review(f"```json\n{GOOD}\n```")  # markdown 围栏剥离
    assert r2.verdict == "likely_false"


def test_parse_rejects_bad():
    for bad in ("not json", '{"verdict":"whatever"}', '{"verdict":"likely_true","extra":1}',
                '{"reason":"缺 verdict"}'):
        with pytest.raises(genai.GenAIError):
            genai.parse_review(bad)
    # confidence 钳制 + reason 截断
    r = genai.parse_review(json.dumps({"verdict": "uncertain", "reason": "x" * 500,
                                       "confidence": 5}))
    assert r.confidence == 1.0 and len(r.reason) == 200


# ---------- 回环保护 ----------

def test_remote_url_rejected_without_flag():
    with pytest.raises(genai.GenAIError) as ei:
        genai._assert_loopback("http://192.168.1.50:11434")
    assert "ALLOW_REMOTE" in str(ei.value)
    genai._assert_loopback("http://127.0.0.1:11434")  # 回环 OK 不抛
    genai._assert_loopback("http://localhost:11434")


# ---------- provider 请求体 ----------

def test_ollama_request_shape(monkeypatch, tmp_path):
    snap = tmp_path / "s.jpg"
    import cv2
    import numpy as np
    cv2.imencode(".jpg", np.zeros((720, 1280, 3), dtype=np.uint8))[1].tofile(str(snap))
    monkeypatch.setattr(settings, "genai_provider", "ollama")
    monkeypatch.setattr(genai, "_write_back", lambda *a: None)  # 只验证请求形状
    seen = {}

    def fake_post(url, json=None, timeout=None, headers=None):
        seen.update(url=url, body=json)
        class R:
            status_code = 200
            def json(self):
                return {"response": GOOD}
        return R()

    monkeypatch.setattr(genai.httpx, "post", fake_post)
    r = genai._review_one(1, str(snap))
    assert seen["url"].endswith("/api/generate")
    assert seen["body"]["images"] and seen["body"]["format"] == "json"
    assert "verdict" in GOOD
    assert r is None


def test_openai_compat_request_shape(monkeypatch, tmp_path):
    snap = tmp_path / "s.jpg"
    import cv2
    import numpy as np
    cv2.imencode(".jpg", np.zeros((100, 100, 3), dtype=np.uint8))[1].tofile(str(snap))
    seen = {}
    monkeypatch.setattr(genai, "_write_back", lambda *a: None)  # 只验证请求形状

    def fake_post(url, json=None, timeout=None, headers=None):
        seen.update(url=url, body=json, headers=headers)
        class R:
            status_code = 200
            def json(self):
                return {"choices": [{"message": {"content": GOOD}}]}
        return R()

    monkeypatch.setattr(genai.httpx, "post", fake_post)
    monkeypatch.setattr(settings, "genai_provider", "openai_compat")
    monkeypatch.setattr(settings, "genai_api_key", "sk-test")
    genai._review_one(1, str(snap))
    assert seen["url"].endswith("/v1/chat/completions")
    assert seen["headers"]["Authorization"] == "Bearer sk-test"
    content = seen["body"]["messages"][0]["content"]
    assert content[1]["image_url"]["url"].startswith("data:image/jpeg;base64,")


def test_http_failure_silent(monkeypatch, tmp_path):
    snap = tmp_path / "s.jpg"
    snap.write_bytes(b"\xff\xd8x")

    def boom(*a, **kw):
        raise genai.httpx.ConnectTimeout("t")

    monkeypatch.setattr(genai.httpx, "post", boom)
    with pytest.raises(genai.GenAIError):
        genai._review_one(1, str(snap))


# ---------- 队列 / worker ----------

def test_enqueue_gates(monkeypatch):
    assert genai.maybe_enqueue(1, "D:/x.jpg", 3) is False  # provider none：零入队
    monkeypatch.setattr(settings, "genai_provider", "ollama")
    assert genai.maybe_enqueue(1, "D:/x.jpg", 1) is False  # L1 不复核
    assert genai.maybe_enqueue(1, None, 3) is False  # 无截图
    assert genai.maybe_enqueue(1, "D:/x.jpg", 3) is True


def test_worker_reviews_and_writes_back(monkeypatch, tmp_path):
    st = Store(tmp_path / "s.db")
    cid = st.add_camera("c", "D:/x.mp4", "file")
    snap = tmp_path / "s.jpg"
    import cv2
    import numpy as np
    cv2.imencode(".jpg", np.zeros((100, 100, 3), dtype=np.uint8))[1].tofile(str(snap))
    eid = st.add_event(cid, "enter", 3, 0, "待复核")
    monkeypatch.setattr(settings, "genai_provider", "ollama")
    monkeypatch.setattr(settings, "db_path", st.path)  # _write_back 直连 settings.db_path

    def fake_post(url, json=None, timeout=None, headers=None):
        class R:
            status_code = 200
            def json(self):
                return {"response": GOOD}
        return R()

    monkeypatch.setattr(genai.httpx, "post", fake_post)
    genai.maybe_enqueue(eid, str(snap), 3)
    monkeypatch.setattr(genai, "_write_back",
                        lambda eid_, review: st.set_ai_review(eid_, review.model_dump_json(),
                                                              "test-vlm"))
    genai.start_worker()
    for _ in range(50):
        row = st.event_by_id(eid)
        if row.get("ai_review"):
            break
        time.sleep(0.1)
    row = st.event_by_id(eid)
    assert row["ai_review"] and json.loads(row["ai_review"])["verdict"] == "likely_false"
    assert row["ai_review_model"] == "test-vlm" and row["ai_review_ts"]
    genai.stop_worker()


def test_worker_survives_bad_payload(monkeypatch, tmp_path):
    monkeypatch.setattr(settings, "genai_provider", "ollama")
    snap = tmp_path / "s.jpg"
    snap.write_bytes(b"\xff\xd8x")

    def boom(url, **kw):
        class R:
            status_code = 500
            def json(self):
                return {}
        return R()

    monkeypatch.setattr(genai.httpx, "post", boom)
    genai.maybe_enqueue(1, str(snap), 3)
    genai.maybe_enqueue(2, str(snap), 3)
    genai.start_worker()
    deadline = time.time() + 5
    while genai._stats["failed"] < 2 and time.time() < deadline:
        time.sleep(0.1)
    assert genai._stats["failed"] == 2  # 两条都失败且线程仍活着
    assert genai._thread.is_alive()
    genai.stop_worker()


def test_single_concurrency(monkeypatch, tmp_path):
    """fake provider 慢调用：并发计数不得超过 1。"""
    monkeypatch.setattr(settings, "genai_provider", "ollama")
    snap = tmp_path / "s.jpg"
    import cv2
    import numpy as np
    cv2.imencode(".jpg", np.zeros((100, 100, 3), dtype=np.uint8))[1].tofile(str(snap))
    lock = threading.Lock()
    state = {"cur": 0, "max": 0}

    def slow_post(url, **kw):
        with lock:
            state["cur"] += 1
            state["max"] = max(state["max"], state["cur"])
        time.sleep(0.3)
        with lock:
            state["cur"] -= 1
        class R:
            status_code = 200
            def json(self):
                return {"response": GOOD}
        return R()

    monkeypatch.setattr(genai.httpx, "post", slow_post)
    monkeypatch.setattr(genai, "_write_back", lambda *a: None)
    for i in range(3):
        genai.maybe_enqueue(i, str(snap), 3)
    genai.start_worker()
    deadline = time.time() + 8
    while genai._stats["reviewed"] < 3 and time.time() < deadline:
        time.sleep(0.1)
    assert state["max"] == 1  # 单并发
    genai.stop_worker()


def test_queue_full_drops(monkeypatch):
    monkeypatch.setattr(settings, "genai_provider", "ollama")
    genai._q.queue.clear()
    for i in range(genai.QUEUE_MAX):
        genai.maybe_enqueue(i, "D:/x.jpg", 3)
    assert genai.maybe_enqueue(999, "D:/x.jpg", 3) is False  # 满即丢弃
    assert genai._stats["dropped"] == 1 and genai._stats["enqueued"] == genai.QUEUE_MAX


def test_compress_jpeg_chinese_path(tmp_path):
    """回归：项目默认路径含中文，cv2.imread 会静默失败——必须走 vision.read_image。"""
    import cv2
    import numpy as np
    p = tmp_path / "中文_截图.jpg"
    p.write_bytes(cv2.imencode(".jpg", np.zeros((360, 640, 3), dtype=np.uint8))[1].tobytes())
    data = genai._compress_jpeg(str(p))
    assert data is not None and data[:2] == b"\xff\xd8"


def test_migration_v9_old_rows_null(tmp_path):
    from app.store import Store as S
    import sqlite3
    db = tmp_path / "old.db"
    con = sqlite3.connect(str(db))
    con.executescript("""
      CREATE TABLE events(id INTEGER PRIMARY KEY, ts TEXT, camera_id INTEGER, type TEXT,
        level INTEGER, track_id INTEGER, detail TEXT, snapshot TEXT, video TEXT,
        status TEXT DEFAULT 'new', handled_by TEXT, handled_ts TEXT);
      INSERT INTO events(ts, camera_id, type, level) VALUES('2026-01-01T00:00:00', 1, 'enter', 2);
    """)
    con.commit()
    con.close()
    s = S(db)
    assert s.schema_version() >= 9
    row = s.event_by_id(1)
    assert row["ai_review"] is None  # 旧数据未复核，行为不变


def test_set_ai_review(tmp_path):
    st = Store(tmp_path / "s.db")
    cid = st.add_camera("c", "D:/x.mp4", "file")
    eid = st.add_event(cid, "enter", 2, 0, "x")
    st.set_ai_review(eid, GOOD, "test-vlm")
    row = st.event_by_id(eid)
    assert json.loads(row["ai_review"])["verdict"] == "likely_false"
