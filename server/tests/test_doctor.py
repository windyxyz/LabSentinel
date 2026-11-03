"""单元测试：任务70 doctor.py 自检（最小可用包/坏项/JSON）。
运行: pytest tests/test_doctor.py -q"""
import importlib.util
import json
import os
import sqlite3
import sys
from pathlib import Path

import pytest

SPEC = importlib.util.spec_from_file_location(
    "doctor", os.path.join(os.path.dirname(__file__), "..", "..", "scripts", "doctor.py"))
doctor = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(doctor)


def make_pkg(tmp_path: Path, model_bytes: int = 2_000_000) -> Path:
    """最小"便携包"：runtime/模型/classes/web dist/媒体/工具/db。"""
    r = tmp_path / "pkg"
    (r / "runtime").mkdir(parents=True)
    (r / "runtime" / "python.exe").write_bytes(b"x")
    (r / "runtime" / "pythonw.exe").write_bytes(b"x")
    (r / "app" / "server" / "app").mkdir(parents=True)
    (r / "app" / "server" / "app" / "main.py").write_text("#", encoding="ascii")
    (r / "app" / "web").mkdir(parents=True)
    (r / "app" / "web" / "index.html").write_text("<html>", encoding="ascii")
    (r / "models").mkdir()
    (r / "models" / "yolo11n_ppe.onnx").write_bytes(b"0" * model_bytes)
    (r / "models" / "yolo11n_ppe.classes.txt").write_text("person\nlab coat\n", encoding="ascii")
    (r / "media").mkdir()
    (r / "media" / "demo.mp4").write_bytes(b"x")
    (r / "tools").mkdir()
    (r / "tools" / "go2rtc.exe").write_bytes(b"x")
    (r / "tools" / "ffmpeg").mkdir()
    (r / "tools" / "ffmpeg" / "ffmpeg.exe").write_bytes(b"x")
    (r / "_userdata" / "LabSentinel" / "data").mkdir(parents=True)
    return r


def get(names, results):
    by = {r["name"]: r for r in results}
    return [by[n] for n in names]


def test_min_pkg_all_pass(tmp_path, monkeypatch):
    r = make_pkg(tmp_path)
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "sandbox"))
    results = doctor.run_checks(r, port=8765)
    fails = [x for x in results if x["status"] == "FAIL"]
    assert fails == [], fails
    names = [x["name"] for x in results]
    assert any("模型" in n for n in names) and any("数据库" in n for n in names)


def test_missing_model_fails(tmp_path, monkeypatch):
    r = make_pkg(tmp_path)
    (r / "models" / "yolo11n_ppe.onnx").unlink()
    results = doctor.run_checks(r, port=8765)
    model = next(x for x in results if "模型" in x["name"])
    assert model["status"] == "FAIL" and "模型" in model["advice"]


def test_missing_web_dist_fails(tmp_path, monkeypatch):
    r = make_pkg(tmp_path)
    (r / "app" / "web" / "index.html").unlink()
    results = doctor.run_checks(r, port=8765)
    prog = next(x for x in results if x["name"] == "程序文件")
    assert prog["status"] == "FAIL"


def test_broken_db_fails(tmp_path, monkeypatch):
    r = make_pkg(tmp_path)
    monkeypatch.setenv("LOCALAPPDATA", str(r / "_userdata"))  # data_dir = pkg/_userdata/LabSentinel/data
    db = r / "_userdata" / "LabSentinel" / "data" / "labsentinel.db"
    db.write_bytes(b"this is not a sqlite file" * 10)
    results = doctor.run_checks(r, port=8765)
    dbcheck = next(x for x in results if x["name"] == "数据库")
    assert dbcheck["status"] == "FAIL" and "备份" in dbcheck["advice"]


