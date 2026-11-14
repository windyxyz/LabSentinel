"""T-333（L4）/RA4.2：Rust 单二进制正式分发构建（ADR-0011 过渡期末形态）。

产物包：labsentinel-server.exe + VERSION + LabSentinel.bat + web/dist + models/
（yolo11n + PPE）+ tools/ffmpeg + tools/onnxruntime + go2rtc + 许可/说明。
不含嵌入式 CPython runtime → 单二进制 + 资源，双击启动即用，无需 Python/Node。
Python 源码在过渡期内保留于 server/（legacy，退役后移除）；训练工具链 tools/ 长期保留。

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


def assemble(strict: bool = False) -> None:
    DIST.mkdir(parents=True, exist_ok=True)
    missing: list[str] = []
    exe = build_binary()
    shutil.copy2(exe, DIST / exe.name)
    web = ROOT / "web" / "dist"
    if (web / "index.html").exists():
        shutil.copytree(web, DIST / "web", dirs_exist_ok=True)
    else:
        missing.append("web/dist（先执行 npm run build）")
    models = DIST / "models"
    models.mkdir(exist_ok=True)
    # RA4.2：基础检测 + PPE 模型（含类别名）随包分发（缺失时提示，不阻断——仅检测功能降级）
    for name in ("yolo11n.onnx", "yolo11n_ppe.onnx", "yolo11n_ppe.classes.txt"):
        src = ROOT / "m0-spike" / name
        if src.exists():
            shutil.copy2(src, models / name)
        elif name == "yolo11n.onnx":
            missing.append(f"模型 {src}（scripts/prepare_rust_assets.py）")
            print(f"[警告] 缺少基础模型 {src}（scripts/fetch_assets.py --name yolo11n.onnx）")
        else:
            # Q-08：PPE 许可待确认不随包分发——缺失仅警告（CI 无该资产）
            print(f"[警告] 缺少 {name}（Q-08：实验服模型按需在网页端确认许可后下载）")
    # RA4.2：ffmpeg/ffprobe（RTSP/视频取流与存证）与 onnxruntime（ORT_DYLIB_PATH）
    ff_src = ROOT / "tools" / "ffmpeg"
    if ff_src.is_dir() and (ff_src / "ffmpeg.exe").is_file():
        shutil.copytree(ff_src, DIST / "tools" / "ffmpeg", dirs_exist_ok=True)
    else:
        missing.append("tools/ffmpeg/ffmpeg.exe（scripts/prepare_rust_assets.py）")
        print(f"[警告] 缺少 {ff_src}/ffmpeg.exe（RTSP/视频源将不可用）")
    ort_src = ROOT / "tools" / "onnxruntime"
    if ort_src.is_dir() and (ort_src / "onnxruntime.dll").is_file():
        shutil.copytree(ort_src, DIST / "tools" / "onnxruntime", dirs_exist_ok=True)
        # exe 同目录兜底：Windows DLL 搜索优先应用目录，双击 exe 也能加载
        for dll in ort_src.glob("*.dll"):
            shutil.copy2(dll, DIST / dll.name)
    else:
        missing.append("tools/onnxruntime/onnxruntime.dll（pip install onnxruntime 后重跑）")
        print(f"[警告] 缺少 {ort_src}/onnxruntime.dll（推理将无法加载）")
    g2 = ROOT / "tools" / "go2rtc.exe"
    if g2.exists():
        (DIST / "tools").mkdir(exist_ok=True)
        shutil.copy2(g2, DIST / "tools" / "go2rtc.exe")
    gy = ROOT / "tools" / "go2rtc.yaml"
    if gy.exists():
        shutil.copy2(gy, DIST / "tools" / "go2rtc.yaml")
    notice = ROOT / "NOTICE"
    if notice.exists():
        shutil.copy2(notice, DIST / "NOTICE.txt")
    (DIST / "VERSION").write_text(f"{VERSION}\n", encoding="ascii")
    # RA4.2：便携启动器（双击即起服务并开控制台；数据默认落 LOCALAPPDATA）
    launcher = (
        "@echo off\r\n"
        "setlocal\r\n"
        'cd /d "%~dp0"\r\n'
        'if "%LS_DATA_DIR%"=="" set "LS_DATA_DIR=%LOCALAPPDATA%\\LabSentinel\\data"\r\n'
        'if "%LS_DB_PATH%"=="" set "LS_DB_PATH=%LS_DATA_DIR%\\labsentinel.db"\r\n'
        'if "%LS_WEB_DIST%"=="" set "LS_WEB_DIST=%~dp0web"\r\n'
        'if "%LS_MODEL_PATH%"=="" if exist "%~dp0models\\yolo11n.onnx" '
        'set "LS_MODEL_PATH=%~dp0models\\yolo11n.onnx"\r\n'
        'if "%LS_PPE_MODEL%"=="" if exist "%~dp0models\\yolo11n_ppe.onnx" '
        'set "LS_PPE_MODEL=%~dp0models\\yolo11n_ppe.onnx"\r\n'
        'if "%ORT_DYLIB_PATH%"=="" if exist "%~dp0onnxruntime.dll" '
        'set "ORT_DYLIB_PATH=%~dp0onnxruntime.dll"\r\n'
        'if not exist "%LS_DATA_DIR%" mkdir "%LS_DATA_DIR%"\r\n'
        '"%~dp0labsentinel-server.exe" --launch\r\n'
        "if errorlevel 1 pause\r\n"
    )
    (DIST / "LabSentinel.bat").write_bytes(launcher.encode("ascii"))
    (DIST / "THIRD_PARTY_LICENSES.txt").write_text(
        "Rust crates: 见 engine-rs/Cargo.lock（cargo-deny 生成完整清单）\n"
        "go2rtc: MIT · onnxruntime: MIT · FFmpeg: LGPL/GPL（见 tools/ffmpeg 附带的许可文本）\n"
        "模型权重许可与来源见 models/ 与 docs/adr/0009-alignment-vs-gpu-and-model-provenance.md\n",
        encoding="utf-8")
    (DIST / "README.txt").write_text(
        f"LabSentinel {VERSION} — Rust 单二进制分发（ADR-0011 正式形态，无需 Python/Node）\n"
        "\n启动：双击 LabSentinel.bat（数据默认 %LOCALAPPDATA%\\LabSentinel\\data，浏览器自动打开）\n"
        "  等价命令行：labsentinel-server.exe --launch\n"
        "  停止：网页「设置 → 服务控制 → 停止服务」，或任务管理器结束进程\n"
        "升级：解压新包覆盖（旧版本自动归档到 versions/）；回滚：\n"
        "  labsentinel-server.exe --rollback <安装根>\n"
        "\n可选环境变量：LS_PORT（默认 8000）/ LS_ADMIN_TOKEN / LS_HOST（局域网必须配令牌）\n"
        "  LS_MODEL_PATH=models/yolo11n.onnx  LS_PPE_MODEL=models/yolo11n_ppe.onnx\n"
        "  LS_FFMPEG / LS_FFPROBE 覆盖外部二进制路径（默认 tools/ffmpeg/）\n"
        "\n包内：web/（前端）models/（检测模型）tools/ffmpeg（取流）tools/onnxruntime（推理）\n"
        "      tools/go2rtc.exe（实时画面，可选）\n", encoding="utf-8")
    (DIST / ".env.example").write_text(
        "# LabSentinel 最小配置（重命名 .env 生效；全部可省略）\n"
        "LS_PORT=8000\n"
        "# 局域网暴露必须配令牌（否则拒绝启动）\n"
        "# LS_HOST=0.0.0.0\n# LS_ADMIN_TOKEN=<随机>\n"
        "# 进程内分析 worker（LabSentinel.bat 已自动指向包内模型）\n"
        "# LS_MODEL_PATH=models/yolo11n.onnx\n"
        "# LS_PPE_MODEL=models/yolo11n_ppe.onnx\n", encoding="utf-8")
    if missing:
        if strict:
            raise SystemExit("[失败] 发行包缺少必需资产：\n  - " + "\n  - ".join(missing))
        print("[警告] 发行包资产不完整：" + "；".join(missing))


def zip_it() -> Path:
    out = ROOT / "Resources" / "dist" / f"labsentinel-rust-{VERSION}-{date.today():%Y%m%d}.zip"
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as z:
        for f in DIST.rglob("*"):
            if f.is_file():
                z.write(f, f.relative_to(DIST))
    print(f"[打包] {out} ({out.stat().st_size / 1e6:.1f}MB)")
    return out


if __name__ == "__main__":
    assemble(strict="--strict" in sys.argv)
    if "--zip" in sys.argv:
        zip_it()
    print("[L4 构建] 完成")
