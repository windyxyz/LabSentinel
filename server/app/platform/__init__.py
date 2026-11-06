"""T-601（S6/X-08）：平台适配层——唯一允许出现 os.name / sys.platform 判断的地方。

各子模块提供 Windows / macOS / Linux 实现：
- power.py     PowerInhibitor（Win SetThreadExecutionState / mac caffeinate / linux systemd-inhibit）
- devices.py   list_cameras()（DirectShow / AVFoundation / V4L2）
- binaries.py  find_binary("ffmpeg"|"ffprobe"|"go2rtc")
- paths.py     data_dir()/log_dir()（platformdirs 语义，服务模式系统目录）
- process.py   spawn_hidden()/is_our_process()（跨平台进程语义）
- fonts.py     cjk_font_path()（内置 Noto 优先）

本任务只做"搬家 + Windows 行为等价"；POSIX 实现在 T-602。
"""
import os
import sys

_WIN = os.name == "nt"
_MAC = sys.platform == "darwin"
_LINUX = sys.platform.startswith("linux")


def is_windows() -> bool:
    return _WIN


def is_macos() -> bool:
    return _MAC


def is_linux() -> bool:
    return _LINUX


def platform_name() -> str:
    return "windows" if _WIN else ("macos" if _MAC else ("linux" if _LINUX else sys.platform))


def arch() -> str:
    import platform
    m = platform.machine().lower()
    return "arm64" if m in ("arm64", "aarch64") else "x64"