def test_json_output(tmp_path, monkeypatch, capsys):
    r = make_pkg(tmp_path)
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "sandbox"))
    rc = doctor.main.__wrapped__() if hasattr(doctor.main, "__wrapped__") else None
    sys_argv = sys.argv
    sys.argv = ["doctor.py", "--root", str(r), "--port", "8765", "--json"]
    try:
        code = doctor.main()
    finally:
        sys.argv = sys_argv
    assert code == 0
    out = capsys.readouterr().out
    d = json.loads(out)
    assert d["mode"] == "portable"
    assert all({"name", "status", "detail"} <= set(x) for x in d["results"])


def test_small_model_fails_threshold(tmp_path, monkeypatch):
    r = make_pkg(tmp_path, model_bytes=100)
    results = doctor.run_checks(r, port=8765)
    model = next(x for x in results if "模型" in x["name"])
    assert model["status"] == "FAIL"


def make_source(tmp_path: Path) -> Path:
    """最小"源码布局"：server/app/main.py + web/dist + m0-spike 模型。"""
    r = tmp_path / "src"
    (r / "server" / "app").mkdir(parents=True)
    (r / "server" / "app" / "main.py").write_text("#", encoding="ascii")
    (r / "web" / "dist").mkdir(parents=True)
    (r / "web" / "dist" / "index.html").write_text("<html>", encoding="ascii")
    (r / "m0-spike").mkdir()
    (r / "m0-spike" / "yolo11n_ppe.onnx").write_bytes(b"0" * 2_000_000)
    (r / "m0-spike" / "yolo11n_ppe.classes.txt").write_text("person\n", encoding="ascii")
    return r


def test_source_mode_ignores_stale_portable_config(tmp_path, monkeypatch):
    """任务119：源码模式读 server/.env；LOCALAPPDATA 里旧便携 config.env 的坏路径不得造成假 FAIL。"""
    r = make_source(tmp_path)
    (r / "server" / ".env").write_text("LS_ADMIN_TOKEN=t\n", encoding="utf-8")
    la = tmp_path / "sandbox"
    (la / "LabSentinel").mkdir(parents=True)
    (la / "LabSentinel" / "config.env").write_text(
        "LS_MODEL_FP32=Z:/definitely/not/exist.onnx\n", encoding="utf-8")
    monkeypatch.setenv("LOCALAPPDATA", str(la))
    results = doctor.run_checks(r, port=8765)
    cfg = next(x for x in results if x["name"] == "配置")
    assert cfg["status"] == "PASS", cfg


def test_source_mode_prefers_server_env(tmp_path, monkeypatch):
    """任务119：源码模式两端点都在时以 server/.env 为准——.env 内坏路径要 FAIL（证明读的是它）。"""
    r = make_source(tmp_path)
    (r / "server" / ".env").write_text(
        "LS_PPE_MODEL=Z:/definitely/not/exist.onnx\n", encoding="utf-8")
    la = tmp_path / "sandbox"
    (la / "LabSentinel").mkdir(parents=True)
    (la / "LabSentinel" / "config.env").write_text("LS_ADMIN_TOKEN=t\n", encoding="utf-8")
    monkeypatch.setenv("LOCALAPPDATA", str(la))
    results = doctor.run_checks(r, port=8765)
    cfg = next(x for x in results if x["name"] == "配置")
    assert cfg["status"] == "FAIL", cfg


def test_portable_mode_reads_config_env(tmp_path, monkeypatch):
    """任务119：便携模式仍以 LOCALAPPDATA config.env 为准（坏路径 → FAIL）。"""
    r = make_pkg(tmp_path)
    la = tmp_path / "sandbox"
    (la / "LabSentinel").mkdir(parents=True)
    (la / "LabSentinel" / "config.env").write_text(
        "LS_PPE_MODEL=Z:/definitely/not/exist.onnx\n", encoding="utf-8")
    monkeypatch.setenv("LOCALAPPDATA", str(la))
    results = doctor.run_checks(r, port=8765)
    cfg = next(x for x in results if x["name"] == "配置")
    assert cfg["status"] == "FAIL", cfg
