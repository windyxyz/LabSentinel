"""便携运行时构建器（任务44）：嵌入式 CPython + 预装依赖 + 模型/二进制/前端产物。

用法（m0-venv 或 .venv）:
    python scripts/build_portable.py            # 构建到 dist/portable/LabSentinel/（幂等）
    python scripts/build_portable.py --zip      # 构建后打包 dist/LabSentinel-portable-<日期>.zip

产物布局（docs/58-packaging-strategy.md 方案 B）:
    LabSentinel/
      runtime/            嵌入式 Python 3.12 + Lib/site-packages（预装全部依赖）
      app/server/app/     服务端代码        app/web/        前端构建产物
      models/             ONNX 模型 + 词表
      tools/              ffmpeg.exe / go2rtc.exe
      app/server/.env.template / .env.example
      LICENSE NOTICE

幂等：runtime/python.exe 已存在跳过下载解压；site-packages 已有 fastapi 跳过 pip；资产覆盖复制。
下载全部走镜像回退（npmmirror → 华为云 → python.org），请求前校验公网地址（同 deploy.py）。
"""
import argparse
import hashlib
import os
import shutil
import subprocess
import sys
import zipfile
from datetime import date
from pathlib import Path

# R2-004/N-09: CI has no .venv. A --python CLI arg lets the workflow pass
# sys.executable; venv_python() keeps working unchanged for local builds.
_PYTHON_OVERRIDE: Path | None = None
# R2-303/R2-304: variant & rust packaging are filled in by later batches;
# the CLI is declared now so release.yml can be written once.
_VARIANT = "cpu"
_WITH_RUST = False

sys.path.insert(0, str(Path(__file__).resolve().parent))
import deploy  # 复用 http_download/_assert_public_host/log 与 GBK venv 标记读取

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / "Resources" / "dist" / "portable" / "LabSentinel"
RUNTIME = DIST / "runtime"
SITE = RUNTIME / "Lib" / "site-packages"
PY_VER = "3.12.8"
PY_ZIP = f"python-{PY_VER}-embed-amd64.zip"
# PY_ZIP 下载源与 SHA256 统一在 scripts/assets.lock.json 维护（T-005）
PIP_INDEXES = [
    "https://pypi.tuna.tsinghua.edu.cn/simple",
    "https://mirrors.aliyun.com/pypi/simple",
    "https://pypi.org/simple",
]
_PTH = "python312._pth"
# Q-08：人体模型（Ultralytics AGPL-3.0，与项目兼容）随包；实验服权重许可未确认
# 前不随包，改由设置页「按需下载」（许可确认 + assets.lock.json SHA256 校验）。
# model.json 角色声明与 classes.txt 也随包（后者为模型契约，不含权重/数据）。
MODELS = ["yolo11n.onnx", "model.json"]
# R2-004/N-09: model source directory is overridable so CI can fetch the model
# via scripts/fetch_assets.py into a scratch dir instead of relying on a
# gitignored m0-spike/*.onnx in a clean checkout.
_MODELS_DIR: Path | None = None


def dir_mb(p: Path) -> float:
    return sum(f.stat().st_size for f in p.rglob("*") if f.is_file()) / 1e6 if p.exists() else 0.0


def venv_python() -> Path:
    # R2-004/N-09: CI passes --python sys.executable (no .venv in a clean checkout).
    if _PYTHON_OVERRIDE is not None:
        if not _PYTHON_OVERRIDE.exists():
            raise SystemExit(f"[失败] --python 指定的解释器不存在：{_PYTHON_OVERRIDE}")
        return _PYTHON_OVERRIDE
    v = deploy._read_venv_mark() or (ROOT / ".venv" / "Scripts" / "python.exe")
    if not Path(v).exists():
        raise SystemExit(f"[失败] 找不到构建用 venv python（{v}）；先跑 deploy.py install 或传 --python")
    return Path(v)


def ensure_runtime() -> None:
    if (RUNTIME / "python.exe").exists():
        deploy.log(f"  [跳过] runtime 已存在: {RUNTIME}")
        return
    zpath = ROOT / "Resources" / "logs" / PY_ZIP
    if not zpath.exists():
        deploy.log(f"  [1] 下载 CPython {PY_VER} embed（哈希锁定 assets.lock.json）...")
        deploy.download_verified(PY_ZIP, zpath)  # 下载后强制 SHA256 校验，残件不落盘
    with zipfile.ZipFile(zpath) as z:  # 完整性校验（CRC 全量）
        assert z.testzip() is None, "embed zip 校验失败"
        deploy.log(f"  [2] 解压到 {RUNTIME} ...")
        z.extractall(RUNTIME)
    (RUNTIME / _PTH).write_text("python312.zip\n.\nLib\\site-packages\n", encoding="ascii")
    deploy.log(f"  [OK] {_PTH} 已写（含 Lib\\site-packages）")


