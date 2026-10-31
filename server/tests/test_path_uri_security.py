"""单元测试：任务76 路径/URI 安全加固（_safe_within 语义 / 特殊字符路径 / _safe_entry 边界）。"""
import importlib.util
import os
import sqlite3
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402

from app.retention import _safe_within  # noqa: E402
from app.store import Store, ro_uri  # noqa: E402


def test_safe_within_semantics(tmp_path):
    base = tmp_path / "data"
    base.mkdir(exist_ok=True)  # conftest autouse 已建同名目录
    sub = base / "snaps" / "ev.jpg"
    sub.parent.mkdir(parents=True)
    sub.write_bytes(b"x")
    assert _safe_within(str(sub), base) is True
    # 兄弟前缀目录（data-evil 与 data 同级）→ False
    evil = tmp_path / "data-evil" / "x.jpg"
    evil.parent.mkdir(parents=True)
    evil.write_bytes(b"x")
    assert _safe_within(str(evil), base) is False
    # .. 穿越与绝对越界 → False
    assert _safe_within(str(base / ".." / ".." / "windows" / "x"), base) is False
    assert _safe_within(r"C:\Windows\System32\x", base) is False
    # 不存在文件 → False 不崩
    assert _safe_within(str(base / "nope" / "x.jpg"), base) is False
    # parent 同目录文件 → True
    f = base / "db.sqlite"
    f.write_bytes(b"x")
    assert _safe_within(str(f), base) is True


def test_ro_uri_special_chars(tmp_path):
    """临时目录含空格+中文+#：ro_uri 编码后只读连接成功（f-string URI 会挂）。"""
    d = tmp_path / "数据 目录 #test"
    d.mkdir(parents=True)
    db = d / "labsentinel.db"
    con = sqlite3.connect(db)
    con.execute("create table t(a)")
    con.commit()
    con.close()
    # as_uri 对空格/中文/# 全部百分号编码（# 不编码会被 URI fragment 截断）
    import urllib.parse
    assert "%23" in ro_uri(db) and "%20" in ro_uri(db)
    assert urllib.parse.unquote(ro_uri(db)).endswith("数据 目录 #test/labsentinel.db?mode=ro")
    ro = sqlite3.connect(ro_uri(db), uri=True)
    assert ro.execute("select count(*) from t").fetchone()[0] == 0
    ro.close()


def test_store_readonly_special_chars(tmp_path):
    from app import store as store_mod
    d = tmp_path / "数 据#目录"
    d.mkdir(parents=True)
    db = d / "s.db"
    s = Store(str(db))
    with s._conn() as c:
        c.execute("insert into cameras(name, source, type) values('中文机位','x','file')")
    con = store_mod.connect(str(db), readonly=True)  # 模块级 helper（非 Store 方法）
    assert con.execute("select name from cameras").fetchone()[0] == "中文机位"
    con.close()


def _load(name):
    import importlib.util
    spec = importlib.util.spec_from_file_location(
        name, os.path.join(os.path.dirname(__file__), "..", "..", "scripts", f"{name}.py"))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def test_backup_safe_entry_edges(tmp_path):
    bd = _load("backup_data")
    assert not bd._safe_entry(r"..\evil.txt")   # 反斜杠 .. 穿越
    assert not bd._safe_entry("C:foo.txt")      # 盘符相对路径
    assert not bd._safe_entry("/abs/evil.txt")
    assert bd._safe_entry("media/snapshots/中文-1.jpg")  # 正常中文条目放行


def test_doctor_and_collect_special_chars(tmp_path, capsys):
    """目录含空格/中文/# 时 doctor 完整性检查与 collect_diagnostics 打包均成功。"""
    r = tmp_path / "pkg"
    (r / "runtime").mkdir(parents=True)
    (r / "runtime" / "python.exe").write_bytes(b"x")
    (r / "runtime" / "pythonw.exe").write_bytes(b"x")
    (r / "app" / "server" / "app").mkdir(parents=True)
    (r / "app" / "server" / "app" / "main.py").write_text("#", encoding="ascii")
    (r / "app" / "web").mkdir(parents=True)
    (r / "app" / "web" / "index.html").write_text("<html>", encoding="ascii")
    (r / "models").mkdir()
    (r / "models" / "yolo11n_ppe.onnx").write_bytes(b"0" * 2_000_000)
    (r / "models" / "yolo11n_ppe.classes.txt").write_text("person\n", encoding="ascii")
    (r / "tools").mkdir(); (r / "tools" / "go2rtc.exe").write_bytes(b"x")
    (r / "tools" / "ffmpeg.exe").write_bytes(b"x")
    data = r / "_userdata" / "LabSentinel" / "data"
    (data / "logs").mkdir(parents=True)
    (data / "logs" / "server.log").write_text("log line\n", encoding="utf-8")
    con = sqlite3.connect(data / "labsentinel.db")
    con.execute("create table cameras(id integer primary key, name text, type text, status text, source text)")
    con.commit(); con.close()
    doctor = _load("doctor")
    res = doctor.run_checks(r, port=8765)
    dbcheck = next(x for x in res if x["name"] == "数据库")
    assert dbcheck["status"] == "PASS", dbcheck
    cd = _load("collect_diagnostics")
    out = tmp_path / "diag.zip"
    old = sys.argv
    sys.argv = ["collect_diagnostics.py", "--out", str(out), "--data-dir", str(data)]
    try:
        assert cd.main() == 0
    finally:
        sys.argv = old
    import zipfile
    with zipfile.ZipFile(out) as z:
        assert "counts.txt" in z.namelist()
