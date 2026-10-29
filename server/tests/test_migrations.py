"""单元测试：任务69 schema 版本化迁移框架（新库/旧库/幂等/失败回滚）。
运行: pytest tests/test_migrations.py -q"""
import os
import sqlite3
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import store as store_mod  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402


def test_fresh_db_at_latest(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "db_path", tmp_path / "s.db")
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    s = Store(tmp_path / "s.db")
    assert s.schema_version() == store_mod.SCHEMA_VERSION_LATEST
    con = sqlite3.connect(tmp_path / "s.db")
    cols = {r[1] for r in con.execute("PRAGMA table_info(events)")}
    assert {"video", "status", "handled_by", "handled_ts"} <= cols
    rows = con.execute("select count(*) from schema_version").fetchone()[0]
    con.close()
    assert rows == len(store_mod.MIGRATIONS)


def test_legacy_partial_db_migrates(tmp_path, monkeypatch):
    """最小旧库（events 无四列）→ 打开 Store 补列 + 旧数据保留 + 版本写入。"""
    db = tmp_path / "legacy.db"
    con = sqlite3.connect(db)
    con.execute("CREATE TABLE cameras(id INTEGER PRIMARY KEY, name TEXT, source TEXT, type TEXT, status TEXT)")
    con.execute("CREATE TABLE events(id INTEGER PRIMARY KEY, ts TEXT, camera_id INTEGER, type TEXT,"
                " level INTEGER, track_id INTEGER, detail TEXT, snapshot TEXT)")
    con.execute("insert into events(ts, camera_id, type, level, track_id, detail) "
                "values('2026-01-01T00:00:00', 1, 'enter', 2, 0, 'legacy-row')")
    con.commit()
    con.close()
    monkeypatch.setattr(settings, "db_path", db)
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    s = Store(db)
    con = sqlite3.connect(db)
    cols = {r[1] for r in con.execute("PRAGMA table_info(events)")}
    assert {"video", "status", "handled_by", "handled_ts"} <= cols
    n, status = con.execute("select count(*), max(status) from events").fetchone()
    con.close()
    assert n == 1 and status == "new"  # 旧行保留，新列默认值生效
    assert s.schema_version() == store_mod.SCHEMA_VERSION_LATEST


def test_full_legacy_db_marked_latest(tmp_path, monkeypatch):
    """无 schema_version 但结构已最新的旧库 → 探测标记，不重跑迁移。"""
    db = tmp_path / "oldfull.db"
    con = sqlite3.connect(db)
    con.executescript(store_mod._SCHEMA)  # 目标结构已齐
    con.close()
    monkeypatch.setattr(settings, "db_path", db)
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    s = Store(db)
    assert s.schema_version() == store_mod.SCHEMA_VERSION_LATEST


def test_idempotent_reopen(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "db_path", tmp_path / "s.db")
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    v1 = Store(tmp_path / "s.db").schema_version()
    s2 = Store(tmp_path / "s.db")
    con = sqlite3.connect(tmp_path / "s.db")
    rows = con.execute("select count(*) from schema_version").fetchone()[0]
    con.close()
    assert s2.schema_version() == v1 and rows == len(store_mod.MIGRATIONS)  # 不重复插版本行


def test_failed_migration_rolls_back(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "db_path", tmp_path / "s.db")
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    Store(tmp_path / "s.db")  # 先建到最新
    base_version = store_mod.SCHEMA_VERSION_LATEST

    def boom(c):
        c.execute("CREATE TABLE t69(x)")
        raise RuntimeError("boom-69")

    fake = [(base_version + 1, "必失败迁移", boom)]
    monkeypatch.setattr(store_mod, "MIGRATIONS", fake)
    con = sqlite3.connect(tmp_path / "s.db")
    with pytest.raises(RuntimeError, match=f"v{base_version + 1}.*boom-69"):
        store_mod.Store._migrate(con)  # 对现有库直接跑迁移器触发失败迁移
    ver = con.execute("select max(version) from schema_version").fetchone()[0]
    tables = {r[0] for r in con.execute("select name from sqlite_master where type='table'")}
    con.close()
    assert ver == base_version  # 失败版本未写入
    assert "t69" not in tables  # 事务回滚：失败迁移内的建表也没留下


def test_system_route_schema_version(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "db_path", tmp_path / "s.db")
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    with TestClient(app) as c:
        d = c.get("/api/system").json()
        assert d["schema_version"] == store_mod.SCHEMA_VERSION_LATEST