def ensure_deps() -> None:
    """T-002（B-03/R-02）：依赖从带哈希锁文件全新安装到干净 site-packages。
    锁文件 sha256 stamp 不一致（requirements 变更/首装/依赖漂移）即清空重装，
    修复旧逻辑"fastapi/cv2 存在即跳过"导致的 requirements 变更不生效与 opencv 双包并存。"""
    # R2-303: cpu (default) vs dml (DirectML) are separate hash-pinned locks;
    # they must never be installed into the same site-packages (B-03 conflict).
    lock = ROOT / "server" / ("requirements-win-dml.lock" if _VARIANT == "dml"
                              else "requirements-win.lock")
    deploy.log(f"  [锁] 变体 {_VARIANT} → {lock.name}")
    stamp = SITE / ".lock.sha256"
    want = hashlib.sha256(lock.read_bytes()).hexdigest()
    if stamp.exists() and stamp.read_text("ascii").strip() == want:
        deploy.log("  [跳过] site-packages 与锁文件一致（stamp 命中）")
        return
    deploy.log("  [3] 依赖全新安装到 runtime/Lib/site-packages（--require-hashes --no-deps，约 5 分钟）...")
    shutil.rmtree(SITE, ignore_errors=True)
    SITE.mkdir(parents=True)
    py = venv_python()
    r = subprocess.run(
        [str(py), "-m", "pip", "install", "--disable-pip-version-check",
         "--no-warn-script-location", "--require-hashes", "--no-deps",
         "--target", str(SITE), "-r", str(lock)],
        env={**os.environ, "PYTHONUTF8": "1"})
    if r.returncode != 0:
        raise SystemExit("[失败] 锁文件依赖安装失败")
    stamp.write_text(want, encoding="ascii")


