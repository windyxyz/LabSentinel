"""T-333（L4）：Rust 单二进制分发构建（实验产物，未达 L3 门槛不进正式 Release）。

产物包：labsentinel-server.exe + web/dist + models/ + go2rtc + THIRD_PARTY_LICENSES.txt
（不含嵌入式 CPython runtime ≈ 280MB → 单二进制 + 资源 ≈ 数十 MB）。
Python 源码按 76 文档 L4 语义保留一个版本周期于 server/（legacy），训练工具链 tools/ 长期保留。

用法: python scripts/build_rust.py [--zip]

R2-003/R2-506：不再写死 D:/tmp 路径。
  - 产物目录默认 engine-rs/target（仓库相对，已 gitignore），可用 CARGO_TARGET_DIR 覆盖。
  - 本机 Windows GNU 配方需要 LS_W64DEVKIT 指向 w64devkit 根目录；未设置时用宿主
    默认工具链（CI 的 MSVC / Linux 走这条）。rust-toolchain.toml 只钉 channel，
    目标三元组由显式 `cargo +stable-x86_64-pc-windows-gnu` 选择。
"""
import os
import shutil
import subprocess
import sys
import zipfile
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ENGINE_RS = ROOT / "engine-rs"
DIST = ROOT / "Resources" / "dist" / "rust" / "labsentinel-server"
CARGO = Path(os.environ.get("CARGO", str(Path.home() / ".cargo/bin/cargo.exe")))
TARGET = Path(os.environ.get("CARGO_TARGET_DIR", str(ENGINE_RS / "target")))
VERSION = (ROOT / "VERSION").read_text("ascii").strip()
# R2-506: no hard-coded developer-machine path; opt in via environment.
W64 = Path(os.environ["LS_W64DEVKIT"]) if os.environ.get("LS_W64DEVKIT") else None


def _gcc_lib_dir(w64: Path) -> Path:
    """Pick the newest GCC version dir instead of pinning 15.2.0."""
    base = w64 / "lib" / "gcc" / "x86_64-w64-mingw32"
    versions = sorted(p for p in base.iterdir() if p.is_dir()) if base.is_dir() else []
    return versions[-1] if versions else base


def build_binary() -> Path:
    """CI（MSVC/Linux）走宿主默认工具链；本机 w64devkit 配方按 LS_W64DEVKIT 启用。"""
    env = dict(os.environ)
    args = [str(CARGO)]
    if W64 and W64.is_dir() and sys.platform == "win32":
        # rust-toolchain.toml pins channel=stable (host = msvc here); select the
        # GNU toolchain explicitly for the self-contained Windows GNU build.
        args.append("+stable-x86_64-pc-windows-gnu")
        home = Path.home()
        rustup_sc = (home / ".rustup/toolchains/stable-x86_64-pc-windows-gnu/"
                     "lib/rustlib/x86_64-pc-windows-gnu/lib/self-contained")
        env["PATH"] = f"{W64 / 'bin'};{home / '.cargo/bin'};" + env.get("PATH", "")
        env["RUSTFLAGS"] = (f"-L {rustup_sc} "
                            f"-L {_gcc_lib_dir(W64)} "
                            f"-L {W64 / 'lib'} -C link-arg=-lucrt")
    args += ["build", "--release", "-p", "ls-server"]
    env.setdefault("CARGO_TARGET_DIR", str(TARGET))
    r = subprocess.run(args, cwd=ENGINE_RS, env=env)
    if r.returncode:
        raise SystemExit("[失败] cargo build -p ls-server 失败（先确认工具链/LS_W64DEVKIT 构建配方）")
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
        f"LabSentinel {VERSION} — Rust 单二进制分发（ADR-0011 RA3 正式形态）\n"
        "启动: labsentinel-server.exe（LS_DB_PATH / LS_DATA_DIR / LS_PORT / LS_ADMIN_TOKEN；\n"
        "      进程内分析 worker：LS_MODEL_PATH=models/yolo11n.onnx 即启用检测）\n"
        "模型目录 models/，前端 web/（SPA 回退+immutable 缓存），实时画面 go2rtc 见 tools/\n"
        "安装/升级/卸载链仍走 Python 便携包（RA4 评估替换）；训练工具链 tools/（Python）\n", encoding="utf-8")
    (DIST / ".env.example").write_text(
        "# LabSentinel Rust 形态最小配置（重命名 .env 生效；全部可省略）\n"
        "LS_DB_PATH=data/labsentinel.db\n"
        "LS_DATA_DIR=data\n"
        "LS_PORT=8000\n"
        "# 局域网暴露必须配令牌（否则拒绝启动，与 Python 同语义）\n"
        "# LS_HOST=0.0.0.0\n# LS_ADMIN_TOKEN=<随机>\n"
        "# 进程内分析 worker\n"
        "LS_MODEL_PATH=models/yolo11n.onnx\n"
        "# LS_PPE_MODEL=models/yolo11n_ppe.onnx\n", encoding="utf-8")


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
