"""T-401（B-15）：版本单一来源——仓库根 VERSION（SemVer，Q-05）。

main.py / api.py / build_portable / install_portable / web 全部从这里取；
web/package.json 由 scripts/bump_version.py 同步。app/VERSION 由构建脚本落包。
"""
from pathlib import Path

_FALLBACK = "0.2.0"


def read_version() -> str:
    """依次找 app/VERSION（打包布局）→ 仓库根 VERSION（源码布局）；缺失用 fallback。"""
    for cand in (Path(__file__).resolve().parent / "VERSION",
                 Path(__file__).resolve().parents[2] / "VERSION"):
        try:
            v = cand.read_text("ascii").strip()
            if v:
                return v
        except OSError:
            continue
    return _FALLBACK
