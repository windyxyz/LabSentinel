"""R2-106/N-12 回归：加密组件不可用时拒绝保存，绝不静默降级明文。

审阅实测：POSIX 上 cryptography 未入锁 → _fernet() 返回 None → protect()
返回明文且只打一条 WARNING，告警密钥在 DB 里裸奔。
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402

from app import secretstore  # noqa: E402
from app.alert_config import AlertConfigError  # noqa: E402


def test_no_backend_rejects_plaintext(tmp_path, monkeypatch):
    """两个后端都不可用 → 必须抛错，DB 不留明文。"""
    monkeypatch.setattr(secretstore, "_dpapi_protect", lambda data: None)
    monkeypatch.setattr(secretstore, "_fernet", lambda data_dir: None)
    with pytest.raises(AlertConfigError, match="加密组件不可用"):
        secretstore.protect("super-secret-token", tmp_path)
    # 没有任何文件被写成明文
    assert not (tmp_path / "secret.key").exists()


def test_fernet_backend_roundtrip(tmp_path, monkeypatch):
    """DPAPI 不可用时走 Fernet（cryptography 现已入锁）；不外改 os.name，
    否则 pathlib 会在 Windows 上试图实例化 PosixPath。"""
    monkeypatch.setattr(secretstore, "_dpapi_protect", lambda data: None)
    monkeypatch.setattr(secretstore, "_dpapi_unprotect", lambda data: None)
    value = "wecom-webhook-key"
    enc = secretstore.protect(value, tmp_path)
    assert enc.startswith("enc:v1:")
    assert value not in enc
    assert secretstore.unprotect(enc, tmp_path) == value


def test_historical_plaintext_still_readable(tmp_path):
    """读取路径兼容历史明文（升级场景不锁死已有配置）。"""
    assert secretstore.unprotect("legacy-plain", tmp_path) == "legacy-plain"


def test_dpapi_scope_flag(monkeypatch):
    """LS_SECRET_SCOPE=machine 时加上 CRYPTPROTECT_LOCAL_MACHINE 位。"""
    from app.config import settings
    monkeypatch.setattr(settings, "secret_scope", "machine")
    assert secretstore._dpapi_flags() & 0x4
    monkeypatch.setattr(settings, "secret_scope", "user")
    assert not (secretstore._dpapi_flags() & 0x4)
