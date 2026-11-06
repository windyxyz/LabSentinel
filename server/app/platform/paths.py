"""T-601/T-602：路径语义（X-09）——数据/日志/配置目录。

- 服务/便携模式由 LS_DATA_DIR 等环境变量显式指定（现状）；
- 未指定时按平台默认：Windows %LOCALAPPDATA%\\LabSentinel；macOS
  ~/Library/Application Support/LabSentinel；Linux $XDG_DATA_HOME 或 ~/.local/share/LabSentinel；
- 服务模式（LS_SERVICE=1）：Linux /var/lib/labsentinel、macOS /Library/Application Support/LabSentinel。
"""
import os
from pathlib import Path

from . import is_macos, is_windows


def data_dir() -> Path:
    env = os.environ.get("LS_DATA_DIR", "").strip()
    if env:
        return Path(env)
    if os.environ.get("LS_SERVICE") == "1" and not is_windows():
        return (Path("/Library/Application Support/LabSentinel") if is_macos()
                else Path("/var/lib/labsentinel"))
    if is_windows():
        return Path(os.environ.get("LOCALAPPDATA", str(Path.home()))) / "LabSentinel" / "data"
    if is_macos():
        return Path.home() / "Library" / "Application Support" / "LabSentinel" / "data"
    xdg = os.environ.get("XDG_DATA_HOME", str(Path.home() / ".local" / "share"))
    return Path(xdg) / "labsentinel" / "data"


def log_dir() -> Path:
    env = os.environ.get("LS_LOG_DIR", "").strip()
    if env:
        return Path(env)
    return data_dir().parent / "logs"


def config_path() -> Path:
    env = os.environ.get("LS_CONFIG_FILE", "").strip()
    if env:
        return Path(env)
    return data_dir().parent / "config.env"
