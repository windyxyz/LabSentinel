"""便携包一键冒烟（任务59）：zip → 隔离启动 → 健康/前端/播种断言 → 停止 → 清理。

用法（标准库）:
    python scripts/smoke_portable.py --zip dist/LabSentinel-portable-20261029.zip
    python scripts/smoke_portable.py --dir <已解压的包目录>      # 调试模式，跳过解压
可选: --port 8180（被占则失败，不漂移）  --timeout 90（必须大于启动器 LS_READY_TIMEOUT_S，默认 60）
      --keep（失败时保留现场；失败时本来就会保留并打印路径）

隔离：zip 解到 %TEMP%\\ls-smoke-<随机>，子进程 LOCALAPPDATA 指向其中 sandbox\\LocalAppData，
真实 %LOCALAPPDATA%\\LabSentinel 运行前后 mtime/文件数必须不变。失败 exit 1。
"""
import argparse
import json
import os
import re
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEVNULL = subprocess.DEVNULL
CREATE_NO_WINDOW = 0x08000000

RESULTS: list[tuple[str, bool, str]] = []


def step(name: str, ok: bool, detail: str = "") -> bool:
    RESULTS.append((name, ok, detail))
    print(f"[{'PASS' if ok else 'FAIL'}] {name}" + (f" —— {detail}" if detail else ""), flush=True)
    return ok


