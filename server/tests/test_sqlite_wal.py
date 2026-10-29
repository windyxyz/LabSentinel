"""单元测试：任务64 SQLite WAL + busy_timeout + 并发读写。
运行: pytest tests/test_sqlite_wal.py -q"""
import logging
import os
import sqlite3
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402

from app.store import Store, connect  # noqa: E402


@pytest.fixture()
def store(tmp_path):
    s = Store(tmp_path / "wal.db")
    yield s
    # 收尾：关闭连接后 checkpoint，清掉 -wal/-shm
    try:
        c = sqlite3.connect(s.path)
        c.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        c.close()
    except sqlite3.Error:
        pass


def test_wal_mode_and_busy_timeout(store):
    con = connect(store.path)
    try:
        assert con.execute("PRAGMA journal_mode").fetchone()[0].lower() == "wal"
        assert con.execute("PRAGMA busy_timeout").fetchone()[0] == 5000
        assert con.execute("PRAGMA synchronous").fetchone()[0] == 1  # NORMAL
    finally:
        con.close()


def test_concurrent_writers_readers_no_locked(store):
    """4 写线程 × 50 条 + 2 读线程持续读：无 database is locked，最终 200 行。"""
    errors: list[str] = []
    stop = threading.Event()

    def writer(wid: int):
        for i in range(50):
            try:
                with store._conn() as c:
                    c.execute("insert into events(ts, camera_id, type, level, detail) "
                              "values(?,?,?,?,?)",
                              (f"2026-10-30T0{wid}:00:00", 1, "enter", 2, f"w{wid}-{i}"))
            except sqlite3.OperationalError as e:
                errors.append(f"writer{wid}: {e}")
                return

    def reader(rid: int):
        while not stop.is_set():
            try:
                with store._conn() as c:
                    c.execute("select count(*) from events").fetchone()
            except sqlite3.OperationalError as e:
                errors.append(f"reader{rid}: {e}")
                return
            time.sleep(0.001)

    readers = [threading.Thread(target=reader, args=(i,), daemon=True) for i in range(2)]
    for t in readers:
        t.start()
    t0 = time.time()
    writers = [threading.Thread(target=writer, args=(i,)) for i in range(4)]
    for t in writers:
        t.start()
    for t in writers:
        t.join()
    elapsed = time.time() - t0
    stop.set()
    for t in readers:
        t.join(timeout=5)
    assert not errors, f"出现 OperationalError: {errors[:3]}"
    with store._conn() as c:
        n = c.execute("select count(*) from events").fetchone()[0]
    assert n == 200
    print(f"并发 4×50 写 + 2 读 耗时 {elapsed:.2f}s（WAL+busy_timeout）")


def test_wal_files_and_checkpoint(store):
    with store._conn() as c:
        c.execute("insert into events(ts, camera_id, type, level, detail) "
                  "values('2026-10-30T09:00:00', 1, 'enter', 2, 'wal-check')")
    # WAL 产生的 -wal/-shm 属正常文件
    assert Path(store.path + "-wal").exists() or Path(store.path + "-shm").exists()
    # 显式 checkpoint(TRUNCATE) 后 -wal 清空/消失（正常关闭即等价）
    c = sqlite3.connect(store.path)
    c.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()
    c.close()
    wal = Path(store.path + "-wal")
    assert not wal.exists() or wal.stat().st_size == 0


def test_memory_db_degrades_gracefully(caplog):
    # :memory: 每条连接是独立库（schema 只在 init 连接里）——预期只警告不崩
    with caplog.at_level(logging.WARNING, logger="app.store"):
        s = Store(":memory:")
    assert any("期望 wal" in r.message for r in caplog.records)
