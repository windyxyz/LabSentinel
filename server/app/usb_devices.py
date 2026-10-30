"""USB（DirectShow）摄像头枚举（任务60）。

限制说明：index 是 DirectShow 的枚举顺序，通常与 OpenCV CAP_DSHOW 一致，但驱动/系统更新
或插拔顺序变化后可能改变——**设备名更稳定，索引仅本机当前参考**；前端已按设备名展示。
ffmpeg -list_devices 以 dummy 为输入必然非 0 退出，属正常，只解析 stderr。
10 秒内存缓存（本机设备集合不变，避免每次开页面起进程）；refresh=True 强制重枚举。

ffmpeg 路径（任务75 修复）：源码布局在 `<根>/tools/ffmpeg/ffmpeg.exe`，便携包在
`<根>/tools/ffmpeg.exe`（扁平）——用 resolve_ffmpeg() 按候选顺序解析，不要写死相对下标。
"""
import os
import re
import subprocess
import time
from pathlib import Path

# 兼容保留：旧测试/外部脚本 import FFMPEG。list_devices 每次经 resolve_ffmpeg 重新解析
#（LS_FFMPEG 环境变量优先），避免缓存到错误布局的路径。
FFMPEG = Path(__file__).resolve().parents[2] / "tools" / "ffmpeg" / "ffmpeg.exe"
TTL = 10.0
_LINE = re.compile(r'\[dshow @ [^\]]+\]\s+"(.+?)"\s+\((video|audio)\)')

_cache: dict = {"ts": 0.0, "data": None}


def detect_root() -> Path:
    """从本文件向上推导项目根：便携包（runtime/python.exe）或源码根（server/ 目录）。"""
    for p in Path(__file__).resolve().parents:
        if (p / "runtime" / "python.exe").exists() or (p / "server").exists():
            return p
    return Path(__file__).resolve().parents[2]


def resolve_ffmpeg(root: Path | None = None) -> tuple[Path, list[Path]]:
    """候选顺序：LS_FFMPEG（存在才用）→ 根/tools/ffmpeg/ffmpeg.exe → 根/tools/ffmpeg.exe
    → 根/ffmpeg.exe。返回 (命中的路径或第一个候选[用于报错], 尝试过的候选列表)。"""
    root = root or detect_root()
    candidates: list[Path] = []
    env = os.environ.get("LS_FFMPEG")
    if env:
        candidates.append(Path(env))
    candidates += [root / "tools" / "ffmpeg" / "ffmpeg.exe",
                   root / "tools" / "ffmpeg.exe",
                   root / "ffmpeg.exe"]
    for c in candidates:
        if c.exists():
            return c, candidates
    return candidates[0], candidates


def parse_dshow(stderr: str) -> list[dict]:
    """从 ffmpeg stderr 解析 DirectShow 设备行，按出现顺序赋 index 0..n-1，忽略 (audio)。"""
    devices = []
    for m in _LINE.finditer(stderr or ""):
        if m.group(2) != "video":
            continue
        devices.append({"index": len(devices), "name": m.group(1), "source": str(len(devices))})
    return devices


def list_devices(refresh: bool = False, root: Path | None = None) -> dict:
    """返回 {"devices": [...], "error": str|None, "ffmpeg": path}；失败不抛异常（200 语义降级）。"""
    now = time.monotonic()
    if not refresh and _cache["data"] and now - _cache["ts"] < TTL:
        return _cache["data"]
    fallback = "可手动输入设备索引"
    ffmpeg, tried = resolve_ffmpeg(root)
    if not ffmpeg.exists():
        tried_txt = "、".join(str(t) for t in tried[:3])
        data = {"devices": [], "error": f"未找到 ffmpeg（尝试过：{tried_txt}），"
                                        f"无法枚举 USB 设备，{fallback}（也可用 LS_FFMPEG 指定路径）",
                "ffmpeg": str(ffmpeg)}
    else:
        try:
            r = subprocess.run(
                [str(ffmpeg), "-hide_banner", "-f", "dshow", "-list_devices", "true", "-i", "dummy"],
                capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=10)
            devices = parse_dshow(r.stderr)
            if devices:
                data = {"devices": devices, "error": None, "ffmpeg": str(ffmpeg)}
            else:
                data = {"devices": [],
                        "error": f"未发现 DirectShow 视频设备（检查插拔/隐私挡板/是否被会议软件占用），{fallback}",
                        "ffmpeg": str(ffmpeg)}
        except subprocess.TimeoutExpired:
            data = {"devices": [], "error": f"枚举超时（10s），可点「刷新设备」重试，{fallback}",
                    "ffmpeg": str(ffmpeg)}
        except OSError as e:
            data = {"devices": [], "error": f"无法启动 ffmpeg（{e}），{fallback}", "ffmpeg": str(ffmpeg)}
    _cache.update(ts=now, data=data)
    return data