def http_get(url: str, timeout: float = 5.0) -> tuple[int, bytes]:
    req = urllib.request.Request(url, headers={"User-Agent": "ls-smoke/1.0"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.status, r.read()


def dir_stat(p: Path) -> tuple[int, float]:
    """(文件数, 目录 mtime)；目录不存在返回 (0, 0)。"""
    if not p.exists():
        return 0, 0.0
    n = sum(1 for _ in p.rglob("*") if _.is_file())
    return n, p.stat().st_mtime


def free_port(p: int) -> bool:
    s = socket.socket()
    try:
        return s.connect_ex(("127.0.0.1", p)) != 0
    finally:
        s.close()


def print_log_tails(cfg_dir: Path) -> None:
    """失败时打印 launcher/server 日志尾部（任务119：不保留现场也能看到判负原因）。"""
    logs = cfg_dir / "logs"
    if not logs.exists():
        print("[日志] 无 logs 目录")
        return
    for name in sorted(logs.glob("*.log"))[-4:]:
        try:
            tail = name.read_text(encoding="utf-8", errors="replace").splitlines()[-15:]
        except OSError as e:
            print(f"[日志] {name.name} 读取失败: {e}")
            continue
        print(f"[日志] {name.name} 尾部:")
        for ln in tail:
            print(f"    {ln}")


def wait_lock(lock: Path, timeout: int) -> tuple[int, int] | None:
    """轮询 launcher.lock（'pid port'）直到出现或超时。"""
    dl = time.time() + timeout
    while time.time() < dl:
        try:
            pid, port = lock.read_text(encoding="ascii").split()
            return int(pid), int(port)
        except (OSError, ValueError):
            time.sleep(1)
    return None


def _force_kill_pid(pid: int) -> None:
    """taskkill /T /F 兜底清理（只对确认属 LabSentinel 的 PID 调用，见 reap_leftover_pids）。"""
    subprocess.run(["taskkill", "/PID", str(pid), "/T", "/F"], capture_output=True,
                   creationflags=CREATE_NO_WINDOW)


def reap_leftover_pids(pids, wait_s: float = 30.0, alive=None, matcher=None,
                       killer=None) -> list[int]:
    """--stop 后残留收尾（任务110）：等待窗内未退出的 PID，确认仍是 LabSentinel 的
    python 进程（防 PID 复用误杀）才 taskkill /T /F。返回强杀后仍存活的 PID（交人工）。"""
    import portable_launcher as pl
    alive = alive or pl.pid_alive
    matcher = matcher or pl._pid_matches_launcher
    killer = killer or _force_kill_pid
    dl = time.time() + wait_s
    pending = [int(p) for p in pids if alive(int(p))]
    while pending and time.time() < dl:
        time.sleep(min(1.0, max(0.0, dl - time.time())))
        pending = [p for p in pending if alive(p)]
    failed = []
    for p in pending:
        if matcher(p) is not True:  # None（查询失败）同样拒绝——宁可漏杀不可误杀
            print(f"[残留] pid={p} 已非 LabSentinel 进程或身份不明，跳过强杀（请人工确认）")
            continue
        killer(p)
        if alive(p):
            failed.append(p)
    return failed


def main() -> int:
    ap = argparse.ArgumentParser(description="便携包一键冒烟（任务59）")
    ap.add_argument("--zip", type=Path, help="便携包 zip 路径")
    ap.add_argument("--dir", type=Path, help="已解压包目录（调试模式，跳过解压）")
    ap.add_argument("--port", type=int, default=8180)
    ap.add_argument("--timeout", type=int, default=90,
                    help="就绪等待秒数，必须大于启动器 LS_READY_TIMEOUT_S（默认 60）")
    ap.add_argument("--keep", action="store_true", help="成功也保留临时目录（调试用）")
    a = ap.parse_args()
    if a.timeout <= 60:  # 任务119：启动器就绪窗口默认 60s，冒烟必须留出余量，否则冷启动误判
        print(f"[WARN] --timeout={a.timeout}s ≤ 启动器默认就绪窗口 60s，冷启动可能误判；建议 ≥90s")
    if not a.zip and not a.dir:
        ap.error("--zip 或 --dir 必填其一")

    tmp = Path(tempfile.mkdtemp(prefix="ls-smoke-"))
    pkg = tmp / "pkg"
    keep_reason = ""
    try:
        # [1] 准备
        if a.dir:
            if not a.dir.exists():
                print(f"[失败] --dir 不存在: {a.dir}")
                return 1
            shutil.copytree(a.dir, pkg)
            src_label = str(a.dir)
        else:
            if not a.zip or not a.zip.exists():
                print(f"[失败] zip 不存在: {a.zip}")
                return 1
            with zipfile.ZipFile(a.zip) as z:
                names = z.namelist()
                top = names[0].split("/")[0]
                z.extractall(tmp)
            pkg = tmp / top
            src_label = a.zip.name
        if not free_port(a.port):
            print(f"[失败] 端口 {a.port} 已被占用（冒烟不自动漂移）")
            return 1
        sandbox = tmp / "sandbox" / "LocalAppData"
        cfg_dir = sandbox / "LabSentinel"
        cfg_dir.mkdir(parents=True)
        (cfg_dir / "config.env").write_text(f"port={a.port}\n", encoding="utf-8")
        real_la = Path(os.environ["LOCALAPPDATA"]) / "LabSentinel"
        real_before = dir_stat(real_la)
        step("1 准备隔离环境", True, f"pkg={pkg} sandbox={sandbox} port={a.port}（来源 {src_label}）")

        env = {**os.environ, "LOCALAPPDATA": str(sandbox), "LS_LAUNCHER_NO_BROWSER": "1",
               "PYTHONUTF8": "1"}
        exe = pkg / "LabSentinel.exe"
        if exe.exists():
            stop_cmd = [str(exe), "--stop"]
            proc = subprocess.Popen([str(exe)], env=env, stdin=DEVNULL, stdout=DEVNULL,
                                    stderr=DEVNULL, creationflags=CREATE_NO_WINDOW)
            launcher_label = "LabSentinel.exe"
        else:
            bat = pkg / "启动.bat"
            if not bat.exists():
                step("2 启动", False, "无 LabSentinel.exe 且无 启动.bat")
                return 1
            stop_cmd = [str(pkg / "runtime" / "python.exe"),
                        str(pkg / "scripts" / "portable_launcher.py"), "--stop"]
            proc = subprocess.Popen(["cmd", "/c", str(bat)], env=env, stdin=DEVNULL,
                                    stdout=DEVNULL, stderr=DEVNULL, cwd=str(pkg),
                                    creationflags=CREATE_NO_WINDOW)
            launcher_label = "启动.bat"
        step("2 启动", True, f"入口={launcher_label}（服务由 pythonw 托管，入口进程应自行退出）")

        # [3] 就绪
        lock = cfg_dir / "logs" / "launcher.lock"
        got = wait_lock(lock, a.timeout)
        if not step("3a 锁出现", got is not None, "" if got else f"{a.timeout}s 内未出现 {lock}"):
            keep_reason = "锁未出现"
            print_log_tails(cfg_dir)
            return 1
        lock_pid, lock_port = got  # type: ignore[misc]
        api = f"http://127.0.0.1:{lock_port}"
        ok = False
        detail = ""
        dl = time.time() + a.timeout  # 任务119：窗口与 --timeout 一致（> 启动器就绪窗口），冷启动 ~29s 不再擦边
        while time.time() < dl:
            try:
                st, body = http_get(f"{api}/api/system")
                if st == 200:
                    sysinfo = json.loads(body)
                    ok = bool(sysinfo.get("backend")) and bool(sysinfo.get("version"))
                    detail = f"backend={sysinfo.get('backend')} version={sysinfo.get('version')} port={lock_port}"
                    break
                time.sleep(1)  # 2026-10-30 审阅：非 200 也要等，否则连发空转后立即判负
            except OSError:
                time.sleep(1)
        if not step("3b /api/system 200 且 backend/version 非空", ok, detail):
            keep_reason = "服务未就绪"
            print_log_tails(cfg_dir)
            return 1

        # [4] 前端
        try:
            st, html = http_get(f"{api}/")
            marker_ok = st == 200 and b'id="app"' in html
            assets = re.findall(r'(?:src|href)="(/assets/[^"]+)"', html.decode("utf-8", "replace"))
            asset_st = None
            if assets:
                asset_st, _ = http_get(f"{api}{assets[0]}")
            step("4 前端首页+资源", marker_ok and asset_st == 200,
                 f"根标记={'命中' if marker_ok else '缺失'} 资源 {assets[0] if assets else '无'} → {asset_st}")
        except OSError as e:
            step("4 前端首页+资源", False, repr(e))

        # [5] 首启播种
        db = cfg_dir / "data" / "labsentinel.db"
        n_cams = -1
        if db.exists():
            con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
            try:
                n_cams = con.execute("select count(*) from cameras").fetchone()[0]
            finally:
                con.close()
        step("5 首启播种", n_cams >= 1, f"cameras={n_cams}（预期 4）")

        # [6] 停止
        rc = subprocess.run(stop_cmd, env=env, stdin=DEVNULL, stdout=DEVNULL,
                            stderr=DEVNULL, creationflags=CREATE_NO_WINDOW,
                            timeout=60).returncode
        # 任务110：入口进程 30s 等待窗内应自行退出；不退就句柄级 kill（Popen 句柄无 PID 复用风险）
        entry_gone = True
        if proc is not None and proc.poll() is None:
            try:
                proc.wait(timeout=30)
            except subprocess.TimeoutExpired:
                entry_gone = False
                try:
                    proc.kill()
                except OSError:
                    pass
        # 锁内服务 PID 若等待窗后仍活，确认属 LabSentinel 才强杀（防 PID 复用误杀）
        failed_pids = reap_leftover_pids([lock_pid])
        if failed_pids:
            print(f"[残留] PID {failed_pids} 强杀后仍存活，请人工处理")
        released = False
        for _ in range(15):
            if free_port(lock_port) and not lock.exists():
                released = True
                break
            time.sleep(1)
        step("6 停止", rc == 0 and released and not failed_pids and entry_gone,
             f"--stop rc={rc} 端口释放+锁删除={'是' if released else '否'} "
             f"入口退出={'是' if entry_gone else '否'} 残留PID={failed_pids or '无'}")

        # [7] 隔离断言
        real_after = dir_stat(real_la)
        step("7 隔离断言", real_before == real_after,
             f"真实 %LOCALAPPDATA%\\LabSentinel 前={real_before} 后={real_after}（不存在=0）")

        # [8] 清理
        if a.keep or keep_reason:
            print(f"[保留] 现场: {tmp}")
        else:
            shutil.rmtree(tmp, ignore_errors=True)
            print(f"[清理] 已删除 {tmp}")
    finally:
        # 兜底：任何路径离开都要确保没有残留服务进程（任务110：确认属 LabSentinel 才杀）
        lock = None
        cand = tmp / "sandbox" / "LocalAppData" / "LabSentinel" / "logs" / "launcher.lock"
        if cand.exists():
            try:
                pid = int(cand.read_text(encoding="ascii").split()[0])
                if reap_leftover_pids([pid], wait_s=0.0):
                    print(f"[残留] PID {pid} 兜底清理后仍存活，请人工处理（锁: {cand}）")
            except (OSError, ValueError, IndexError):
                pass

    n_fail = sum(1 for _, ok, _ in RESULTS if not ok)
    print(f"== 冒烟汇总: {len(RESULTS) - n_fail}/{len(RESULTS)} PASS ==")
    return 1 if n_fail else 0


if __name__ == "__main__":
    sys.exit(main())