def copy_assets() -> None:
    deploy.log("  [4] 复制资产（服务端代码/前端产物/模型/二进制，幂等覆盖）...")
    # 任务119 防御：修复前的启动器跑包时 go2rtc 配置会写到 <包>/app/tools/go2rtc.yaml
    # （源码布局的默认值推导错误）。staging 是持久的，旧残留会混进新 zip——每次构建先清掉
    stale_app_tools = DIST / "app" / "tools"
    if stale_app_tools.exists():
        shutil.rmtree(stale_app_tools, ignore_errors=True)
        deploy.log("  [清理] 删除 staging 旧残留 app/tools/（任务119 前 go2rtc.yaml 误写位置）")
    # 服务端代码（排除 data/__pycache__）
    app_dir = DIST / "app" / "server"
    for stale_cache in (app_dir / "app").glob("__pycache__"):  # 任务129：清 staging 旧残留
        shutil.rmtree(stale_cache, ignore_errors=True)
    shutil.copytree(ROOT / "server" / "app", app_dir / "app", dirs_exist_ok=True,
                    ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
    # 前端构建产物
    if not (ROOT / "web" / "dist" / "index.html").exists():
        raise SystemExit("[失败] web/dist/index.html 不存在——先执行 npm run build（本脚本不代跑 npm）")
    shutil.copytree(ROOT / "web" / "dist", DIST / "app" / "web", dirs_exist_ok=True)
    # 模型
    (DIST / "models").mkdir(parents=True, exist_ok=True)
    models_dir = _MODELS_DIR or (ROOT / "m0-spike")
    for name in MODELS:
        # model.json is tracked in git (model contract, no weights); only the
        # .onnx weight comes from --models-dir in CI.
        src = (models_dir if name.endswith(".onnx") else ROOT / "m0-spike") / name
        assert src.exists(), (f"缺模型文件 {src}；CI 请先运行 "
                              "python scripts/fetch_assets.py --name yolo11n.onnx --out <models-dir>/yolo11n.onnx")
        shutil.copy2(src, DIST / "models" / name)
    # 演示视频：T-406/Q-07 起不再随包（Mixkit 素材仅个人使用，公开分发有版权风险）。
    # 首启无演示摄像头，引导用户添加自己的源；自拍 CC0 素材就绪后经 media/SOURCES.md 恢复。
    (DIST / "media").mkdir(parents=True, exist_ok=True)
    sources_md = ROOT / "Resources" / "效果演示" / "SOURCES.md"
    if sources_md.exists():
        shutil.copy2(sources_md, DIST / "media" / "SOURCES.md")
    seed_media = ROOT / "m0-spike" / "data"
    for fn in ("lab-walk-24076.mp4", "lab-equip-21453.mp4"):
        if (seed_media / fn).exists():
            print(f"  [提示] {fn} 不再进包（T-406）；本地保留用于开发/语料")
        else:
            print(f"  [提示] {fn} 不存在（已按 T-406 移除随包分发）")
    # 二进制
    tools = DIST / "tools"
    tools.mkdir(parents=True, exist_ok=True)
    go2rtc = ROOT / "tools" / "go2rtc.exe"
    if go2rtc.exists():
        shutil.copy2(go2rtc, tools / "go2rtc.exe")
    else:
        deploy.log("  [警告] tools/go2rtc.exe 不存在，便携包将没有实时画面（WebRTC）")
    ffmpeg = ROOT / "tools" / "ffmpeg" / "ffmpeg.exe"
    if ffmpeg.exists():
        shutil.copy2(ffmpeg, tools / "ffmpeg.exe")
    else:
        deploy.log("  [警告] tools/ffmpeg/ffmpeg.exe 不存在，便携包将没有实时画面转码（可后补）")
    # R2-304：实验性 rust 分析器随包（--with-rust）；失败不阻断 cpu 包构建。
    if _WITH_RUST:
        bin_dir = DIST / "bin"
        bin_dir.mkdir(exist_ok=True)
        target = Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "engine-rs" / "target")))
        exe = target / "release" / "ls-analyzer.exe"
        if exe.exists():
            shutil.copy2(exe, bin_dir / "ls-analyzer.exe")
            # ort 后端运行时动态库（D:/tmp/ort-dylib 是本机布局，逐候选探测）
            for ort_dir in (ROOT / "tools" / "onnxruntime",
                            Path(os.environ.get("ORT_DYLIB_DIR", ""))):
                if ort_dir and ort_dir.is_dir():
                    for dll in ort_dir.glob("onnxruntime*.dll"):
                        shutil.copy2(dll, bin_dir / dll.name)
                    break
            deploy.log(f"  [R2-304] bin/ls-analyzer.exe 已打包（实验性；LS_ANALYZER_EXE 由启动器注入）")
        else:
            deploy.log("  [警告] --with-rust：未找到 " + str(exe) +
                       "（先 cargo build --release -p ls-analyzer），继续不含 rust 分析器")
    # 法律与配置模板
    for f in ("LICENSE", "NOTICE"):
        if (ROOT / f).exists():
            shutil.copy2(ROOT / f, DIST / f)
    shutil.copy2(ROOT / "server" / ".env.example", app_dir / ".env.example")
    (app_dir / ".env.template").write_text(
        "# 便携版配置模板（启动器复制到数据目录并把相对路径改为绝对路径）\n"
        "LS_MODEL_FP32=../../models/yolo11n.onnx\n"
        "# Q-08：实验服模型不随包，未下载时以「仅人员检测」模式运行；\n"
        "# 设置页确认许可后按需下载，端点会写入 LS_PPE_MODEL 绝对路径。\n"
        "# LS_PPE_MODEL=\n"
        "LS_PPE_CONF=0.30\n"
        "LS_WEB_DIST=../web\n"
        "LS_FILE_LOOP=true\n"
        "# 24h 挂机建议开启：服务运行期间阻止系统睡眠（不改电源计划，停止服务即恢复）\n"
        "# LS_KEEP_AWAKE=true\n"
        "# LS_KEEP_AWAKE_DISPLAY=true\n",
        encoding="utf-8")
    # 双击入口（内容纯 ASCII，中文文件名不影响 cmd 解析）
    (DIST / "scripts").mkdir(exist_ok=True)
    here = Path(__file__).resolve().parent
    shutil.copy2(here / "portable_launcher.py", DIST / "scripts" / "portable_launcher.py")
    shutil.copy2(here / "install_portable.py", DIST / "scripts" / "install_portable.py")
    # R2-402 升级健康检查拆为独立模块（install_portable.py 第 26 行同目录导入）；
    # 漏拷会让 launcher 启动即 ModuleNotFoundError（2026-11-11 冒烟步骤 3a 实证：
    # "无 logs 目录" = 进程在写日志前就崩了）——必须随包。
    shutil.copy2(here / "health_check.py", DIST / "scripts" / "health_check.py")
    # 2026-11-11 反误报治理：build 冒烟探针改为文件执行，探针亦需随脚本目录
    # 提供（发布体检/排障可复用；不进包对启动无影响，随包便于现场诊断）。
    shutil.copy2(here / "probe_runtime.py", DIST / "scripts" / "probe_runtime.py")
    shutil.copy2(here / "installer.py", DIST / "scripts" / "installer.py")  # 安装器 UI 层（任务120）
    shutil.copy2(here / "seed_demo.py", DIST / "scripts" / "seed_demo.py")  # 播种单源（任务51）
    shutil.copy2(here / "doctor.py", DIST / "scripts" / "doctor.py")  # 自检工具（任务70）
    # 任务75：运维手册让用户在包内跑备份/诊断——这两个脚本必须随包
    shutil.copy2(here / "backup_data.py", DIST / "scripts" / "backup_data.py")
    shutil.copy2(here / "collect_diagnostics.py", DIST / "scripts" / "collect_diagnostics.py")
    # C# 启动 EXE（任务55）：csc 缺失/编译失败只警告，包仍可用 启动.bat
    import build_launcher_exe
    exe = build_launcher_exe.compile_exe(DIST)
    if not exe:
        deploy.log("  [警告] 包根没有 LabSentinel.exe——双击入口为 启动.bat（安装器快捷方式回退 pythonw）")
    shutil.copy2(here / "portable_start.bat", DIST / "启动.bat")
    shutil.copy2(here / "portable_stop.bat", DIST / "停止.bat")
    (DIST / "安装.bat").write_text(
        "@echo off\n"
        "\"%~dp0runtime\\python.exe\" \"%~dp0scripts\\installer.py\" install --ui --source \"%~dp0.\" %*\n"
        "pause\n",
        encoding="ascii")
    (DIST / "app" / "VERSION").write_text(
        (ROOT / "VERSION").read_text("ascii").strip(), encoding="ascii")  # T-401


