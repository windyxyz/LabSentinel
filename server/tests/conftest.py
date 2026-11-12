# 测试会话全局隔离（105 补丁 P1-2）：
# 1) 关闭文件日志（避免 pytest 污染真实 server/data/logs）——单测自行覆盖的可照旧；
# 2) 所有 settings 路径默认指向本用例的 tmp_path——任何 `with TestClient(app)` 的完整 lifespan
#    （Store/migrations/start_all/retention 调度）都落临时目录，绝不碰真实 server/data，
#    也不再有"起 4 路演示摄像头线程"导致套件 10s→90s 的耗时回归。
# 已自行 monkeypatch 同名字段的用例（test_retention_preview/test_health_errors/test_camera_status
# 等）不受影响：其 monkeypatch 在本 fixture 之后生效，覆盖默认值。
import os
from pathlib import Path

os.environ.setdefault("LS_LOG_FILE", "0")
# R2-108: allow the TestClient "testserver" Host only under pytest (before app import).
os.environ.setdefault("LS_TESTING", "1")

import pytest  # noqa: E402

REPO = Path(__file__).resolve().parents[2]
# R2-506/N-29: no developer-machine absolute paths. Cargo's default target
# directory (engine-rs/target, gitignored) is the fallback; CI can override it.
RS_TARGET = Path(os.environ.get("CARGO_TARGET_DIR", str(REPO / "engine-rs" / "target")))


def rs_exe(name: str, env_var: str) -> Path:
    """Resolve a Rust binary for cross-language tests (exe missing -> caller skips)."""
    override = os.environ.get(env_var)
    if override:
        return Path(override)
    suffix = ".exe" if os.name == "nt" else ""
    return RS_TARGET / "debug" / f"{name}{suffix}"


@pytest.fixture(autouse=True)
def isolate_settings_paths(tmp_path, monkeypatch):
    from app.config import settings

    data = tmp_path / "data"
    data.mkdir(parents=True, exist_ok=True)
    for sub in ("snapshots", "videos", "annotation_pool"):
        (data / sub).mkdir(parents=True, exist_ok=True)
    monkeypatch.setattr(settings, "db_path", data / "test.db")
    # R2-102/R2-103: validate_settings() and _enforce_bind() may flip read
    # protection in place; give every test a deterministic auth baseline.
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "host", "127.0.0.1")
    monkeypatch.setattr(settings, "data_dir", data)
    monkeypatch.setattr(settings, "snapshot_dir", data / "snapshots")
    monkeypatch.setattr(settings, "video_dir", data / "videos")
    monkeypatch.setattr(settings, "annotation_pool_dir", data / "annotation_pool")
    # lifespan 会重写 tools/go2rtc.yaml（真实文件！测试期间指到 tmp）
    monkeypatch.setattr(settings, "go2rtc_yaml", tmp_path / "go2rtc.yaml")
    # 写限流桶是模块级全局（main._rate_windows，client_ip → deque）——CI linux 上
    # TestClient 全部来自 testclient 且跑得快，120/60s 桶被前面用例耗尽后，
    # 后续写请求全 429（实测 face/discovery 用例 assert 429 in (401,403) 失败）。
    # 每用例清空，恢复"每个用例独立预算"的语义。
    import app.main as _main
    if hasattr(_main, "_rate_windows"):
        _main._rate_windows.clear()
    yield
