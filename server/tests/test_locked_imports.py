"""R2-001/N-01 回归：按锁文件安装后，requirements.in 的每个顶层包都必须可导入。

背景：grpcio 与 cryptography 曾在 .venv 里手工安装过，却从未回写到
requirements.in / 三份锁，导致按锁全新安装后 analyzer_sidecar 的 import grpc
失败、POSIX 密钥静默降级明文、更新清单验签不可用。这个测试在 CI 的「按锁
安装」环境里运行，把「锁文件漏顶层依赖」变成红灯。

注意：本测试只断言「顶层声明的包可导入」，不做版本/哈希校验（锁文件本身的
--require-hashes 安装已保证）。conda/系统 site-packages 混入时同样能过。
"""
from __future__ import annotations

import importlib
import re
import sys
from pathlib import Path

SERVER = Path(__file__).resolve().parents[1]
REQ_IN = SERVER / "requirements.in"

# 发行名 -> 实际 import 名（不一致的几个）
IMPORT_NAME = {
    "opencv-python-headless": "cv2",
    "python-multipart": "multipart",
    "pydantic-settings": "pydantic_settings",
    "paho-mqtt": "paho.mqtt.client",
    "uvicorn[standard]": "uvicorn",
    "grpcio": "grpc",
    "defusedxml": "defusedxml",
    "onnxruntime-directml": "onnxruntime",
    "onnxruntime-gpu": "onnxruntime",
}


def _top_level(path: Path):
    """Yield (distribution_name, raw_line) for each top-level requirement."""
    for raw in path.read_text("ascii").splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line or line.startswith("-r"):
            continue
        if "sys_platform == \"never\"" in line:
            # B-03 override marker: intentionally never installed
            continue
        if sys.platform == "win32":
            if 'sys_platform != "win32"' in line:
                continue
        elif 'sys_platform == "win32"' in line:
            continue
        name = re.split(r"[<>=!~\[; ]", line)[0].strip()
        if name:
            yield name, line


def test_top_level_requirements_importable():
    for name, line in _top_level(REQ_IN):
        mod = IMPORT_NAME.get(name, name.replace("-", "_"))
        importlib.import_module(mod)


def test_grpc_generated_version_satisfied():
    """analyzer_pb2_grpc.py raises at import if grpc < GRPC_GENERATED_VERSION."""
    from app import analyzer_pb2_grpc  # noqa: F401  (import itself is the check)
