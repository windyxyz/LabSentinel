"""T-601/T-602：摄像头枚举跨平台（X-03）。

Windows：usb_devices.py 现有 DirectShow（委托）；Linux：/dev/video* + v4l2-ctl 名称；
macOS：ffmpeg -f avfoundation -list_devices 解析。返回结构不变 [{index, name}]。
"""
from . import is_linux, is_macos, is_windows
from .process import run_text  # R2-006: locale-correct subprocess text decoding


def list_cameras(refresh: bool = False) -> list[dict]:
    if is_windows():
        from .. import usb_devices
        return usb_devices.list_devices(refresh=refresh)
    if is_linux():
        return _list_v4l2()
    if is_macos():
        return _list_avfoundation()
    return []


def _list_v4l2() -> list[dict]:
    from pathlib import Path
    out = []
    names: dict[str, str] = {}
    try:
        import subprocess
        r = run_text(["v4l2-ctl", "--list-devices"], capture_output=True, timeout=5)
        cur = None
        for line in (r.stdout or "").splitlines():
            if line and not line.startswith(("\t", " ")) and line.rstrip().endswith(":"):
                cur = line.rstrip().rstrip(":")
            elif line.strip().startswith("/dev/video") and cur:
                names[line.strip()] = cur
    except (OSError, subprocess.TimeoutExpired):
        pass
    for p in sorted(Path("/dev").glob("video*"), key=lambda x: x.name):
        if not p.name[len("video"):].isdigit():
            continue
        out.append({"index": int(p.name[len("video"):]),
                    "name": names.get(str(p), f"Video device {p.name}")})
        if len(out) >= 16:
            break
    return out


def _list_avfoundation() -> list[dict]:
    """macOS：解析 ffmpeg avfoundation 设备列表（视频设备段）。"""
    try:
        import subprocess
        from .binaries import find_binary
        ff = find_binary("ffmpeg") or "ffmpeg"
        r = run_text([ff, "-f", "avfoundation", "-list_devices", "true", "-i", ""],
                     capture_output=True, timeout=10)
        out, in_video = [], False
        for line in (r.stderr or "").splitlines():
            if "AVFoundation video devices" in line:
                in_video = True
                continue
            if "AVFoundation audio devices" in line:
                in_video = False
            if in_video:
                import re
                m = re.search(r"\[(\d+)\]\s+(.+)$", line)
                if m:
                    out.append({"index": int(m.group(1)), "name": m.group(2).strip()})
        return out
    except (OSError, subprocess.TimeoutExpired):
        return []
