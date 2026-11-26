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
import argparse
import os
import platform
import shutil
import subprocess
import sys
import zipfile
from datetime import date
from pathlib import Path

from prepare_rust_assets import GO2RTC, MODELS, PORTABLE_MODEL_ASSETS, valid_model_asset, valid_go2rtc
from vulkan_assets import read_bundle as read_vulkan_bundle

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


def build_binary(vulkan_cv: bool = False) -> Path:
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
    if vulkan_cv:
        args += ["--features", "vulkan-cv"]
    env.setdefault("CARGO_TARGET_DIR", str(TARGET))
    r = subprocess.run(args, cwd=ENGINE_RS, env=env,
                       creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    if r.returncode:
        raise SystemExit("[失败] cargo build -p ls-server 失败（先确认工具链/LS_W64DEVKIT 构建配方）")
    name = "labsentinel-server.exe" if sys.platform == "win32" else "labsentinel-server"
    exe = TARGET / "release" / name
    if not exe.exists():
        raise SystemExit(f"[失败] 未找到产物 {exe}")
    return exe


def assemble(strict: bool = False, vulkan_probe: Path | None = None) -> None:
    if vulkan_probe and (sys.platform != "win32" or platform.machine().lower() not in {"amd64", "x86_64"}):
        raise SystemExit("[失败] Vulkan 探针包当前仅支持 Windows x64")
    vulkan = read_vulkan_bundle(vulkan_probe, MODELS) if vulkan_probe else None
    invalid_models = {name for name in PORTABLE_MODEL_ASSETS if not valid_model_asset(MODELS / name)}
    if (strict or vulkan is not None) and invalid_models:
        names = ", ".join(sorted(invalid_models))
        raise SystemExit(f"[失败] 权重缺失或校验失败 {names}（scripts/prepare_rust_assets.py）")
    # LS-045：先清空 staging 目录再组装，防止历史版本废弃资产残留进新发行包
    if DIST.exists():
        shutil.rmtree(DIST)
    DIST.mkdir(parents=True, exist_ok=True)
    missing: list[str] = []
    exe = build_binary(vulkan_cv=True) if vulkan is not None else build_binary()
    shutil.copy2(exe, DIST / exe.name)
    web = ROOT / "web" / "dist"
    if (web / "index.html").exists():
        shutil.copytree(web, DIST / "web", dirs_exist_ok=True)
    else:
        missing.append("web/dist（先执行 npm run build）")
    models = DIST / "models"
    models.mkdir(exist_ok=True)
    # RA4.2：基础检测 + PPE 模型（含类别名）随包分发（--strict 缺任一即失败）
    # 2026-11-21（用户指令）：实验服模型改为随包发布——此前 Q-08 只警告，
    # 安装版用户看到「仅人员检测」且下载源 404。
    for name in PORTABLE_MODEL_ASSETS:
        src = MODELS / name
        if name not in invalid_models:
            target = models / name
            shutil.copy2(src, target)
            if valid_model_asset(target):
                continue
            target.unlink()
        missing.append(f"模型缺失或校验失败 {src}（scripts/prepare_rust_assets.py）")
    shutil.copy2(ROOT / "docs/licenses/close-mask-MIT.txt", models / "close-mask-LICENSE.txt")
    if vulkan is not None:
        target = DIST / "vulkan"
        target.mkdir()
        for name, data in vulkan.items():
            (target / name).write_bytes(data)
    # RA4.2：ffmpeg/ffprobe（RTSP/视频取流与存证）与 onnxruntime（ORT_DYLIB_PATH）
    ff_src = ROOT / "tools" / "ffmpeg"
    if ff_src.is_dir() and (ff_src / "ffmpeg.exe").is_file():
        shutil.copytree(ff_src, DIST / "tools" / "ffmpeg", dirs_exist_ok=True)
    else:
        missing.append("tools/ffmpeg/ffmpeg.exe（scripts/prepare_rust_assets.py）")
        print(f"[警告] 缺少 {ff_src}/ffmpeg.exe（RTSP/视频源将不可用）")
    ort_src = ROOT / "tools" / "onnxruntime"
    if ort_src.is_dir() and (ort_src / "onnxruntime.dll").is_file():
        # LS-045：仅复制到 exe 同目录（Windows DLL 搜索优先应用目录，LabSentinel.bat
        # 与 ls-infer 均优先读同目录 onnxruntime.dll），避免在 tools/onnxruntime 重复
        # 存一份导致安装包虚增 ~18MB。
        for dll in ort_src.glob("*.dll"):
            shutil.copy2(dll, DIST / dll.name)
    else:
        missing.append("tools/onnxruntime/onnxruntime.dll（运行 scripts/prepare_rust_assets.py）")
        print(f"[警告] 缺少 {ort_src}/onnxruntime.dll（推理将无法加载）")
    if valid_go2rtc(GO2RTC):
        (DIST / "tools").mkdir(exist_ok=True)
        shutil.copy2(GO2RTC, DIST / "tools" / "go2rtc.exe")
        shutil.copy2(ROOT / "docs" / "licenses" / "go2rtc-MIT.txt", DIST / "tools" / "go2rtc-LICENSE.txt")
    else:
        missing.append("tools/go2rtc.exe（缺失或校验失败，请运行 scripts/prepare_rust_assets.py）")
    notice = ROOT / "NOTICE"
    if notice.exists():
        shutil.copy2(notice, DIST / "NOTICE.txt")
    shutil.copy2(ROOT / "LICENSE", DIST / "LICENSE")
    (DIST / "VERSION").write_text(f"{VERSION}\n", encoding="ascii")
    # RA4.2：便携启动器（双击即起服务并开控制台；数据默认落 LOCALAPPDATA）
    # UX3（2026-11-20）：无黑框——start 拉起 GUI 子系统 exe（--tray 托盘常驻），
    # cmd 窗口立即关闭；致命错误由 exe 弹原生 MessageBox + 落盘日志（不再依赖
    # 控制台 errorlevel 暂停）
    launcher = (
        "@echo off\r\n"
        "setlocal\r\n"
        'cd /d "%~dp0"\r\n'
        'if "%LS_DATA_DIR%"=="" set "LS_DATA_DIR=%LOCALAPPDATA%\\LabSentinel\\data"\r\n'
        'if "%LS_DB_PATH%"=="" set "LS_DB_PATH=%LS_DATA_DIR%\\labsentinel.db"\r\n'
        'if "%LS_WEB_DIST%"=="" set "LS_WEB_DIST=%~dp0web"\r\n'
        'if "%ORT_DYLIB_PATH%"=="" if exist "%~dp0onnxruntime.dll" '
        'set "ORT_DYLIB_PATH=%~dp0onnxruntime.dll"\r\n'
        'if not exist "%LS_DATA_DIR%" mkdir "%LS_DATA_DIR%"\r\n'
        'start "" "%~dp0labsentinel-server.exe" --launch --tray\r\n'
    )
    (DIST / "LabSentinel.bat").write_bytes(launcher.encode("ascii"))
    (DIST / "THIRD_PARTY_LICENSES.txt").write_text(
        "Rust crates: 见 engine-rs/Cargo.lock（cargo-deny 生成完整清单）\n"
        "go2rtc: MIT · onnxruntime: MIT · FFmpeg: LGPL/GPL（见 tools/ffmpeg 附带的许可文本）\n"
        "模型权重许可与来源见 models/ 与 docs/adr/0009-alignment-vs-gpu-and-model-provenance.md\n",
        encoding="utf-8")
    (DIST / "README.txt").write_text(
        f"LabSentinel {VERSION} — Rust 单二进制分发（ADR-0011 正式形态，无需 Python/Node）\n"
        "\n启动：双击 LabSentinel.bat（无窗口；数据默认 %LOCALAPPDATA%\\LabSentinel\\data，"
        "浏览器自动打开；任务栏托盘常驻——打开控制台/复制地址/退出）\n"
        "  等价命令行：labsentinel-server.exe --launch --tray（排障加 --console）\n"
        "  停止：网页「设置 → 服务控制 → 停止服务」，或任务管理器结束进程\n"
        "升级：解压新包覆盖（旧版本自动归档到 versions/）；回滚：\n"
        "  labsentinel-server.exe --rollback <安装根>\n"
        "\n可选环境变量：LS_PORT（默认 8000）/ LS_ADMIN_TOKEN / LS_HOST（局域网必须配令牌）\n"
        "  LS_MODEL_PATH=models/yolo11n.onnx  LS_PPE_MODEL=models/yolo11n_ppe.onnx\n"
        "  LS_FFMPEG / LS_FFPROBE 覆盖外部二进制路径（默认 tools/ffmpeg/）\n"
        "\n包内：web/（前端）models/（检测模型）tools/ffmpeg（取流）tools/onnxruntime（推理）\n"
        "      tools/go2rtc.exe（RTSP 实时画面）\n", encoding="utf-8")
    (DIST / ".env.example").write_text(
        "# LabSentinel 最小配置（重命名 .env 生效；全部可省略）\n"
        "LS_PORT=8000\n"
        "# 局域网暴露必须配令牌（否则拒绝启动）\n"
        "# LS_HOST=0.0.0.0\n# LS_ADMIN_TOKEN=<随机>\n"
        "# 进程内分析 worker（LabSentinel.bat 已自动指向包内模型）\n"
        "# LS_MODEL_PATH=models/yolo11n.onnx\n"
        "# LS_PPE_MODEL=models/yolo11n_ppe.onnx\n", encoding="utf-8")
    if missing:
        if strict or vulkan is not None:
            raise SystemExit("[失败] 发行包缺少必需资产：\n  - " + "\n  - ".join(missing))
        print("[警告] 发行包资产不完整：" + "；".join(missing))


def zip_it(output_dir: Path | None = None) -> Path:
    variant = "vulkan-probe" if (DIST / "vulkan").is_dir() else "rust"
    output_dir = output_dir if output_dir is not None else ROOT / "Resources" / "dist"
    out = output_dir / f"labsentinel-{variant}-{VERSION}-{date.today():%Y%m%d}.zip"
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as z:
        for f in DIST.rglob("*"):
            if f.is_file():
                z.write(f, f.relative_to(DIST))
    print(f"[打包] {out} ({out.stat().st_size / 1e6:.1f}MB)")
    return out


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--strict", action="store_true")
    parser.add_argument("--zip", action="store_true")
    parser.add_argument("--vulkan-probe", type=Path, help="use the pinned Vulkan probe bundle in a separate staging directory")
    options = parser.parse_args()
    if options.vulkan_probe:
        DIST = DIST.with_name("labsentinel-vulkan-probe")
    assemble(strict=options.strict, vulkan_probe=options.vulkan_probe)
    if options.zip:
        zip_it()
    print("[L4 构建] 完成")
