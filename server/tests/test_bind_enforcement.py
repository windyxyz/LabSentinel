"""R2-102/N-03 回归：绑定地址单一来源 + 运行时兜底 + 安全 FATAL 无条件拒绝启动。

- settings.host 是唯一事实源（pydantic-settings 同时读环境变量与 .env）；
- 非回环且无令牌 / 逃生口 → validate 产出 FATAL（不再依赖 LS_STRICT_CONFIG）；
- 运行时 _enforce_bind 按 uvicorn 实际 scope["server"] 兜底；
- .env 里的 LS_HOST 参与判断（旧 os.environ.get 读不到）。
"""
import importlib
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402

from app.config import settings  # noqa: E402
from app.validate import (fatal_count, is_loopback_host,  # noqa: E402
                          validate_settings)


def test_loopback_definition():
    assert is_loopback_host("127.0.0.1")
    assert is_loopback_host("localhost")
    assert is_loopback_host("::1")
    assert is_loopback_host("")
    assert not is_loopback_host("0.0.0.0")
    assert not is_loopback_host("192.168.1.10")


def test_non_loopback_without_token_is_fatal(monkeypatch):
    monkeypatch.setattr(settings, "host", "0.0.0.0")
    monkeypatch.setattr(settings, "admin_token", "")
    monkeypatch.setattr(settings, "auth_disabled", False)
    monkeypatch.setattr(settings, "auth_read_protected", False)  # validate 会就地改它
    monkeypatch.setattr(settings, "strict_config", False)  # 关键：非 strict 也必须 FATAL
    results = validate_settings()
    fatals = [r for r in results if r["level"] == "FATAL"]
    assert any(r["key"] == "admin_token" for r in fatals)
    assert fatal_count(results) >= 1


def test_non_loopback_with_auth_disabled_is_fatal(monkeypatch):
    monkeypatch.setattr(settings, "host", "0.0.0.0")
    monkeypatch.setattr(settings, "admin_token", "adm")
    monkeypatch.setattr(settings, "auth_disabled", True)
    monkeypatch.setattr(settings, "auth_read_protected", False)  # validate 会就地改它
    results = validate_settings()
    assert any(r["key"] == "auth_disabled" and r["level"] == "FATAL" for r in results)


def test_loopback_without_token_is_warn_not_fatal(monkeypatch):
    monkeypatch.setattr(settings, "host", "127.0.0.1")
    monkeypatch.setattr(settings, "admin_token", "")
    monkeypatch.setattr(settings, "auth_disabled", False)
    results = validate_settings()
    assert fatal_count(results) == 0


def test_non_loopback_forces_read_protection(monkeypatch):
    monkeypatch.setattr(settings, "host", "192.168.1.10")
    monkeypatch.setattr(settings, "admin_token", "adm")
    monkeypatch.setattr(settings, "auth_disabled", False)
    # monkeypatch tracks the attribute, so validate_settings()'s in-place
    # mutation is reverted on teardown (no cross-test settings pollution).
    monkeypatch.setattr(settings, "auth_read_protected", False)
    validate_settings()
    assert settings.auth_read_protected is True


def test_enforce_bind_loopback_is_noop(monkeypatch):
    from app import main
    monkeypatch.setattr(main, "_bind_checked", False)
    monkeypatch.setattr(settings, "admin_token", "")
    monkeypatch.setattr(settings, "auth_disabled", False)
    # 不应退出、不改读保护
    main._enforce_bind({"server": ("127.0.0.1", 8000)})
    main._enforce_bind({"server": ("testserver", 80)})
    assert main._bind_checked is True


def test_enforce_bind_non_loopback_with_token_force_read(monkeypatch):
    from app import main
    monkeypatch.setattr(main, "_bind_checked", False)
    monkeypatch.setattr(settings, "admin_token", "adm")
    monkeypatch.setattr(settings, "auth_disabled", False)
    monkeypatch.setattr(settings, "auth_read_protected", False)
    # NOTE: this scope is non-loopback WITH a token -> no os._exit, only flips
    # read protection. monkeypatch restores the flag on teardown.
    main._enforce_bind({"server": ("0.0.0.0", 8000)})
    assert settings.auth_read_protected is True


def test_settings_host_reads_env_file(tmp_path, monkeypatch):
    """.env 里的 LS_HOST 旧实现读不到（os.environ.get），pydantic-settings 能读到。"""
    from pydantic_settings import BaseSettings, SettingsConfigDict

    class HostOnly(BaseSettings):
        host: str = "127.0.0.1"
        model_config = SettingsConfigDict(env_prefix="LS_", env_file=str(tmp_path / ".env"))

    (tmp_path / ".env").write_text("LS_HOST=0.0.0.0\n", encoding="utf-8")
    monkeypatch.chdir(tmp_path)
    assert HostOnly().host == "0.0.0.0"
