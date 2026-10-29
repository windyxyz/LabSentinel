"""USB（DirectShow）摄像头枚举（任务60）。

限制说明：index 是 DirectShow 的枚举顺序，通常与 OpenCV CAP_DSHOW 一致，但驱动/系统更新
或插拔顺序变化后可能改变——**设备名更稳定，索引仅本机当前参考**；前端已按设备名展示。
ffmpeg -list_devices 以 dummy 为输入必然非 0 退出，属正常，只解析 stderr。
10 秒内存缓存（本机设备集合不变，避免每次开页面起进程）；refresh=True 强制重枚举。
"""
import re
import subprocess
import time
from pathlib import Path

FFMPEG = Path(__file__).resolve().parents[2] / "tools" / "ffmpeg" / "ffmpeg.exe"
TTL = 10.0
_LINE = re.compile(r'\[dshow @ [^\]]+\]\s+"(.+?)"\s+\((video|audio)\)')

_cache: dict = {"ts": 0.0, "data": None}


def parse_dshow(stderr: str) -> list[dict]:
    """从 ffmpeg stderr 解析 DirectShow 设备行，按出现顺序赋 index 0..n-1，忽略 (audio)。"""
    devices = []
    for m in _LINE.finditer(stderr or ""):
        if m.group(2) != "video":
            continue
        devices.append({"index": len(devices), "name": m.group(1), "source": str(len(devices))})
    return devices


def list_devices(refresh: bool = False) -> dict:
    """返回 {"devices": [...], "error": str|None, "ffmpeg": path}；失败不抛异常（200 语义降级）。"""
    now = time.monotonic()
    if not refresh and _cache["data"] and now - _cache["ts"] < TTL:
        return _cache["data"]
    fallback = "可手动输入设备索引"
    if not FFMPEG.exists():
        data = {"devices": [], "error": f"未找到 ffmpeg（{FFMPEG}），无法枚举 USB 设备，{fallback}",
                "ffmpeg": str(FFMPEG)}
    else:
        try:
            r = subprocess.run(
                [str(FFMPEG), "-hide_banner", "-f", "dshow", "-list_devices", "true", "-i", "dummy"],
                capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=10)
            devices = parse_dshow(r.stderr)
            if devices:
                data = {"devices": devices, "error": None, "ffmpeg": str(FFMPEG)}
            else:
                data = {"devices": [],
                        "error": f"未发现 DirectShow 视频设备（检查插拔/隐私挡板/是否被会议软件占用），{fallback}",
                        "ffmpeg": str(FFMPEG)}
        except subprocess.TimeoutExpired:
            data = {"devices": [], "error": f"枚举超时（10s），可点「刷新设备」重试，{fallback}",
                    "ffmpeg": str(FFMPEG)}
        except OSError as e:
            data = {"devices": [], "error": f"无法启动 ffmpeg（{e}），{fallback}", "ffmpeg": str(FFMPEG)}
    _cache.update(ts=now, data=data)
    return data
