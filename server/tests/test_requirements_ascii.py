"""T-001（B-01/B-02）+ R2-002 回归：所有 pip 输入文件必须纯 ASCII。

pip 在没有 BOM 时按系统代码页解码 requirements，GBK 机器上的 UTF-8 中文注释
会直接 UnicodeDecodeError。R2-002 之后 requirements.in / -cuda.in / overrides.txt
同样是事实源，一并纳入检查，防止中文注释再次混入。
"""
from pathlib import Path

SERVER = Path(__file__).resolve().parents[1]
FILES = [
    "requirements.in",
    "requirements-cuda.in",
    "requirements.txt",
    "overrides.txt",
    "requirements-win.lock",
    "requirements-linux.lock",
    "requirements-cuda.lock",
]


def test_requirements_files_are_ascii():
    for name in FILES:
        p = SERVER / name
        if p.exists():
            p.read_bytes().decode("ascii")


def test_reportlab_listed():
    # R2-002: requirements.txt 只是兼容 shim，reportlab 由 -r requirements.in 引入
    txt = (SERVER / "requirements.txt").read_text("ascii")
    assert "-r requirements.in" in txt
    assert "reportlab" in (SERVER / "requirements.in").read_text("ascii")


def test_in_has_grpc_and_cryptography():
    """R2-001/N-01: both must be top-level requirements, not just transitives."""
    txt = (SERVER / "requirements.in").read_text("ascii")
    assert "grpcio" in txt
    assert "cryptography" in txt