def smoke() -> None:
    deploy.log("  [5] PATH 剥离冒烟（PATH=C:\\Windows\\System32）...")
    env = {"PATH": r"C:\Windows\System32",
           "SystemRoot": os.environ.get("SystemRoot", r"C:\Windows"),
           "TEMP": os.environ.get("TEMP", str(RUNTIME)), "PYTHONUTF8": "1"}
    # 2026-11-11 反误报治理：三段 `python -c 动态代码串` 改为文件执行（探针脚本）——
    # 卡巴斯基对「python.exe -c 动态代码串」启发式敏感（2026-11-09 install_portable
    # 先例；2026-11-11 staging 删除时间线与 build 冒烟的 -c 执行完全吻合），静态
    # 二分实证证明文件集本身无特征（见 docs/guide/杀毒软件误报说明.md）。
    probe = ROOT / "scripts" / "probe_runtime.py"
    r = subprocess.run([str(RUNTIME / "python.exe"), str(probe),
                        "--app-dir", str(DIST / "app" / "server")],
                       env=env, capture_output=True, text=True, cwd=str(RUNTIME))
    deploy.log("    " + (r.stdout.strip() or r.stderr.strip()[-300:]))
    assert r.returncode == 0, "runtime 冒烟探针失败（依赖导入/opencv 单发行版/app.main）"
    deploy.log("    （_pth 隔离模式不带 cwd；运行时由 uvicorn 自行插入 cwd，探针显式 insert 模拟）")
    # config 模块级 data_dir.mkdir 会在导入时创建空数据目录——不属于构建产物，清掉（任务45 再外置）
    stray = DIST / "app" / "server" / "data"
    if stray.exists():
        shutil.rmtree(stray, ignore_errors=True)


def report() -> None:
    rows = [("runtime(python)", dir_mb(RUNTIME)), ("app(server+web)", dir_mb(DIST / "app")),
            ("models", dir_mb(DIST / "models")), ("tools", dir_mb(DIST / "tools"))]
    total = sum(v for _, v in rows)
    deploy.log("[体积] " + "  ".join(f"{k}={v:.0f}MB" for k, v in rows) + f"  合计={total:.0f}MB")


def make_zip() -> Path:
    # R2-303: two variants produce two distinct artifacts.
    suffix = f"-{_VARIANT}" if _VARIANT != "cpu" else ""
    out = (ROOT / "Resources" / "dist" /
           f"LabSentinel-portable-{date.today():%Y%m%d}{suffix}.zip")
    deploy.log(f"[打包] {out} ...")
    for pyc_dir in (DIST / "app").rglob("__pycache__"):
        # 构建期冒烟 import app.main 会在清理后于 staging 重新生成 __pycache__（任务129）
        shutil.rmtree(pyc_dir, ignore_errors=True)
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as z:
        for f in DIST.rglob("*"):
            if f.is_file():
                z.write(f, Path("LabSentinel") / f.relative_to(DIST))
    deploy.log(f"[打包] 完成 {out.stat().st_size / 1e6:.0f}MB")
    return out


