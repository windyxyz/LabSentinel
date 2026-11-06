"""T-601/T-602：CJK 字体（X-07）——PDF 中文不再依赖 C:\\Windows\\Fonts\\msyh.ttc。

查找顺序：包内内置（app/assets/fonts/NotoSansCJKsc-*.otf）→ 平台常见路径 → Helvetica 回退。
"""
from pathlib import Path

from . import is_macos, is_windows

BUILTIN = [
    "NotoSansCJKsc-Regular.otf",
    "NotoSansSC-Regular.otf",
    "SourceHanSansSC-Regular.otf",
]


def cjk_font_path(configured: str | None = None) -> str | None:
    """返回可用中文字体路径；None = 无（调用方回退 Helvetica 并告警）。"""
    if configured and Path(configured).exists():
        return configured
    root = Path(__file__).resolve().parents[1]  # app/
    for name in BUILTIN:
        p = root / "assets" / "fonts" / name
        if p.exists():
            return str(p)
    if is_windows():
        cands = [
            Path(r"C:\Windows\Fonts\msyh.ttc"),
            Path(r"C:\Windows\Fonts\msyh.ttf"),
            Path(r"C:\Windows\Fonts\simhei.ttf"),
            Path(r"C:\Windows\Fonts\simsun.ttc"),
        ]
    elif is_macos():
        cands = [
            Path("/System/Library/Fonts/PingFang.ttc"),
            Path("/System/Library/Fonts/STHeiti Light.ttc"),
            Path("/Library/Fonts/Arial Unicode.ttf"),
        ]
    else:
        cands = [
            Path("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
            Path("/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc"),
            Path("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
        ]
    for c in cands:
        if c.exists():
            return str(c)
    return None
