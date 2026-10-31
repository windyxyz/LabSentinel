# 测试会话全局隔离（105 补丁 P1-2）：
# 1) 关闭文件日志（避免 pytest 污染真实 server/data/logs）——单测自行覆盖的可照旧；
# 2) 所有 settings 路径默认指向本用例的 tmp_path——任何 `with TestClient(app)` 的完整 lifespan
#    （Store/migrations/start_all/retention 调度）都落临时目录，绝不碰真实 server/data，
#    也不再有"起 4 路演示摄像头线程"导致套件 10s→90s 的耗时回归。
# 已自行 monkeypatch 同名字段的用例（test_retention_preview/test_health_errors/test_camera_status
# 等）不受影响：其 monkeypatch 在本 fixture 之后生效，覆盖默认值。
import os

os.environ.setdefault("LS_LOG_FILE", "0")

import pytest  # noqa: E402


@pytest.fixture(autouse=True)
def isolate_settings_paths(tmp_path, monkeypatch):
    from app.config import settings

    data = tmp_path / "data"
    data.mkdir(parents=True, exist_ok=True)
    for sub in ("snapshots", "videos", "annotation_pool"):
        (data / sub).mkdir(parents=True, exist_ok=True)
    monkeypatch.setattr(settings, "db_path", data / "test.db")
    monkeypatch.setattr(settings, "data_dir", data)
    monkeypatch.setattr(settings, "snapshot_dir", data / "snapshots")
    monkeypatch.setattr(settings, "video_dir", data / "videos")
    monkeypatch.setattr(settings, "annotation_pool_dir", data / "annotation_pool")
    # lifespan 会重写 tools/go2rtc.yaml（真实文件！测试期间指到 tmp）
    monkeypatch.setattr(settings, "go2rtc_yaml", tmp_path / "go2rtc.yaml")
    yield
