"""T-601/T-602：进程语义跨平台（X-06/X-10）。

spawn_hidden：Windows CREATE_NO_WINDOW；POSIX start_new_session（脱离控制终端）。
is_our_process：Windows 用 PowerShell/CIM 校验命令行；POSIX 读 /proc 或 ps（无 psutil 依赖）。
"""
import os
import subprocess
from pathlib import Path

from . import is_windows


def run_text(argv: list[str], **kwargs) -> subprocess.CompletedProcess:
    """R2-006/N-37/N-39: decode subprocess text with the correct code page.

    Windows PowerShell / system tools emit ANSI (GBK on zh-CN Windows) while
    the portable launcher sets PYTHONUTF8=1 -- plain `text=True` would decode
    with UTF-8 and raise UnicodeDecodeError in the reader thread, making
    is_our_process() return False / hiding backup failures. On POSIX use UTF-8.
    `errors="replace"` keeps the process list usable even with mixed encodings.
    """
    if is_windows():
        kwargs.setdefault("encoding", "mbcs")
    else:
        kwargs.setdefault("encoding", "utf-8")
    kwargs.setdefault("errors", "replace")
    kwargs.setdefault("text", True)
    return subprocess.run(argv, **kwargs)


def spawn_hidden(argv: list[str], **kwargs) -> subprocess.Popen:
    """跨平台隐藏窗口启动：Windows 无控制台窗口；POSIX 独立会话。"""
    if is_windows():
        kwargs.setdefault("creationflags", subprocess.CREATE_NO_WINDOW)  # type: ignore[attr-defined]
    else:
        kwargs.setdefault("start_new_session", True)
    return subprocess.Popen(argv, **kwargs)


def is_our_process(pid: int, markers: tuple[str, ...] = ("portable_launcher.py", "uvicorn")) -> bool:
    """pid 是不是我们的进程（防 PID 复用误杀）。markers 任一出现在命令行即认定。"""
    try:
        if is_windows():
            ps = run_text(
                ["powershell", "-NoProfile", "-Command",
                 f"(Get-CimInstance Win32_Process -Filter 'ProcessId={int(pid)}').CommandLine"],
                capture_output=True, timeout=8,
                creationflags=subprocess.CREATE_NO_WINDOW)  # type: ignore[attr-defined]
            cmd = (ps.stdout or "").strip()
            return bool(cmd) and any(m in cmd for m in markers)
        # POSIX：/proc/<pid>/cmdline（Linux）或 ps（macOS）
        proc = Path(f"/proc/{int(pid)}/cmdline")
        if proc.exists():
            cmd = proc.read_bytes().replace(b"\x00", b" ").decode("utf-8", "replace")
            return any(m in cmd for m in markers)
        ps = run_text(["ps", "-p", str(int(pid)), "-o", "command="],
                      capture_output=True, timeout=5)
        return any(m in (ps.stdout or "") for m in markers)
    except (OSError, subprocess.TimeoutExpired):
        return False


def kill_tree(pid: int) -> None:
    """终止进程树：Windows taskkill /T /F；POSIX kill 进程组/单进程。"""
    try:
        if is_windows():
            subprocess.run(["taskkill", "/PID", str(int(pid)), "/T", "/F"],
                           capture_output=True,
                           creationflags=subprocess.CREATE_NO_WINDOW)  # type: ignore[attr-defined]
        else:
            os.kill(int(pid), 15)
    except OSError:
        pass


