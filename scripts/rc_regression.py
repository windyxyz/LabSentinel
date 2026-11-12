"""RC 一键回归（任务97）：统一编排 pytest/build/release_check/manifest/smoke/doctor。

用法（标准库）:
    python scripts/rc_regression.py [--fast] [--smoke] [--zip <path>] [--python <exe>] [--json out.json]

每步失败继续跑完并汇总；任一 FAIL → exit 2。不修改任何文件（除 --json 输出）。
"""
import argparse
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
_PY = sys.executable
_DEFAULT_PYTEST_PY = r"D:\LabSentinel\m0-venv\Scripts\python.exe"


def log(m=""):
    print(m, flush=True)


def _run(name, cmd, cwd, timeout, results, stop_on_fail=False, shell=False):
    t0 = time.monotonic()
    try:
        r = subprocess.run(cmd, cwd=str(cwd), capture_output=True, text=True,
                           errors="replace", timeout=timeout, shell=shell)
        rc = r.returncode
        detail = (r.stdout.strip() or r.stderr.strip())[-200:] if rc != 0 else \
            (r.stdout.strip().splitlines()[-1] if r.stdout.strip() else "OK")
    except subprocess.TimeoutExpired:
        rc, detail = -1, f"超时（>{timeout}s）"
    except OSError as e:
        rc, detail = -1, str(e)[:120]
    dur = round(time.monotonic() - t0, 1)
    status = "PASS" if rc == 0 else "FAIL"
    results.append({"step": name, "status": status, "duration": dur, "detail": detail[:200]})
    mark = "✓" if status == "PASS" else "✗"
    log(f" [{mark}] {status:<4} {name}  {dur}s  {detail[:100]}")
    return rc == 0


def run_regression(fast=False, smoke=False, zip_path=None, py_exe=None) -> list[dict]:
    results: list[dict] = []
    py = py_exe or _PY
    server_dir = ROOT / "server"
    web_dir = ROOT / "web"

    # 1. pytest
    pytest_py = py  # --python 传入时用之
    if py_exe is None and Path(r"D:\LabSentinel\m0-venv\Scripts\python.exe").exists():
        pytest_py = r"D:\LabSentinel\m0-venv\Scripts\python.exe"
    _run("pytest", [pytest_py, "-m", "pytest", "tests/", "-q"], server_dir, 600, results)

    # 2. web build（2026-11-13 P3：list + shell=True 在 POSIX 上首参后全变 shell
    # 位置参数，实际执行裸 npm 可能假通过——用 which 解析 npm.cmd 去掉 shell）
    npm = shutil.which("npm") or "npm"
    _run("npm build", [npm, "run", "build"], web_dir, 300, results)

    # 3. release_check
    _run("release_check", [sys.executable, str(ROOT / "scripts" / "release_check.py"),
                           "--zip", str(zip_path)] if zip_path else
         [sys.executable, str(ROOT / "scripts" / "release_check.py")],
         ROOT, 180, results)

    # 4. doctor
    _run("doctor", [sys.executable, str(ROOT / "scripts" / "doctor.py"), "--port", "8190"], ROOT, 120, results)

    # 5. manifest + verify
    if zip_path:
        _run("manifest", [sys.executable, str(ROOT / "scripts" / "release_manifest.py"),
                          "--zip", str(zip_path)], ROOT, 120, results)
        _run("manifest verify", [sys.executable, str(ROOT / "scripts" / "release_manifest.py"),
                                 "--verify", str(ROOT / "Resources" / "dist")], ROOT, 60, results)
    elif not fast:
        _run("manifest", [sys.executable, str(ROOT / "scripts" / "release_manifest.py")],
             ROOT, 120, results)

    # 6. smoke（--smoke 才跑）
    if smoke and zip_path:
        _run("smoke", [sys.executable, str(ROOT / "scripts" / "smoke_portable.py"),
                       "--zip", str(zip_path)], ROOT, 300, results)

    return results


def main() -> int:
    ap = argparse.ArgumentParser(description="RC 一键回归（任务97）")
    ap.add_argument("--fast", action="store_true", help="跳过 smoke 与 manifest")
    ap.add_argument("--smoke", action="store_true", help="含便携包冒烟（需 zip）")
    ap.add_argument("--zip", type=Path, help="便携包 zip 路径")
    ap.add_argument("--python", type=Path, help="pytest 用的 Python 解释器（默认当前）")
    ap.add_argument("--json", type=Path, help="输出 JSON 结果文件")
    ap.add_argument("--stop-on-fail", action="store_true", help="第一个 FAIL 即停止")
    a = ap.parse_args()

    results = run_regression(a.fast, a.smoke, a.zip, a.python)
    n_fail = sum(1 for r in results if r["status"] == "FAIL")

    log(f"\n===== RC 回归汇总: {len(results) - n_fail}/{len(results)} PASS =====")
    for r in results:
        mark = "✓" if r["status"] == "PASS" else "✗"
        log(f" [{mark}] {r['status']:<4} {r['step']:<20} {r['duration']:>6}s")
    if a.json:
        a.json.write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
        log(f"JSON: {a.json}")
    return 2 if n_fail else 0


if __name__ == "__main__":
    sys.exit(main())
