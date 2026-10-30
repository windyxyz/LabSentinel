"""单元测试：任务62 备份/恢复 CLI（热备份/校验/保守恢复/路径穿越防护）。
运行: pytest tests/test_backup_restore.py -q"""
import json
import os
import sqlite3
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "scripts"))

import pytest  # noqa: E402

import backup_data as bd  # noqa: E402


def make_data(tmp_path: Path, with_media: bool = True) -> Path:
    d = tmp_path / "data"
    d.mkdir(exist_ok=True)  # conftest autouse 已建同名目录树（105 补丁 P1-2）
    con = sqlite3.connect(d / "labsentinel.db")
    con.execute("create table cameras(id integer primary key, name text)")
    con.execute("create table events(id integer primary key, camera_id integer, detail text)")
    con.execute("insert into cameras values(1,'前门')")
    con.executemany("insert into events values(?,?,?)",
                    [(i, 1, f"ev{i}") for i in range(1, 6)])
    con.commit()
    con.close()
    if with_media:
        s = d / "snapshots"
        s.mkdir(exist_ok=True)
        (s / "snap-中文-1.jpg").write_bytes(b"\xff\xd8fake")
        v = d / "videos"
        v.mkdir(exist_ok=True)
        (v / "ev-1.mp4").write_bytes(b"0" * 1024)
    return d


def run(argv: list[str]) -> int:
    argv_backup = sys.argv
    sys.argv = ["backup_data.py", *argv]
    try:
        return bd.main()
    finally:
        sys.argv = argv_backup


def test_backup_verify_restore_roundtrip(tmp_path):
    data = make_data(tmp_path)
    out = tmp_path / "b.zip"
    assert run(["backup", "--data-dir", str(data), "--out", str(out)]) == 0
    assert run(["verify", "--zip", str(out)]) == 0
    with zipfile.ZipFile(out) as z:
        m = json.loads(z.read("manifest.json"))
        assert m["media_files"] == 2 and m["integrity_check"] == "ok"
        assert m["db_sha256"] == bd.sha256_bytes(z.read("labsentinel.db"))
    # 删原数据 → 恢复 → 行数与文件一致
    import shutil
    shutil.rmtree(data)
    target = tmp_path / "restored"
    assert run(["restore", "--zip", str(out), "--data-dir", str(target)]) == 0
    con = sqlite3.connect(target / "labsentinel.db")
    assert con.execute("select count(*) from events").fetchone()[0] == 5
    con.close()
    assert (target / "snapshots" / "snap-中文-1.jpg").read_bytes() == b"\xff\xd8fake"
    assert (target / "videos" / "ev-1.mp4").stat().st_size == 1024


def test_no_media_backup_and_restore(tmp_path):
    data = make_data(tmp_path)
    out = tmp_path / "b.zip"
    assert run(["backup", "--data-dir", str(data), "--out", str(out), "--no-media"]) == 0
    with zipfile.ZipFile(out) as z:
        assert not [n for n in z.namelist() if n.startswith("media/")]
    target = tmp_path / "restored"
    assert run(["restore", "--zip", str(out), "--data-dir", str(target), "--no-media"]) == 0
    assert not (target / "snapshots").exists()


def test_restore_refuses_and_force_baks(tmp_path):
    data = make_data(tmp_path)
    out = tmp_path / "b.zip"
    assert run(["backup", "--data-dir", str(data), "--out", str(out)]) == 0
    target = tmp_path / "t"
    target.mkdir()
    (target / "labsentinel.db").write_bytes(b"old")
    assert run(["restore", "--zip", str(out), "--data-dir", str(target)]) == 1  # 无 force 拒绝
    assert (target / "labsentinel.db").read_bytes() == b"old"
    # 2026-10-30 审阅修复：保守恢复"不删目标多出的文件"原先没被真正测到（恒真断言）——
    # 先放一个额外文件，restore 后它必须仍在
    (target / "user-extra.txt").write_text("x", encoding="ascii")
    assert run(["restore", "--zip", str(out), "--data-dir", str(target), "--force"]) == 0
    assert len(list(target.glob("labsentinel.db.bak-*"))) == 1
    assert (target / "user-extra.txt").read_text(encoding="ascii") == "x"


def test_bak_retention_3(tmp_path):
    target = tmp_path / "t"
    target.mkdir()
    db = target / "labsentinel.db"
    for i in range(5):
        db.write_bytes(b"x")
        bd._prune_baks(db)
        (target / f"labsentinel.db.bak-{i:08d}").write_bytes(b"b")
    bd._prune_baks(db)
    assert len(list(target.glob("labsentinel.db.bak-*"))) == 3


