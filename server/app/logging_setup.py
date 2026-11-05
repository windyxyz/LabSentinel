"""服务端文件日志（轮转）+ 内存错误环形缓冲（任务61）。

- setup_file_logging()：幂等；追加 RotatingFileHandler 到 root（不动 uvicorn 的 console handler），
  5MB×3 份，`settings.data_dir/logs/server.log`；`LS_LOG_LEVEL` 覆盖级别（默认 INFO）；
  `LS_LOG_FILE=0/false` 关闭文件日志（返回 None，但 ring 仍工作）。
- RingHandler：进程内 deque 只保留最近 200 条 WARNING+（不落盘），diagnostics 取最近 20 条 ERROR。
- main.py 在导入阶段调用一次（尽早捕获启动错误）；tests 经 conftest 设 LS_LOG_FILE=0 避免污染。
"""
import logging
import os
import sys
import time
from collections import deque
from logging.handlers import RotatingFileHandler
from pathlib import Path

_START_MONO = time.monotonic()

_ring: deque[tuple[str, str]] = deque(maxlen=200)  # (formatted, levelname)
_file_handler: RotatingFileHandler | None = None
_ring_attached = False

_FMT = "%(asctime)s %(levelname)s %(name)s [req=%(request_id)s] %(message)s"


class _RequestIdFilter(logging.Filter):
    """给每条记录注入 request_id（来自 main.request_id_var；无请求上下文时为 "-"）。

    延迟导入 main 避免循环 import；contextvars 在无上下文时返回 default "-"。
    """
    def filter(self, record: logging.LogRecord) -> bool:
        try:
            from .main import request_id_var
            record.request_id = request_id_var.get("-")
        except Exception:  # noqa: BLE001
            record.request_id = "-"
        return True


_RID_FILTER = _RequestIdFilter()


class RingHandler(logging.Handler):
    """WARNING+ 进内存环；ERROR 才进入 recent_errors 输出。"""

    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno < logging.WARNING:
            return
        try:
            _ring.append((self.format(record), record.levelname))
        except Exception:  # noqa: BLE001  日志永不反噬业务
            pass


def recent_errors(limit: int = 20) -> list[str]:
    """最近 limit 条 ERROR 级日志（新→旧只取最近，按发生顺序返回）。"""
    return [msg for msg, lv in _ring if lv in ("ERROR", "CRITICAL")][-limit:]


def _ensure_ring() -> None:
    global _ring_attached
    if not _ring_attached:
        h = RingHandler()
        h.setFormatter(logging.Formatter(_FMT))
        h.addFilter(_RID_FILTER)
        logging.getLogger().addHandler(h)
        _ring_attached = True


def setup_file_logging(data_dir: Path | None = None) -> Path | None:
    """幂等挂文件 handler；返回日志文件路径或 None（关闭/失败）。"""
    global _file_handler
    _ensure_ring()
    if os.environ.get("LS_LOG_FILE", "").strip().lower() in ("0", "false"):
        return None
    from .config import settings
    target_dir = (data_dir or settings.data_dir) / "logs"
    target = target_dir / "server.log"
    if _file_handler is not None:
        if Path(_file_handler.baseFilename) == target:
            return target  # 已挂在同一文件：幂等
        logging.getLogger().removeHandler(_file_handler)
        _file_handler.close()
        _file_handler = None
    try:
        target_dir.mkdir(parents=True, exist_ok=True)
        level = getattr(logging, (os.environ.get("LS_LOG_LEVEL") or "INFO").upper(), logging.INFO)
        h = RotatingFileHandler(target, maxBytes=5 * 1024 * 1024, backupCount=3, encoding="utf-8")
        h.setFormatter(logging.Formatter(_FMT))
        h.addFilter(_RID_FILTER)
        h.setLevel(level)
        root = logging.getLogger()
        # 2026-10-30 审阅修复：原条件写反——root 默认 WARNING 且 level=INFO 时两个分支都不成立，
        # root 维持 WARNING 导致 app.* 的 INFO 日志在源头被丢弃、文件里只有 WARNING+
        if root.level == logging.NOTSET or root.level > level:
            root.setLevel(level)
        root.addHandler(h)
        _file_handler = h
        _attach_print_tee()  # T-504（B-22）：历史 print 输出同样进轮转文件（/api/logs 可查）
        return target
    except OSError:
        return None  # 目录不可写等：文件日志静默关闭，console 照常


class _PrintTee:
    """T-504：stdout 行级 tee——把各模块的历史 print（如 [engine]/[cam-1]/[retention]）
    以 INFO 级别双写进 logging（→ server.log 轮转文件），print 本身照常上控制台。
    替代方案是全仓 100+ 处 print 逐个改 logging；该重定向以零侵入达成同等可观测性。"""

    def __init__(self, stream):
        self._stream = stream
        self._buf = ""

    def write(self, s):
        self._stream.write(s)
        self._buf += s
        while "\n" in self._buf:
            line, self._buf = self._buf.split("\n", 1)
            if line.strip():
                logging.getLogger("app.stdout").info(line.rstrip())

    def flush(self):
        self._stream.flush()

    def isatty(self):
        return getattr(self._stream, "isatty", lambda: False)()

    def __getattr__(self, name):
        return getattr(self._stream, name)


_tee_attached = False


def _attach_print_tee() -> None:
    global _tee_attached
    if _tee_attached or os.environ.get("LS_LOG_TEE", "").strip().lower() in ("0", "false"):
        return
    # pytest 的捕获流不接（caplog 已覆盖）；仅真实 stdout/stderr 场景挂
    out = sys.__stdout__
    if out is None:
        return
    sys.stdout = _PrintTee(out)  # noqa: A001
    _tee_attached = True


def uptime_s() -> int:
    return int(time.monotonic() - _START_MONO)
