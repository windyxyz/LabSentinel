"""T-333（L4）：Rust 单二进制分发构建。

产物包：labsentinel-server.exe + web/dist + models/ + go2rtc + THIRD_PARTY_LICENSES.txt
（不含嵌入式 CPython runtime ≈ 280MB → 单二进制 + 资源 ≈ 数十 MB）。
Python 源码按 76 文档 L4 语义保留一个版本周期于 server/（legacy），训练工具链 tools/ 长期保留。

用法: python scripts/build_rust.py [--zip]
"""
import os
import shutil
import subprocess
import sys
import zipfile
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / "Resources" / "dist" / "rust" / "labsentinel-server"
CARGO = Path(os.environ.get("CARGO", str(Path.home() / ".cargo/bin/cargo.exe")))
TARGET = Path(os.environ.get("CARGO_TARGET_DIR", "D:/tmp/ls-rs-target"))
VERSION = (ROOT / "VERSION").read_text("ascii").strip()
W64 = Path("D:/tmp/w64devkit/w64devkit")


def build_binary() -> Path:
    """CI（MSVC/Linux）走宿主默认工具链；本机 w64devkit 配方自动探测启用。"""
    env = dict(os.environ)
    if W64.exists() and sys.platform == "win32":
        # 本机 GNU 工具链配方（rustup self-contained 缺 as/cc1；w64devkit 为 UCRT 版）
        rustup_sc = (Path.home() / ".rustup/toolchains/stable-x86_64-pc-windows-gnu/"
                     "lib/rustlib/x86_64-pc-windows-gnu/lib/self-contained")
        env["PATH"] = f"{W64 / 'bin'};{Path.home() / '.cargo/bin'};" + env.get("PATH", "")
        env["RUSTFLAGS"] = (f"-L {rustup_sc} "
                            f"-L {W64 / 'lib/gcc/x86_64-w64-mingw32/15.2.0'} "
                            f"-L {W64 / 'lib'} -C link-arg=-lucrt")
    env.setdefault("CARGO_TARGET_DIR", str(TARGET))
    r = subprocess.run([str(CARGO), "build", "--release", "-p", "ls-server"],
                       cwd=ROOT / "engine-rs", env=env)
    if r.returncode:
        raise SystemExit("[失败] cargo build -p ls-server 失败（先确认 w64devkit 构建配方）")
    name = "labsentinel-server.exe" if sys.platform == "win32" else "labsentinel-server"
    exe = TARGET / "release" / name
    if not exe.exists():
        raise SystemExit(f"[失败] 未找到产物 {exe}")
    return exe


def assemble() -> None:
    DIST.mkdir(parents=True, exist_ok=True)
    exe = build_binary()
    shutil.copy2(exe, DIST / exe.name)
    web = ROOT / "web" / "dist"
    if (web / "index.html").exists():
        shutil.copytree(web, DIST / "web", dirs_exist_ok=True)
    models = DIST / "models"
    models.mkdir(exist_ok=True)
    for name in ("yolo11n.onnx",):
        src = ROOT / "m0-spike" / name
        if src.exists():
            shutil.copy2(src, models / name)
    g2 = ROOT / "tools" / "go2rtc.exe"
    if g2.exists():
        (DIST / "tools").mkdir(exist_ok=True)
        shutil.copy2(g2, DIST / "tools" / "go2rtc.exe")
    notice = ROOT / "NOTICE"
    if notice.exists():
        shutil.copy2(notice, DIST / "NOTICE.txt")
    (DIST / "THIRD_PARTY_LICENSES.txt").write_text(
        "Rust crates: 见 engine-rs/Cargo.lock（cargo-deny 生成完整清单）\n"
        "go2rtc: MIT · onnxruntime: MIT · OpenCV(ffmpeg): 见 NOTICE\n", encoding="utf-8")
    (DIST / "README.txt").write_text(
        f"LabSentinel {VERSION} — Rust 单二进制分发\n"
        "启动: labsentinel-server.exe（LS_DB_PATH / LS_PORT / LS_ADMIN_TOKEN）\n"
        "模型目录 models/，前端 web/，实时画面 go2rtc 见 tools/\n"
        "训练/量化/标注工具链仍在仓库 tools/（Python，AI 生态依赖）\n", encoding="utf-8")


def zip_it() -> Path:
    out = ROOT / "Resources" / "dist" / f"labsentinel-rust-{VERSION}-{date.today():%Y%m%d}.zip"
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as z:
        for f in DIST.rglob("*"):
            if f.is_file():
                z.write(f, f.relative_to(DIST))
    print(f"[打包] {out} ({out.stat().st_size / 1e6:.1f}MB)")
    return out


if __name__ == "__main__":
    assemble()
    if "--zip" in sys.argv:
        zip_it()
    print("[L4 构建] 完成")
