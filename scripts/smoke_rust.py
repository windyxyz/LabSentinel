"""Rust 单二进制便携包冒烟（RA3，ADR-0011）：隔离解包 → 启动 → 就绪 →
API/前端断言 → 停止 → 清理。对齐 smoke_portable.py 的判定口径，但目标形态
是 labsentinel-server.exe（无嵌入式 Python，无安装器）。

用法: python scripts/smoke_rust.py --zip <labsentinel-rust-*.zip> [--timeout 90]
退出码 0 = 全部 PASS。失败自动保留现场（--keep 可强制保留）。
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

CREATE_NO_WINDOW = 0x0800_0000 if sys.platform == "win32" else 0
DEVNULL = subprocess.DEVNULL


def http_get(url: str, timeout: float = 2.0) -> tuple[int, bytes]:
    import urllib.request
    req = urllib.request.Request(url)
    # 回环绝不走系统代理（与 smoke_portable 同口径）
    op = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with op.open(req, timeout=timeout) as r:
        return r.status, r.read()


def main() -> int:
    ap = argparse.ArgumentParser(description="Rust 单二进制便携包冒烟（RA3）")
    ap.add_argument("--zip", type=Path, required=True)
    ap.add_argument("--timeout", type=int, default=90)
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    if not a.zip.is_file():
        print(f"[FAIL] zip 不存在: {a.zip}")
        return 1
    tmp = Path(tempfile.mkdtemp(prefix="ls-rust-smoke-"))
    ok_all = True
    proc = None
    try:
        # [1] 隔离解包
        pkg = tmp / "pkg"
        with __import__("zipfile").ZipFile(a.zip) as z:
            z.extractall(pkg)
        exe = pkg / "labsentinel-server.exe"
        if not exe.is_file():
            exe = pkg / "labsentinel-server"
        ok1 = exe.is_file()
        print(f"[{'PASS' if ok1 else 'FAIL'}] 1 解包 —— exe={'有' if ok1 else '缺失'}")
        ok_all &= ok1
        if not ok1:
            return 1

        # [2] 隔离环境启动（数据/DB 全在 sandbox，不污染真实 LOCALAPPDATA）
        sandbox = tmp / "sandbox"
        (sandbox / "data").mkdir(parents=True)
        port = 18180
        env = dict(os.environ)
        env.update({
            "LS_DB_PATH": str(sandbox / "data" / "labsentinel.db"),
            "LS_DATA_DIR": str(sandbox / "data"),
            "LS_PORT": str(port),
            "LS_WEB_DIST": str(pkg / "web"),
            "LS_ADMIN_TOKEN": "",
            "LS_LAUNCHER_NO_BROWSER": "1",
        })
        proc = subprocess.Popen(
            [str(exe)], cwd=str(pkg), env=env,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            creationflags=CREATE_NO_WINDOW)
        base = f"http://127.0.0.1:{port}"

        # [3] 就绪轮询（/api/health/ready）
        ok3, last = False, ""
        deadline = time.time() + a.timeout
        while time.time() < deadline:
            if proc.poll() is not None:
                last = f"进程提前退出 rc={proc.returncode}"
                break
            try:
                st, body = http_get(f"{base}/api/health/ready", timeout=1.0)
                if st == 200 and json.loads(body).get("status") == "ready":
                    ok3 = True
                    break
            except (OSError, ValueError) as e:
                last = f"{type(e).__name__}"
            time.sleep(1)
        print(f"[{'PASS' if ok3 else 'FAIL'}] 3 就绪 —— {last or 'ready'}")
        ok_all &= ok3

        # [4] /api/system：backend=rust + version 非空
        ok4, detail = False, ""
        try:
            st, body = http_get(f"{base}/api/system", timeout=3)
            j = json.loads(body)
            ok4 = st == 200 and j.get("backend") == "rust" and bool(j.get("version"))
            detail = f"backend={j.get('backend')} version={j.get('version')}"
        except (OSError, ValueError) as e:
            detail = repr(e)
        print(f"[{'PASS' if ok4 else 'FAIL'}] 4 /api/system —— {detail}")
        ok_all &= ok4

        # [5] 前端 SPA（index + immutable 资产头抽验）
        ok5, detail5 = False, ""
        try:
            st, html = http_get(f"{base}/", timeout=3)
            marker = b'id="app"' in html
            asset_ok = True
            import re as _re
            assets = _re.findall(r'(?:src|href)="(/assets/[^"]+)"', html.decode("utf-8", "replace"))
            if assets:
                st2, _ = http_get(f"{base}{assets[0]}", timeout=3)
                asset_ok = st2 == 200
            ok5 = st == 200 and marker and asset_ok
            detail5 = f"根标记={'命中' if marker else '缺失'} 资产 {assets[0] if assets else '无'}"
        except (OSError, ValueError) as e:
            detail5 = repr(e)
        print(f"[{'PASS' if ok5 else 'FAIL'}] 5 前端 —— {detail5}")
        ok_all &= ok5
    finally:
        if proc is not None and proc.poll() is None:
            subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"],
                           capture_output=True, creationflags=CREATE_NO_WINDOW)
        if ok_all and not a.keep:
            shutil.rmtree(tmp, ignore_errors=True)
            print("[清理] 已删除 " + str(tmp))
        else:
            print(f"[保留] 现场: {tmp}" + ("（--keep）" if a.keep else "（失败排障）"))
    print(f"[结果] {'PASS' if ok_all else 'FAIL'}")
    return 0 if ok_all else 1


if __name__ == "__main__":
    sys.exit(main())
