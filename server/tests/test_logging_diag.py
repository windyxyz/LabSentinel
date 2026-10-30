"""单元测试：任务61 文件日志轮转 + 诊断聚合 + 诊断包（脱敏）。
运行: pytest tests/test_logging_diag.py -q"""
import logging
import os
import sqlite3
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "scripts"))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import logging_setup as ls  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402

import collect_diagnostics as cd  # noqa: E402


@pytest.fixture()
def log_env(tmp_path, monkeypatch):
    """临时 data_dir + 打开文件日志 + 隔离 ring/handler 状态。"""
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    monkeypatch.setattr(ls, "_ring", __import__("collections").deque(maxlen=200))
    old_fh = ls._file_handler
    ls._file_handler = None
    monkeypatch.setenv("LS_LOG_FILE", "1")
    monkeypatch.delenv("LS_LOG_LEVEL", raising=False)
    yield tmp_path
    if ls._file_handler is not None:
        logging.getLogger().removeHandler(ls._file_handler)
        ls._file_handler.close()
        ls._file_handler = None
    ls._file_handler = old_fh


def test_setup_creates_file_and_idempotent(log_env):
    p1 = ls.setup_file_logging()
    n1 = len(logging.getLogger().handlers)
    p2 = ls.setup_file_logging()
    n2 = len(logging.getLogger().handlers)
    assert p1 == p2 and p1.exists()
    assert n1 == n2  # 幂等：不重复加 handler
    h = ls._file_handler
    assert h.maxBytes == 5 * 1024 * 1024 and h.backupCount == 3


def test_log_file_disabled(log_env, monkeypatch):
    monkeypatch.setenv("LS_LOG_FILE", "0")
    assert ls.setup_file_logging() is None
    assert not (log_env / "logs" / "server.log").exists()


def test_rotation(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "data_dir", tmp_path)
    monkeypatch.setenv("LS_LOG_FILE", "1")
    old = ls._file_handler
    ls._file_handler = None
    try:
        p = ls.setup_file_logging()
        h = ls._file_handler
        h.maxBytes = 2048  # 临时调小验证轮转
        lg = logging.getLogger("rotate61")
        for i in range(200):
            lg.warning("x" * 100 + f" {i}")
        h.flush()
        assert any(p.parent.glob("server.log.*")), "应产生 .1 轮转文件"
    finally:
        logging.getLogger().removeHandler(ls._file_handler)
        ls._file_handler.close()
        ls._file_handler = None
        ls._file_handler = old


def test_ring_captures_error(log_env):
    ls.setup_file_logging()
    logging.getLogger("boom61").error("test-error-marker-61")
    errs = ls.recent_errors()
    assert any("test-error-marker-61" in e for e in errs)


def test_redaction():
    # 75.4 规格清单：全部 ***
    for k in ("LS_SMTP_PASS", "LS_ADMIN_TOKEN", "LS_WEBHOOK_SECRET", "LS_QMSG_KEY"):
        assert cd.redact_line(k, "supersecret") == f"{k}=***", k
    assert cd.redact_line("smtp_pass", "supersecret") == "smtp_pass=***"
    # 普通键不误脱敏
    assert cd.redact_line("LS_SMTP_HOST", "smtp.qq.com") == "LS_SMTP_HOST=smtp.qq.com"
    assert cd.redact_line("LS_SMTP_USER", "user@qq.com") == "LS_SMTP_USER=user@qq.com"
    assert cd.redact_line("LS_PPE_CONF", "0.3") == "LS_PPE_CONF=0.3"  # 普通键不受影响
    # URL 凭证与 query-token
    out = cd.redact_line("LS_RTSP_URL", "rtsp://admin:pass@192.168.1.3:554/ch1")
    assert "admin" not in out and "pass" not in out and "192.168.1.3:554/ch1" in out
    q = cd.redact_value("https://hook.internal/x?token=tok123&x=1")
    assert "tok123" not in q and "x=1" in q
    assert cd.sanitize_source("rtsp://u:p@1.2.3.4:554/x") == "rtsp://1.2.3.4:554"
    assert cd.sanitize_source("file:///D:/x.mp4") == "***"  # file 无主机 → 只 ***


def test_summary_endpoint(log_env):
    ls.setup_file_logging()
    logging.getLogger("boom61b").error("diag-error-marker-61")
    from app.store import Store
    st = Store()  # settings.data_dir 已指向临时目录
    import app.api as api_mod
    old = api_mod.store
    api_mod.store = st
    try:
        with TestClient(app) as c:
            r = c.get("/api/diagnostics/summary")
            assert r.status_code == 200
            d = r.json()
            for k in ("version", "backend", "uptime_s", "counts", "disk", "logs", "recent_errors"):
                assert k in d
            assert d["counts"]["cameras"] >= 0
            assert any("diag-error-marker-61" in e for e in d["recent_errors"])
    finally:
        api_mod.store = old


def test_collect_zip(tmp_path):
    data = tmp_path / "data"
    (data / "logs").mkdir(parents=True)
    con = sqlite3.connect(data / "labsentinel.db")
    con.execute("create table cameras(id integer primary key, name text, type text, status text, source text)")
    con.execute("insert into cameras values(1,'前门','rtsp','running','rtsp://admin:pw@10.0.0.5:554/1')")
    con.commit()
    con.close()
    (data / "logs" / "server.log").write_text("2026-10-30 INFO line\n" * 20, encoding="utf-8")
    out = tmp_path / "diag.zip"
    # 以命令行参数驱动 main()
    sys_argv = sys.argv
    sys.argv = ["collect_diagnostics.py", "--out", str(out), "--data-dir", str(data), "--log-lines", "10"]
    try:
        assert cd.main() == 0
    finally:
        sys.argv = sys_argv
    with zipfile.ZipFile(out) as z:
        names = set(z.namelist())
        assert {"README.txt", "system.txt", "config.txt", "counts.txt",
                "cameras.txt", "logs/server.log.tail.txt"} <= names
        cams = z.read("cameras.txt").decode("utf-8")
        assert "admin" not in cams and "pw" not in cams and "10.0.0.5" in cams
        counts = z.read("counts.txt").decode("utf-8")
        assert "cameras=1" in counts
        assert not any(n.endswith(".db") for n in names)  # 绝不整库打包
    # --out 已存在默认拒绝
    sys.argv = ["collect_diagnostics.py", "--out", str(out), "--data-dir", str(data)]
    try:
        assert cd.main() == 1
    finally:
        sys.argv = sys_argv