def find_iscc() -> Path | None:
    """ADR-0010：定位 Inno Setup 命令行编译器（ISCC.exe）。

    顺序：ISCC_PATH 环境变量 → 常见安装路径（Inno Setup 5/6/7，x86/x64 与各盘符）→ PATH。
    本机实测装在 `D:\\Program Files\\Inno Setup 7\\ISCC.exe`（IS7），故版本号与盘符都
    需覆盖；CI 由 choco 装 6。未装时返回 None，--installer 仅警告跳过（zip-only 仍成功）。
    """
    import shutil
    override = os.environ.get("ISCC_PATH")
    if override and Path(override).is_file():
        return Path(override)
    bases = [os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)"),
             os.environ.get("ProgramFiles", r"C:\Program Files"),
             r"D:\Program Files", r"C:\Program Files (x86)"]
    for base in bases:
        for ver in ("7", "6", "5"):
            p = Path(base) / f"Inno Setup {ver}" / "ISCC.exe"
            if p.is_file():
                return p
    found = shutil.which("ISCC.exe")
    return Path(found) if found else None


def make_installer() -> Path | None:
    """用 deploy/windows/installer.iss 从同一 staging 产出 setup.exe。"""
    iss = ROOT / "deploy" / "windows" / "installer.iss"
    if not iss.is_file():
        deploy.log(f"[警告] 安装器脚本缺失: {iss}")
        return None
    iscc = find_iscc()
    if iscc is None:
        deploy.log("[警告] 未找到 ISCC.exe（Inno Setup 6）——跳过 setup.exe 编译，"
                   "zip 便携形态照常产出；CI 由 choco install innosetup 提供")
        return None
    version = (ROOT / "VERSION").read_text("ascii").strip()
    out = ROOT / "Resources" / "dist" / f"LabSentinel-setup-{version}.exe"
    deploy.log(f"[安装器] ISCC 编译 → {out.name} (iscc={iscc})")
    r = subprocess.run(
        [str(iscc), "/Q", f"/DLsAppVersion={version}",
         f"/DLsSourceDir={DIST}", f"/DLsOutputDir={out.parent}", str(iss)],
        capture_output=True, text=True, encoding="mbcs", errors="replace")
    if r.returncode != 0 or not out.is_file():
        deploy.log(f"[警告] ISCC 编译失败（exit {r.returncode}）："
                   f"{(r.stdout or '').strip()[-300:]}")
        return None
    deploy.log(f"[安装器] 完成 {out.stat().st_size / 1e6:.1f}MB")
    return out


def main() -> int:
    global _PYTHON_OVERRIDE, _VARIANT, _WITH_RUST
    ap = argparse.ArgumentParser(description="便携运行时构建器（任务44）")
    ap.add_argument("--zip", action="store_true", help="构建后打包 zip")
    ap.add_argument("--smoke-only", action="store_true", help="只重跑冒烟（已构建时）")
    ap.add_argument("--python", type=Path, default=None,
                    help="构建用 Python 解释器（CI 传 sys.executable；本机默认 .venv）")
    ap.add_argument("--models-dir", type=Path, default=None,
                    help="模型源目录（默认 <仓库>/m0-spike；CI 下载模型后指向该目录）")
    ap.add_argument("--variant", choices=["cpu", "dml"], default="cpu",
                    help="R2-303：推理变体（cpu=统一锁 / dml=DirectML 可选包）")
    ap.add_argument("--with-rust", action="store_true",
                    help="R2-304：把 ls-analyzer 打进 bin/（实验产物）")
    ap.add_argument("--installer", action="store_true",
                    help="ADR-0010：用 Inno Setup (ISCC.exe) 编译 setup.exe（zip 双形态；未装 ISCC 则警告跳过）")
    a = ap.parse_args()
    _PYTHON_OVERRIDE = a.python
    _VARIANT = a.variant
    _WITH_RUST = a.with_rust
    if a.models_dir is not None:
        global _MODELS_DIR
        _MODELS_DIR = a.models_dir
    deploy.log("[便携构建] 目标 " + str(DIST))
    DIST.mkdir(parents=True, exist_ok=True)
    if a.smoke_only:  # 2026-10-30 审阅：参数原先从未被使用（定义后死参）
        smoke()
        report()
        deploy.log("[便携构建] --smoke-only 完成")
        return 0
    ensure_runtime()
    ensure_deps()
    copy_assets()
    smoke()
    report()
    if a.zip:
        make_zip()
    if a.installer:
        make_installer()
    deploy.log("[便携构建] 完成")
    return 0


if __name__ == "__main__":
    sys.exit(main())
