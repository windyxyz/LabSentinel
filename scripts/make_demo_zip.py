"""打 LabSentinel demo 分发包。

用法: python scripts/make_demo_zip.py
产物: dist/LabSentinel-demo-<日期>.zip（新电脑解压后双击 一键启动.bat 即可）

包含: 源码/前端/模型(ONNX+classes)/演示视频/go2rtc/文档/启动与部署脚本/效果演示视频
排除: 数据集、训练产物、venv、node_modules、ffmpeg、Node、数据库与事件产物、日志、git
"""
import zipfile
from pathlib import Path
from datetime import date

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "dist" / f"LabSentinel-demo-{date.today():%Y%m%d}.zip"

INCLUDE_DIRS = ["server", "web/src", "web/public", "web/mock", "docs", "tools", "scripts", "效果演示"]
INCLUDE_FILES = [
    "一键启动.bat", "快速开始.md", "README.md", "GLOSSARY.md", "LICENSE", "NOTICE",
    "docker-compose.yml", "web/package.json", "web/package-lock.json",
    "web/index.html", "web/vite.config.js", "web/Dockerfile",
    "server/.env.example", "server/requirements.txt", "server/Dockerfile",
    "m0-spike/yolo11n_ppe.onnx", "m0-spike/yolo11n_ppe.classes.txt",
    "m0-spike/yolo11n_ppe.pt",
    "m0-spike/yolo11n.onnx",  # 主 Engine（人员检测兜底）默认模型，缺了 /api/system 500
    "m0-spike/data/lab-walk-24076.mp4", "m0-spike/data/lab-equip-21453.mp4",
    "tools/go2rtc.exe", "tools/go2rtc.yaml", "tools/README.md",
]
# tools/ 只带走脚本与 go2rtc，ffmpeg/Node 等大件由 deploy.py 在新机器上按需下载
TOOLS_SKIP_PREFIXES = ("tools/ffmpeg", "tools/node", "tools/__pycache__",
                       "tools/out_", "tools/datasets")
# server/data（数据库/事件截图/视频/标注池）是运行时数据，新机器由 deploy.py 播种
SERVER_SKIP_PREFIXES = ("server/data", "server/__pycache__")
SERVER_SKIP_FILES = {"server/.env"}  # 含机器特定路径，由 deploy.py 在目标机生成
# scripts 里排除运行时生成物
SCRIPTS_SKIP = {"scripts/venv_path.txt", "scripts/npu-venv-freeze.txt"}

def want(rel: Path) -> bool:
    posix = rel.as_posix()
    if posix in SCRIPTS_SKIP or posix in SERVER_SKIP_FILES:
        return False
    for p in TOOLS_SKIP_PREFIXES + SERVER_SKIP_PREFIXES:
        if posix.startswith(p):
            return False
    if "__pycache__" in posix or posix.endswith((".pyc", ".db", ".log")):
        return False
    return True


def main() -> None:
    OUT.parent.mkdir(exist_ok=True)
    files: list[Path] = []
    for d in INCLUDE_DIRS:
        base = ROOT / d
        if not base.exists():
            continue
        for f in base.rglob("*"):
            if f.is_file() and want(f.relative_to(ROOT)):
                files.append(f)
    for f in INCLUDE_FILES:
        p = ROOT / f
        if p.exists():
            files.append(p)
        else:
            print(f"[警告] 清单文件缺失: {f}")
    # 去重
    seen, uniq = set(), []
    for f in files:
        r = f.relative_to(ROOT)
        if r not in seen:
            seen.add(r)
            uniq.append(f)

    total = sum(f.stat().st_size for f in uniq)
    print(f"打包 {len(uniq)} 个文件，原始 {total/1e6:.1f} MB -> {OUT.name}")
    with zipfile.ZipFile(OUT, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for f in uniq:
            z.write(f, Path("LabSentinel") / f.relative_to(ROOT))
    print(f"完成: {OUT}  ({OUT.stat().st_size/1e6:.1f} MB)")


if __name__ == "__main__":
    main()
