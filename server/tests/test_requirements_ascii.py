"""T-001（B-01/B-02）回归：requirements.txt 必须纯 ASCII（GBK 机器 pip 解码），且列出 reportlab。"""
from pathlib import Path

REQ = Path(__file__).resolve().parents[1] / "requirements.txt"


def test_requirements_is_ascii():
    # pip 在无 BOM 时按系统代码页解码；GBK 机器上 UTF-8 中文注释直接 UnicodeDecodeError
    REQ.read_bytes().decode("ascii")


def test_reportlab_listed():
    txt = REQ.read_text("ascii")
    assert "reportlab" in txt
