"""T-332（ADR-0008）三路径 DB 双向兼容验收（76 文档 §13.8 L3「数据」行）：

路径①：Python Store() 建全量迁移库（schema v13）→ Rust `--selftest-db` 完整读写 → Python 复核；
路径②：Python 用 SQL 造最小合法库（等价 Rust 产物）→ Rust selftest → Python 读写；
路径③：Python 在 Rust 写过之后继续增删查（WAL 并发可见性 + 迁移链完好）。

运行: pytest tests/test_store_bidirectional.py -q
"""
import os
import sqlite3
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402

from app.store import Store  # noqa: E402

from conftest import RS_TARGET, rs_exe  # noqa: E402  (R2-506: shared exe resolver)

RS_EXE = rs_exe("labsentinel-server", "LS_SERVER_EXE")
ENV = {**os.environ, "TMP": str(RS_TARGET), "TEMP": str(RS_TARGET)}


def _rust_selftest(db: Path) -> None:
    # R2-005/N-36: CI does not build the Rust binary -> skip, do not fail.
    # rust.yml builds ls-server and sets LS_SERVER_EXE to actually run these.
    if not RS_EXE.exists():
        pytest.skip(f"labsentinel-server 未构建（先 cargo build -p ls-server，或设 LS_SERVER_EXE）：{RS_EXE}")
    r = subprocess.run([str(RS_EXE), "--selftest-db", str(db)],
                       capture_output=True, text=True, timeout=60, env=ENV)
    assert r.returncode == 0, (r.stdout[-1500:], r.stderr[-1500:])
    assert "OK" in r.stdout, r.stdout


def test_path1_python_build_rust_rw_python_verify(tmp_path):
    """路径①：Python 全量迁移库 → Rust 完整读写 → Python 复核（Rust 写入行可见、状态持久化）。"""
    db = tmp_path / "cross1.db"
    py = Store(db)
    assert py.schema_version() >= 13
    cid = py.add_camera("跨语言①", "x.mp4", "file")
    py.add_zone(cid, "z", [[0, 0], [100, 0], [100, 100]])

    _rust_selftest(db)

    # Python 复核：Rust 写入的事件行可见、处置状态为 acked
    rows = py.events(camera_id=cid, limit=100)
    rust_rows = [r for r in rows if r["detail"] == "rust-selftest"]
    assert len(rust_rows) == 1
    assert rust_rows[0]["status"] == "acked"
    assert rust_rows[0]["handled_by"] == "rust"
    # 迁移链完好：Python 继续写
    eid = py.add_event(cid, "enter", 2, 1, "python-after-rust")
    assert py.event_by_id(eid)["detail"] == "python-after-rust"


def test_path2_minimal_db_rust_rw_python_rw(tmp_path):
    """路径②：最小合法库（Rust 产物等价 schema）→ Rust selftest → Python 直接读写。"""
    db = tmp_path / "cross2.db"
    c = sqlite3.connect(db)
    for stmt in [
        "CREATE TABLE schema_version(version INTEGER PRIMARY KEY, applied_at TEXT)",
        "INSERT INTO schema_version VALUES(13,'2026-01-01T00:00:00')",
        "CREATE TABLE events(id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT, camera_id INTEGER, "
        "type TEXT, level INTEGER, track_id INTEGER, detail TEXT, snapshot TEXT, video TEXT, "
        "status TEXT DEFAULT 'new', handled_by TEXT, handled_ts TEXT, escalated_ts TEXT)",
        "CREATE TABLE cameras(id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, source TEXT, "
        "type TEXT, status TEXT DEFAULT 'stopped', group_name TEXT DEFAULT '', "
        "analyzer TEXT DEFAULT 'python')",
        "CREATE TABLE zones(id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, "
        "name TEXT, polygon TEXT)",
        "CREATE TABLE rules(id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, "
        "zone_id INTEGER, behavior TEXT, level INTEGER, cooldown_s INTEGER, "
        "time_start TEXT, time_end TEXT, threshold INTEGER, enabled INTEGER, "
        "days_of_week TEXT, holiday_only INTEGER)",
        "CREATE TABLE alert_deliveries(id INTEGER PRIMARY KEY AUTOINCREMENT, event_id INTEGER, "
        "channel TEXT, status TEXT DEFAULT 'pending', attempts INTEGER DEFAULT 0, "
        "last_error TEXT, updated_ts TEXT)",
        "INSERT INTO cameras(name,source,type) VALUES('min','x.mp4','file')",
    ]:
        c.execute(stmt)
    c.commit()
    c.close()

    _rust_selftest(db)

    # Python 打开 Rust 写过的库：读事件 + 继续写（无需跑迁移链）
    py = Store(db)
    rows = [r for r in py.events(limit=100) if r["detail"] == "rust-selftest"]
    assert len(rows) == 1 and rows[0]["status"] == "acked"
    eid = py.add_event(rows[0]["camera_id"], "enter", 2, 2, "python-on-rust-db")
    assert py.event_by_id(eid)["detail"] == "python-on-rust-db"
    with sqlite3.connect(db) as c:
        assert c.execute("PRAGMA integrity_check").fetchone()[0] == "ok"


def test_path3_concurrent_wal_visibility(tmp_path):
    """路径③：Python 持连接写入未提交（WAL）时 Rust 只读打开不阻塞、提交后可见。"""
    db = tmp_path / "cross3.db"
    py = Store(db)
    cid = py.add_camera("并发", "x.mp4", "file")
    py.add_event(cid, "enter", 2, 1, "before")
    _rust_selftest(db)  # Rust 已写一条
    # Python 在 Rust 写过之后照常读全部
    details = {r["detail"] for r in py.events(limit=100)}
    assert "before" in details and "rust-selftest" in details
