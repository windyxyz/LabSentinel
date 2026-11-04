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
import os
import shutil
import subprocess
import sys
import zipfile
from datetime import date
from pathlib import Path

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
MODELS = ["yolo11n.onnx", "yolo11n_ppe.onnx", "yolo11n_ppe.classes.txt"]


def dir_mb(p: Path) -> float:
    return sum(f.stat().st_size for f in p.rglob("*") if f.is_file()) / 1e6 if p.exists() else 0.0


def venv_python() -> Path:
    v = deploy._read_venv_mark() or (ROOT / ".venv" / "Scripts" / "python.exe")
    if not Path(v).exists():
        raise SystemExit(f"[失败] 找不到构建用 venv python（{v}）；先跑 deploy.py install")
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
    if (SITE / "fastapi").exists() and (SITE / "cv2").exists():
        deploy.log("  [跳过] site-packages 已预装（fastapi/cv2 就位）")
        return
    py = venv_python()
    SITE.mkdir(parents=True, exist_ok=True)
    deploy.log("  [3] 预装依赖到 runtime/Lib/site-packages（清华→阿里云→官方，约 5 分钟）...")
    for idx in PIP_INDEXES:
        r = subprocess.run(
            [str(py), "-m", "pip", "install", "--disable-pip-version-check",
             "--no-warn-script-location", "--target", str(SITE),
             "-r", str(ROOT / "server" / "requirements.txt"), "-i", idx],
            env={**os.environ, "PYTHONUTF8": "1"})
        if r.returncode == 0:
            deploy.log(f"    来源: {idx}")
            return
        deploy.log(f"    源失败（exit {r.returncode}），换下一个")
    raise SystemExit("[失败] pip 三个源均失败")


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
    for name in MODELS:
        src = ROOT / "m0-spike" / name
        assert src.exists(), f"缺模型文件 {src}"
        shutil.copy2(src, DIST / "models" / name)
    # 演示视频（任务47：首次运行播种 4 路演示摄像头用）
    (DIST / "media").mkdir(parents=True, exist_ok=True)
    for fn in ("lab-walk-24076.mp4", "lab-equip-21453.mp4"):
        src = ROOT / "m0-spike" / "data" / fn
        assert src.exists() and src.stat().st_size > 1_000_000, f"缺演示视频 {src}"
        shutil.copy2(src, DIST / "media" / fn)
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
    # 法律与配置模板
    for f in ("LICENSE", "NOTICE"):
        if (ROOT / f).exists():
            shutil.copy2(ROOT / f, DIST / f)
    shutil.copy2(ROOT / "server" / ".env.example", app_dir / ".env.example")
    (app_dir / ".env.template").write_text(
        "# 便携版配置模板（启动器复制到数据目录并把相对路径改为绝对路径）\n"
        "LS_MODEL_FP32=../../models/yolo11n.onnx\n"
        "LS_PPE_MODEL=../../models/yolo11n_ppe.onnx\n"
        "LS_PPE_CLASS_NAMES=lab coat\n"
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
    (DIST / "app" / "VERSION").write_text("2026.10", encoding="ascii")


def smoke() -> None:
    deploy.log("  [5] PATH 剥离冒烟（PATH=C:\\Windows\\System32）...")
    env = {"PATH": r"C:\Windows\System32",
           "SystemRoot": os.environ.get("SystemRoot", r"C:\Windows"),
           "TEMP": os.environ.get("TEMP", str(RUNTIME)), "PYTHONUTF8": "1"}
    code = ("import fastapi,uvicorn,cv2,numpy,supervision,onnxruntime;"
            "print('providers=', onnxruntime.get_available_providers())")
    r = subprocess.run([str(RUNTIME / "python.exe"), "-c", code], env=env,
                       capture_output=True, text=True, cwd=str(RUNTIME))
    deploy.log("    " + (r.stdout.strip() or r.stderr.strip()[-300:]))
    assert r.returncode == 0, "依赖导入冒烟失败"
    r = subprocess.run(
        [str(RUNTIME / "python.exe"), "-c",
         "import sys; sys.path.insert(0, '.'); import app.main; print('app.main import OK')"],
        env=env, capture_output=True, text=True, cwd=str(DIST / "app" / "server"))
    deploy.log("    " + (r.stdout.strip() or r.stderr.strip()[-300:]))
    assert r.returncode == 0, "app.main 导入失败"
    deploy.log("    （_pth 隔离模式不带 cwd；运行时由 uvicorn 自行插入 cwd，故冒烟显式 insert 模拟）")
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
    out = ROOT / "Resources" / "dist" / f"LabSentinel-portable-{date.today():%Y%m%d}.zip"
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


def main() -> int:
    ap = argparse.ArgumentParser(description="便携运行时构建器（任务44）")
    ap.add_argument("--zip", action="store_true", help="构建后打包 zip")
    ap.add_argument("--smoke-only", action="store_true", help="只重跑冒烟（已构建时）")
    a = ap.parse_args()
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
    deploy.log("[便携构建] 完成")
    return 0


if __name__ == "__main__":
    sys.exit(main())
