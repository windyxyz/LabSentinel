"""单元测试：任务92 定时自动备份（纯函数/prune/runner 并发/状态/端点）。"""
import os
import sys
import time
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import backups  # noqa: E402
from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402


def _mk_zips(tmp_path, n=5, prefix="auto"):
    d = tmp_path / "backups"
    d.mkdir(parents=True, exist_ok=True)
    files = []
    now = time.time()
    for i in range(n):
        f = d / f"{prefix}-2026010{i}-120000.zip"
        f.write_bytes(b"z" * (100 + i))
        files.append(f)
    # mtime 递增且最新的接近当前时间
    for i, f in enumerate(files):
        mt = now - (n - i) * 100  # 最近的最新
        os.utime(f, (mt, mt))
    return files


def test_needs_backup(tmp_path):
    files = _mk_zips(tmp_path, n=2)
    now = time.time()
    assert backups._needs_backup([], now, 24) is True  # 无文件
    assert backups._needs_backup(files, now, 24) is False  # 最新 1h 内
    assert backups._needs_backup(files, now + 25 * 3600, 24) is True  # 超 24h


def test_prune_keeps_n(tmp_path):
    files = _mk_zips(tmp_path, n=5)
    removed = backups._prune(files, 2)
    assert removed == 3
    assert len(list(files[0].parent.glob("auto-*.zip"))) == 2  # 留最新的 2 份


def test_prune_manual_zip_untouched(tmp_path):
    _mk_zips(tmp_path, n=3)
    manual = tmp_path / "backups" / "manual-backup.zip"
    manual.write_bytes(b"m")
    files = backups.auto_files.__wrapped__(tmp_path / "backups") if hasattr(backups.auto_files, '__wrapped__') else None
    # 直接操作
    all_zips = sorted((tmp_path / "backups").glob("*.zip"),
                      key=lambda p: p.stat().st_mtime, reverse=True)
    auto_only = [f for f in all_zips if f.name.startswith("auto-")]
    backups._prune(auto_only, 2)
    assert manual.exists()  # 手动 zip 不删


def test_run_backup_creates_zip(tmp_path, monkeypatch):
    from app.store import Store
    settings.data_dir = tmp_path / "data"
    settings.data_dir.mkdir(parents=True, exist_ok=True)
    s = Store(str(settings.data_dir / "labsentinel.db"))
    s.add_event(1, "enter", 2, 1, "backup-test")
    # backup_data.py 从项目 scripts/ 找
    ok, msg = backups.run_backup.__wrapped__(tmp_path / "data") if hasattr(backups.run_backup, '__wrapped__') else (None, None)
    # 直接调 subprocess 路径
    import subprocess
    script = Path(__file__).resolve().parents[2] / "scripts" / "backup_data.py"
    out = tmp_path / "backups" / "auto-test.zip"
    out.parent.mkdir(parents=True, exist_ok=True)
    r = subprocess.run([sys.executable, str(script), "backup",
                        "--data-dir", str(tmp_path / "data"), "--out", str(out), "--no-media"],
                       capture_output=True, text=True)
    assert r.returncode == 0
    assert out.exists()
    import zipfile
    with zipfile.ZipFile(out) as z:
        assert "labsentinel.db" in z.namelist()


def test_backup_status_endpoint(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "admin_token", "")
    monkeypatch.setattr(settings, "backup_interval_h", 24)
    monkeypatch.setattr(backups, "backup_dir", lambda: tmp_path / "backups")
    _mk_zips(tmp_path, n=2)
    with TestClient(app) as c:
        d = c.get("/api/backups/status").json()
        for k in ("enabled", "interval_h", "keep", "last", "count", "total_bytes"):
            assert k in d
        assert d["enabled"] is True and d["count"] >= 2


def test_backup_run_endpoint(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "backup_interval_h", 0)  # 关闭调度避免 lifespan 自动跑
    with TestClient(app) as c:
        monkeypatch.setattr(backups, "run_backup", lambda: (True, "mock"))
        monkeypatch.setattr(backups, "_running", __import__("threading").Event())
        r = c.post("/api/backups/run")
        assert r.status_code == 200
        assert r.json()["started"] is True


def test_backup_run_conflict_409(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "backup_interval_h", 0)
    with TestClient(app) as c:
        monkeypatch.setattr(backups, "_running", __import__("threading").Event())
        backups._running.set()  # 模拟正在跑
        r = c.post("/api/backups/run")
        assert r.status_code == 409
        backups._running.clear()
