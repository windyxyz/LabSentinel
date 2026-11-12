"""回归（2026-11-13 CI linux）：LS_PPE_MODEL="" 空串被 pydantic 转成 Path(".")，
validate_settings 在 with_suffix 抛 "empty name" 致全部测试收集失败。
空路径必须视同未配置（WARN），不得抛异常。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from pathlib import Path

from app.config import settings  # noqa: E402
from app.validate import validate_settings  # noqa: E402


def test_empty_ppe_model_env_no_crash(monkeypatch):
    monkeypatch.setattr(settings, "ppe_model", Path(""))  # 模拟 LS_PPE_MODEL=""
    monkeypatch.setattr(settings, "model_fp32", Path(""))  # 兜底同样为空
    results = validate_settings()  # 修复前：ValueError: PosixPath(".") has an empty name
    assert isinstance(results, list)
    # 模型项应为 WARN 未配置（或至少不因空路径崩溃）
