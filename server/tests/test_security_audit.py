"""单元测试：任务134 安全审计修复回归（人脸照片上限 / 配置导入规模上限）。
运行: pytest tests/test_security_audit.py -q"""
import types

from fastapi.testclient import TestClient

from app.main import app


def test_face_photo_10mb_cap(tmp_path, monkeypatch):
    from app.config import settings
    monkeypatch.setattr(settings, "face_enabled", True)  # 绕过 _face_guard 的 403
    monkeypatch.setattr("app.api.face_client",
                        types.SimpleNamespace(configured=True,
                                              add_person=lambda *a: None,
                                              list_persons=lambda: []))
    with TestClient(app) as tc:
        r = tc.post("/api/face/persons",
                    data={"subject": "x", "consent": "true"},
                    files={"photo": ("big.jpg", b"x" * (10 * 1024 * 1024 + 1))})
        assert r.status_code == 413
        r2 = tc.post("/api/face/persons",
                     data={"subject": "x", "consent": "true"},
                     files={"photo": ("ok.jpg", b"x" * 1024)})
        assert r2.status_code in (200, 502)  # 正常大小放行（502=无 CompreFace，非大小问题）


def test_config_import_scale_cap():
    import pytest as _pytest
    from fastapi import HTTPException
    from app.api import _validate_config_import
    big = {"format": "labsentinel-config", "version": 1,
           "cameras": [{"name": f"c{i}", "source": "D:/x.mp4", "type": "file"}
                       for i in range(201)]}
    with _pytest.raises(HTTPException) as ei:
        _validate_config_import(big)
    assert "超限" in ei.value.detail
    # 小规模正常通过
    ok = _validate_config_import({"format": "labsentinel-config", "version": 1,
                                  "cameras": [{"name": "c", "source": "D:/x.mp4",
                                               "type": "file"}]})
    assert len(ok[0]) == 1