@pytest.mark.parametrize("evil", ["../evil.txt", "C:/abs/evil.txt", "/abs/evil.txt"])
def test_path_traversal_rejected(tmp_path, evil):
    zp = tmp_path / "evil.zip"
    with zipfile.ZipFile(zp, "w") as z:
        z.writestr("manifest.json", json.dumps({"db_bytes": 0, "db_sha256": "0", "tool_version": "t"}))
        z.writestr(evil, b"x")
    assert run(["verify", "--zip", str(zp)]) == 1


def test_corrupt_inputs(tmp_path):
    data = make_data(tmp_path)
    out = tmp_path / "b.zip"
    assert run(["backup", "--data-dir", str(data), "--out", str(out)]) == 0
    # 篡改 db 字节（sha256 不匹配）
    raw = out.read_bytes()
    with zipfile.ZipFile(out) as z:
        names = z.namelist()
    import io
    buf = io.BytesIO()
    with zipfile.ZipFile(out) as zin, zipfile.ZipFile(buf, "w") as zout:
        for n in names:
            data_b = zin.read(n)
            if n == "labsentinel.db":
                data_b += b"\x00"
            zout.writestr(n, data_b)
    out.write_bytes(buf.getvalue())
    assert run(["verify", "--zip", str(out)]) == 1
    # 缺 manifest
    bad = tmp_path / "bad.zip"
    with zipfile.ZipFile(bad, "w") as z:
        z.writestr("labsentinel.db", b"x")
    assert run(["verify", "--zip", str(bad)]) == 1
    # 非 zip 文件
    junk = tmp_path / "junk.zip"
    junk.write_bytes(b"not a zip")
    assert run(["verify", "--zip", str(junk)]) == 1


# ---- 105 补丁 P0-3：restore 顺序重排（拒绝不碰文件/checkpoint 后清理 WAL/孤儿 WAL 清理）----

def test_restore_refuse_keeps_wal(tmp_path):
    """有 WAL、无 --force 拒绝覆盖：返回失败且 WAL 文件原样保留。"""
    data = make_data(tmp_path, with_media=False)
    out = tmp_path / "b.zip"
    assert run(["backup", "--data-dir", str(data), "--out", str(out)]) == 0
    target = tmp_path / "t"
    target.mkdir()
    (target / "labsentinel.db").write_bytes(b"old-db")
    wal = target / "labsentinel.db-wal"
    wal.write_bytes(b"WAL-CONTENT-MUST-SURVIVE")
    rc = run(["restore", "--zip", str(out), "--data-dir", str(target)])
    assert rc == 1
    assert wal.read_bytes() == b"WAL-CONTENT-MUST-SURVIVE"  # 拒绝分支不碰任何文件
    assert (target / "labsentinel.db").read_bytes() == b"old-db"


def test_restore_force_checkpoints_and_cleans_wal(tmp_path):
    """--force：checkpoint 后 .bak 生成、WAL/SHM 清理、新库 integrity ok。"""
    import sqlite3
    data = make_data(tmp_path, with_media=False)
    out = tmp_path / "b.zip"
    assert run(["backup", "--data-dir", str(data), "--out", str(out)]) == 0
    target = tmp_path / "t"
    target.mkdir()
    con = sqlite3.connect(target / "labsentinel.db")
    con.execute("create table x(a)")  # 现有库带 WAL 活动
    con.execute("insert into x values(1)")
    con.commit()
    con.close()
    assert run(["restore", "--zip", str(out), "--data-dir", str(target), "--force"]) == 0
    assert list(target.glob("labsentinel.db.bak-*"))
    assert not (target / "labsentinel.db-wal").exists()
    assert not (target / "labsentinel.db-shm").exists()
    con = sqlite3.connect(target / "labsentinel.db")
    assert con.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    con.close()


def test_restore_orphan_wal_cleaned(tmp_path):
    """目标无 db 但有孤儿 WAL：恢复前清理，不回放进新库。"""
    data = make_data(tmp_path, with_media=False)
    out = tmp_path / "b.zip"
    assert run(["backup", "--data-dir", str(data), "--out", str(out)]) == 0
    target = tmp_path / "t"
    target.mkdir()
    (target / "labsentinel.db-wal").write_bytes(b"orphan")
    assert run(["restore", "--zip", str(out), "--data-dir", str(target)]) == 0
    assert not (target / "labsentinel.db-wal").exists()
