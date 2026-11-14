# -*- coding: utf-8 -*-
"""用 Inno Setup（ISCC.exe）把 Rust 发行树打包成 setup.exe。

输入：Resources/dist/rust/labsentinel-server（scripts/build_rust.py 的 staging）
输出：Resources/dist/LabSentinel-setup-<VERSION>.exe

跨环境：版本取自 VERSION 文件；ISCC 依次从环境变量 ISCC、Inno Setup 7/6
默认安装目录、choco（CI）路径查找。用法：
  python scripts/build_setup_exe.py [--version X.Y.Z] [--source DIR] [--out DIR]
"""
import argparse
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

ISCC_CANDIDATES = [
    Path(r"D:\Program Files\Inno Setup 7\ISCC.exe"),
    Path(r"C:\Program Files\Inno Setup 7\ISCC.exe"),
    Path(r"C:\Program Files (x86)\Inno Setup 6\ISCC.exe"),
    Path(r"C:\Program Files\Inno Setup 6\ISCC.exe"),
]


def find_iscc() -> Path | None:
    """环境变量优先（CI 可显式指定），然后本机/Choco 默认目录。"""
    env = os.environ.get("ISCC")
    if env:
        p = Path(env)
        if p.is_file():
            return p
        print(f"[警告] ISCC 环境变量指向的文件不存在：{p}")
    return next((c for c in ISCC_CANDIDATES if c.is_file()), None)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", default=(ROOT / "VERSION").read_text("ascii").strip())
    ap.add_argument("--source", default=str(ROOT / "Resources" / "dist" / "rust" / "labsentinel-server"))
    ap.add_argument("--out", default=str(ROOT / "Resources" / "dist"))
    a = ap.parse_args()

    iscc = find_iscc()
    if iscc is None:
        print("[失败] 未找到 ISCC.exe（设置 ISCC 环境变量，或安装 Inno Setup 6/7）")
        return 2

    src = Path(a.source)
    if not (src / "labsentinel-server.exe").is_file():
        print(f"[失败] staging 缺 labsentinel-server.exe：{src}（先跑 scripts/build_rust.py）")
        return 2

    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    cmd = [
        str(iscc),
        f"/DLsAppVersion={a.version}",
        f"/DLsSourceDir={src}",
        f"/DLsOutputDir={out}",
        str(ROOT / "deploy" / "windows" / "installer.iss"),
    ]
    print("ISCC:", cmd, flush=True)
    r = subprocess.run(cmd, timeout=900)
    print("rc:", r.returncode)
    if r.returncode != 0:
        return r.returncode
    produced = sorted(out.glob(f"LabSentinel-setup-{a.version}.exe"))
    if not produced:
        print("[失败] ISCC 返回 0 但未找到 setup.exe 产物")
        return 1
    for p in produced:
        print(f"[OK] {p.name} ({p.stat().st_size / 1e6:.1f}MB)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
