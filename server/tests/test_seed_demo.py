"""单元测试：任务51 播种单源 scripts/seed_demo.py（空库播种/幂等/用户数据不覆盖）。
运行: pytest tests/test_seed_demo.py -q"""
import os
import sqlite3
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "scripts"))

import pytest  # noqa: E402

import seed_demo  # noqa: E402


@pytest.fixture()
def media(tmp_path):
    m = tmp_path / "media"
    m.mkdir()
    for _, fn, _ in seed_demo.SEED_CAMERAS:
        (m / fn).write_bytes(b"x")
    return m


def test_empty_db_seeds_4_cams(tmp_path, media):
    db = tmp_path / "seed.db"
    n = seed_demo.seed_database(db, media)
    assert n == 4
    con = sqlite3.connect(str(db))
    assert con.execute("select count(*) from cameras").fetchone()[0] == 4
    assert con.execute("select count(*) from zones").fetchone()[0] == 4
    assert con.execute("select count(*) from rules").fetchone()[0] == 8
    # 来源指向 media 绝对路径
    src = con.execute("select source from cameras limit 1").fetchone()[0]
    assert Path(src) == media / "lab-walk-24076.mp4"
    con.close()


def test_idempotent_second_call(tmp_path, media):
    db = tmp_path / "seed.db"
    assert seed_demo.seed_database(db, media) == 4
    assert seed_demo.seed_database(db, media) == 0
    con = sqlite3.connect(str(db))
    assert con.execute("select count(*) from cameras").fetchone()[0] == 4
    con.close()


def test_user_data_not_overwritten(tmp_path, media):
    db = tmp_path / "seed.db"
    seed_demo.seed_database(db, media)
    con = sqlite3.connect(str(db))
    con.execute("delete from cameras")
    con.execute("insert into cameras(name, source, type) values('user-cam','X:/c.mp4','file')")
    con.commit()
    con.close()
    assert seed_demo.seed_database(db, media) == 0
    con = sqlite3.connect(str(db))
    names = [r[0] for r in con.execute("select name from cameras").fetchall()]
    con.close()
    assert names == ["user-cam"]


def test_missing_media_skips(tmp_path):
    db = tmp_path / "seed.db"
    assert seed_demo.seed_database(db, tmp_path / "nope") == 0
    con = sqlite3.connect(str(db))
    assert con.execute("select count(*) from cameras").fetchone()[0] == 0
    # schema 已建（表存在）
    assert con.execute("select count(*) from sqlite_master where type='table'").fetchone()[0] >= 6
    con.close()


def test_schema_matches_store():
    """seed_demo.SCHEMA 的表/索引集合与 server/app/store.py 的 SCHEMA 一致。"""
    server = Path(__file__).resolve().parent.parent / "server"
    sys.path.insert(0, str(server))
    try:
        from app import store as store_mod
        import re

        def objects(schema_sql):
            return sorted(re.findall(r"CREATE (?:TABLE|INDEX) IF NOT EXISTS (\w+)", schema_sql))
        assert objects(seed_demo.SCHEMA) == objects(store_mod._SCHEMA)
    finally:
        sys.path.pop(sys.path.index(str(server)))
