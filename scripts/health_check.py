"""R2-402/N-18：升级后的真实健康检查（独立模块 + 子进程入口）。

旧默认健康检查是 `python -c "sys.exit(0)"`，永远不会失败 → T-404 的自动回滚
形同虚设。这里改为：用便携包的 runtime/python.exe 在**临时回环端口**真实启动
uvicorn，轮询 /api/health/ready 最多 60s，然后终止进程，退出码 0/1。

本文件同时是子进程脚本：install_portable.default_health_cmd() 返回
[python, <本文件路径>]，子进程走 __main__ 分支执行 _boot_main()。
命令文本不拼接任何来自升级包的内容；cwd 通过 LS_HEALTH_CWD 环境变量传入。

安全约束（Mimosa SSRF 白名单模式）：
- URL 只允许 http://127.0.0.1:<临时端口>/api/...，由 _loopback_url() 强校验；
- 端口由 OS 分配（bind 端口 0），绝不使用用户 8000 端口；
- 不读取、不执行安装目录内的任何文本来构造命令。
"""

from __future__ import annotations

import json
import os
import socket
import subprocess
import sys
import time
import urllib.request
from pathlib import Path
from urllib.parse import urlparse

_ALLOWED_HOSTS = frozenset({"127.0.0.1", "localhost", "::1"})
_ALLOWED_PATH_PREFIX = "/api/"


def _loopback_url(port: int, path: str) -> str:
    """Whitelist sanitizer: only loopback http + /api/ paths are allowed.

    Both host and path are checked with an anchored allowlist, so a malformed
    port or an unexpected path can never turn into an outbound request.
    """
    if not isinstance(port, int) or not (1024 < port < 65536):
        raise ValueError("invalid ephemeral port")
    if not path.startswith(_ALLOWED_PATH_PREFIX) or "?" in path or "#" in path:
        raise ValueError("path must be an /api/ path without query/fragment")
    url = f"http://127.0.0.1:{port}{path}"
    parsed = urlparse(url)
    if (parsed.scheme != "http" or parsed.hostname not in _ALLOWED_HOSTS
            or parsed.port != port or parsed.path != path
            or parsed.username or parsed.password):
        raise ValueError("url failed loopback whitelist")
    return url


def default_health_cmd(install: Path) -> tuple[list[str], dict] | None:
    """Return (argv, env) for the real boot check, or None for stub runtimes."""
    py = install / "runtime" / "python.exe"
    if not py.is_file() or py.stat().st_size <= 4096:
        return None  # 测试桩 / runtime 缺失 → 由调用方跳过
    return [str(py), str(Path(__file__).resolve())], {
        **os.environ,
        "LS_LOG_FILE": "0",
        "LS_HOST": "127.0.0.1",
        "LS_HEALTH_CWD": str(install / "app" / "server"),
    }


def _boot_main() -> int:
    """Child-process entry point: boot uvicorn on an ephemeral loopback port."""
    server_cwd = os.environ.get("LS_HEALTH_CWD", "")
    if not server_cwd or not Path(server_cwd).is_dir():
        print("[health] LS_HEALTH_CWD 未设置或目录不存在")
        return 1
    s = socket.socket()
    try:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    finally:
        s.close()
    env = dict(os.environ)
    env["LS_LOG_FILE"] = "0"
    env.setdefault("LS_HOST", "127.0.0.1")
    proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "app.main:app",
         "--host", "127.0.0.1", "--port", str(port)],
        cwd=server_cwd, env=env,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    ok = False
    try:
        url = _loopback_url(port, "/api/health/ready")
        for _ in range(120):
            if proc.poll() is not None:
                break
            try:
                with urllib.request.urlopen(url, timeout=1) as r:
                    if r.status == 200 and json.load(r).get("status") == "ready":
                        ok = True
                        break
            except (OSError, ValueError):
                pass
            time.sleep(0.5)
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
    print(f"[health] ready={ok}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(_boot_main())
