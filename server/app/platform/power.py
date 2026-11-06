"""T-601/T-602：防休眠跨平台（X-02）。

- Windows：keepawake.py 现有 SetThreadExecutionState（本模块委托，保持行为）
- macOS：caffeinate -i -w <pid> 子进程（自动随主进程退出）
- Linux：systemd-inhibit --what=sleep:idle --mode=block sleep infinity（子进程，停止时 kill）
- 容器内（无 systemd）：noop + 日志说明
"""
import subprocess

from . import is_linux, is_macos, is_windows

_proc: subprocess.Popen | None = None
_active = False


def start(display: bool = False, enabled: bool = True) -> bool:
    """开始防休眠。返回是否生效。"""
    global _proc, _active
    if not enabled:
        return False
    if is_windows():
        from .. import keepawake  # 复用现有 per-thread 实现
        keepawake.start(True, display)
        _active = True
        return True
    if _active:
        return True
    try:
        if is_macos():
            import os
            argv = ["caffeinate", "-i", "-w", str(os.getpid())]
            if display:
                argv.insert(1, "-d")
            _proc = subprocess.Popen(argv)
        elif is_linux():
            argv = ["systemd-inhibit", "--what=sleep:idle", "--who=LabSentinel",
                    "--why=monitoring", "--mode=block", "sleep", "infinity"]
            _proc = subprocess.Popen(argv)
        else:
            return False
        _active = True
        return True
    except OSError:
        return False


def stop() -> None:
    global _proc, _active
    if is_windows():
        from .. import keepawake
        keepawake.stop()
        _active = False
        return
    if _proc is not None:
        try:
            _proc.terminate()
        except OSError:
            pass
        _proc = None
    _active = False


def status() -> dict:
    if is_windows():
        from .. import keepawake
        return keepawake.status()
    return {"enabled": _active, "active": _active, "display": False}
