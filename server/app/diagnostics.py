"""诊断聚合（任务61）：/api/diagnostics/summary 的只读聚合，全部为可公开信息。

不含事件详情/截图/密钥；错误仅来自进程内 RingHandler（最近 20 条 ERROR）。
"""
import shutil

from . import logging_setup
from .config import settings


def summary(store) -> dict:
    log_file = (settings.data_dir / "logs" / "server.log")
    rotated = sorted(p.name for p in log_file.parent.glob("server.log.*")) if log_file.parent.exists() else []
    deliveries = store.delivery_counts()  # {"pending":n,"failed":n,"ok":n}
    try:
        du = shutil.disk_usage(settings.data_dir)
        disk = {"data_dir": str(settings.data_dir), "free_bytes": du.free, "total_bytes": du.total}
    except OSError:
        disk = {"data_dir": str(settings.data_dir), "free_bytes": None, "total_bytes": None}
    from .api import _system_version  # 版本常量单一来源（api 模块此时已加载）
    from .engine import get_engine
    return {
        "version": _system_version(),
        "backend": get_engine().backend,
        "uptime_s": logging_setup.uptime_s(),
        "counts": {
            "cameras": len(store.cameras()),
            "zones": len(store.zones()),
            "rules": len(store.rules()),
            "events": store.count_events(),
            "alert_deliveries": sum(deliveries.values()),
        },
        "disk": disk,
        "logs": {
            "file": str(log_file),
            "size_bytes": log_file.stat().st_size if log_file.exists() else 0,
            "rotated": rotated,
        },
        "recent_errors": logging_setup.recent_errors(20),
    }
