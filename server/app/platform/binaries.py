"""T-601/T-602：二进制查找——`tools/<os>-<arch>/<name>` 优先，其次 PATH（X-05）。"""
import shutil
from pathlib import Path

from . import arch, platform_name


def find_binary(name: str) -> str | None:
    """返回可执行文件路径；找不到返回 None。

    查找顺序：
    1. 仓库 tools/<platform>-<arch>/<name>[.exe]（随包/按平台分层）
    2. 仓库 tools/<name>[.exe]（现状布局：go2rtc.exe/ffmpeg 子目录）
    3. tools/ffmpeg/<name>[.exe]
    4. shutil.which(name)
    """
    root = Path(__file__).resolve().parents[3]
    exe = f"{name}.exe" if platform_name() == "windows" else name
    for cand in (
        root / "tools" / f"{platform_name()}-{arch()}" / exe,
        root / "tools" / exe,
        root / "tools" / "ffmpeg" / exe,
    ):
        if cand.exists():
            return str(cand)
    return shutil.which(name)
