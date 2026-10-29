"""便携包启动 EXE 构建器（任务55）：用 .NET Framework 自带 csc 编译 LabSentinelLauncher.cs。

用法（任意 Python，标准库）:
    python scripts/build_launcher_exe.py --out <便携包根>   # 产出 <便携包根>/LabSentinel.exe
    python scripts/build_launcher_exe.py --out ... --check  # 只探测 csc 并打印路径，不编译

csc 查找顺序：%WINDIR%\\Microsoft.NET\\Framework64\\v4.0.30319\\csc.exe → Framework(32 位) → PATH。
csc 缺失或编译失败：打印警告并返回 None/退出码 1——便携包没有 EXE 仍可用 启动.bat，
build_portable.py 集成时只警告不阻塞。
"""
import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
CS = HERE / "launcher_exe" / "LabSentinelLauncher.cs"
CSC_CANDIDATES = (
    r"Microsoft.NET\Framework64\v4.0.30319\csc.exe",
    r"Microsoft.NET\Framework\v4.0.30319\csc.exe",
)


def log(m=""):
    print(m, flush=True)


def find_csc() -> Path | None:
    windir = Path(os.environ.get("WINDIR", r"C:\Windows"))
    for rel in CSC_CANDIDATES:
        p = windir / rel
        if p.exists():
            return p
    found = shutil.which("csc.exe")
    return Path(found) if found else None


def compile_exe(out_dir: Path) -> Path | None:
    """编译到 out_dir/LabSentinel.exe；csc 缺失或编译失败返回 None（不抛异常）。"""
    csc = find_csc()
    if not csc:
        log("[警告] 未找到 csc.exe（.NET Framework 4.x）——跳过 LabSentinel.exe，"
            "包仍可用 启动.bat（Win10/11 一般自带，见执行报告降级说明）")
        return None
    if not CS.exists():
        log(f"[警告] 源码缺失：{CS}")
        return None
    out_dir.mkdir(parents=True, exist_ok=True)
    exe = out_dir / "LabSentinel.exe"
    cmd = [str(csc), "/nologo", "/target:winexe", "/optimize+",
           "/out:" + str(exe), "/r:System.dll", "/r:System.Windows.Forms.dll", str(CS)]
    r = subprocess.run(cmd, capture_output=True, text=True, encoding="mbcs", errors="replace")
    if r.returncode != 0 or not exe.exists():
        log(f"[警告] csc 编译失败（exit {r.returncode}）——跳过 LabSentinel.exe，包仍可用 启动.bat")
        if r.stdout.strip():
            log(r.stdout.strip()[:800])
        if r.stderr.strip():
            log(r.stderr.strip()[:800])
        return None
    log(f"[OK] LabSentinel.exe {exe.stat().st_size / 1024:.1f}KB（winexe 无控制台） csc={csc}")
    return exe


def main() -> int:
    ap = argparse.ArgumentParser(description="LabSentinel 启动 EXE 构建器（任务55）")
    ap.add_argument("--out", type=Path, help="输出目录（便携包根，产 LabSentinel.exe）")
    ap.add_argument("--check", action="store_true", help="只探测 csc 并打印路径，不编译")
    a = ap.parse_args()
    if a.check:
        csc = find_csc()
        log(f"csc: {csc if csc else '未找到（将跳过 EXE）'}")
        return 0 if csc else 1
    if not a.out:
        ap.error("--out 必填（除非 --check）")
    exe = compile_exe(a.out)
    return 0 if exe else 1


if __name__ == "__main__":
    sys.exit(main())
