"""R2-303：/api/system 透出 onnxruntime providers（前端 GPU 加速探测依据）。

B-28 同口径：探测绝不触发模型加载/建 session——onnxruntime 缺失时 providers=[]
且 /api/system 仍 200。
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app.config import settings  # noqa: E402
from app.main import app  # noqa: E402


@pytest.fixture()
def client(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "auth_read_protected", False)
    monkeypatch.setattr(settings, "admin_token", "")
    with TestClient(app) as c:
        yield c


def test_system_exposes_providers(client):
    r = client.get("/api/system")
    assert r.status_code == 200
    body = r.json()
    assert "providers" in body
    providers = body["providers"]
    assert isinstance(providers, list)
    # onnxruntime 已安装的任何环境都至少有 CPUExecutionProvider
    assert "CPUExecutionProvider" in providers


def test_system_providers_reflect_build(client, monkeypatch):
    """providers 来自 onnxruntime 构建（而非配置）：CPU 包无 DML，DML 包才含 DmlExecutionProvider。"""
    import app.api as api_mod

    monkeypatch.setattr(api_mod, "_providers_peek",
                        lambda: ["DmlExecutionProvider", "CPUExecutionProvider"])
    body = client.get("/api/system").json()
    assert body["providers"] == ["DmlExecutionProvider", "CPUExecutionProvider"]

    monkeypatch.setattr(api_mod, "_providers_peek",
                        lambda: ["AzureExecutionProvider", "CPUExecutionProvider"])
    body = client.get("/api/system").json()
    assert "DmlExecutionProvider" not in body["providers"]


def test_providers_peek_never_loads_model(monkeypatch):
    """onnxruntime 导入失败（未安装/损坏）→ []，不抛异常（/api/system 不因此 500）。"""
    import app.api as api_mod

    import builtins
    real_import = builtins.__import__

    def fake_import(name, *a, **kw):
        if name == "onnxruntime":
            raise ImportError("No module named 'onnxruntime'")
        return real_import(name, *a, **kw)

    monkeypatch.setattr(builtins, "__import__", fake_import)
    assert api_mod._providers_peek() == []
