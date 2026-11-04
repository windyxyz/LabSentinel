"""单元测试：任务132 AI 复核徽标与筛选（store/API 过滤、非法值 400、旧数据 NULL 兼容）。
运行: pytest tests/test_ai_filter.py -q"""
import json

import pytest
from fastapi.testclient import TestClient

from app import api as api_mod
from app.main import app
from app.store import Store


def _review(verdict):
    return json.dumps({"verdict": verdict, "reason": "r", "confidence": 0.8})


@pytest.fixture()
def st(tmp_path):
    s = Store(tmp_path / "s.db")
    cid = s.add_camera("c", "D:/x.mp4", "file")
    ids = []
    for verdict, status in (("likely_false", "new"), ("likely_true", "new"),
                            ("uncertain", "acked"), (None, "new")):
        eid = s.add_event(cid, "enter_no_ppe", 3, 0, f"ev-{verdict}")
        if verdict:
            s.set_ai_review(eid, _review(verdict), "test-vlm")
        if status != "new":
            s.set_event_status(eid, status)
        ids.append((eid, verdict))
    return s, cid, ids


def test_store_filter(st):
    s, cid, _ = st
    lf = [e["id"] for e in s.events(ai_verdict="likely_false")]
    lt = [e["id"] for e in s.events(ai_verdict="likely_true")]
    un = [e["id"] for e in s.events(ai_verdict="uncertain")]
    none = [e["id"] for e in s.events(ai_verdict="none")]
    all_rows = s.events()
    assert len(all_rows) == 4
    assert len(lf) == 1 and len(lt) == 1 and len(un) == 1 and len(none) == 1
    assert set(lf) | set(lt) | set(un) | set(none) == {e["id"] for e in all_rows}
    # count 与列表一致
    assert s.count_events(ai_verdict="likely_false") == 1
    assert s.count_events(ai_verdict="none") == 1
    # 组合筛选：uncertain + status=acked → 1；uncertain + status=new → 0
    combo = s.events(ai_verdict="uncertain", status="acked")
    assert len(combo) == 1
    assert s.events(ai_verdict="uncertain", status="new") == []
    # camera 组合
    assert s.events(ai_verdict="likely_true", camera_id=cid)


def test_store_no_filter_unchanged(st):
    s, _, _ = st
    assert len(s.events()) == 4  # 默认查询不受影响
    assert s.count_events() == 4


# ---- API ----

@pytest.fixture()
def client():
    with TestClient(app) as tc:
        yield tc


def test_api_filter_and_400(client, tmp_path):
    st = Store(api_mod.settings.db_path)  # lifespan init 的库（conftest 隔离到 tmp）
    cid = st.add_camera("c2", "D:/x.mp4", "file")
    e1 = st.add_event(cid, "enter_no_ppe", 3, 0, "误报候选")
    st.set_ai_review(e1, _review("likely_false"), "v")
    e2 = st.add_event(cid, "enter", 2, 0, "未复核")
    rows = client.get("/api/events?ai_verdict=likely_false").json()
    assert [r["id"] for r in rows] == [e1]
    assert rows[0]["ai_verdict"] == "likely_false"
    assert rows[0]["ai_review"]["confidence"] == 0.8
    assert client.get("/api/events?ai_verdict=none").json()[0]["id"] == e2
    assert client.get("/api/events/count?ai_verdict=likely_false").json()["count"] == 1
    assert client.get("/api/events/count?ai_verdict=none").json()["count"] == 1
    for bad in ("hacked", "likely_false;drop", ""):
        r = client.get(f"/api/events?ai_verdict={bad}")
        assert r.status_code == 400, bad
    assert client.get("/api/events/count?ai_verdict=x").status_code == 400
    # 组合 camera/status
    combo = client.get(f"/api/events?ai_verdict=likely_false&camera_id={cid}&status=new")
    assert combo.status_code == 200


def test_old_db_rows_null(tmp_path):
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
    s = Store(db)  # 迁移加列后旧行 NULL
    assert s.events(ai_verdict="none")
    assert s.events(ai_verdict="likely_false") == []
