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
DIST = ROOT / "dist" / "portable" / "LabSentinel"
RUNTIME = DIST / "runtime"
SITE = RUNTIME / "Lib" / "site-packages"
PY_VER = "3.12.8"
PY_ZIP = f"python-{PY_VER}-embed-amd64.zip"
PY_SOURCES = [
    f"https://registry.npmmirror.com/-/binary/python/{PY_VER}/{PY_ZIP}",
    f"https://mirrors.huaweicloud.com/python/{PY_VER}/{PY_ZIP}",
    f"https://www.python.org/ftp/python/{PY_VER}/{PY_ZIP}",
]
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
    zpath = ROOT / "logs" / PY_ZIP
    if not zpath.exists():
        deploy.log(f"  [1] 下载 CPython {PY_VER} embed（npmmirror→华为云→python.org）...")
        for url in PY_SOURCES:
            try:
                deploy.http_download(url, zpath, timeout=600)
                break
            except Exception as e:
                deploy.log(f"    源失败，换下一个: {e}")
        else:
            raise SystemExit("[失败] 三个下载源均失败")
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
    # 服务端代码（排除 data/__pycache__）
    app_dir = DIST / "app" / "server"
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
    # 二进制
    tools = DIST / "tools"
    tools.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT / "tools" / "go2rtc.exe", tools / "go2rtc.exe")
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
        "LS_FILE_LOOP=true\n",
        encoding="utf-8")
    # 双击入口（内容纯 ASCII，中文文件名不影响 cmd 解析）
    (DIST / "scripts").mkdir(exist_ok=True)
    here = Path(__file__).resolve().parent
    shutil.copy2(here / "portable_launcher.py", DIST / "scripts" / "portable_launcher.py")
    shutil.copy2(here / "install_portable.py", DIST / "scripts" / "install_portable.py")
    shutil.copy2(here / "portable_start.bat", DIST / "启动.bat")
    shutil.copy2(here / "portable_stop.bat", DIST / "停止.bat")
    (DIST / "安装.bat").write_text(
        "@echo off\n"
        "\"%~dp0runtime\\python.exe\" \"%~dp0scripts\\install_portable.py\" install --source \"%~dp0.\" %*\n"
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
    out = ROOT / "dist" / f"LabSentinel-portable-{date.today():%Y%m%d}.zip"
    deploy.log(f"[打包] {out} ...")
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
