"""发布产物清单与校验和（任务58）：对便携包 zip 生成机器可校验的 manifest。

用法（标准库）:
    python scripts/release_manifest.py                      # 最新 dist/LabSentinel-portable-*.zip → dist/SHA256SUMS.txt + RELEASE-MANIFEST.json
    python scripts/release_manifest.py --zip <path> --out <dir>
    python scripts/release_manifest.py --verify <manifest|dir>   # 重算 sha256/大小对比，失败 exit 2

必需条目按 build_portable.py 实际布局写死（允许一层 LabSentinel/ 前缀）。
注意：卸载.bat 是安装器生成的，zip 内没有——列在可选；LabSentinel.exe 无 csc 构建会降级——可选。
"""
import argparse
import hashlib
import json
import re
import subprocess
import sys
import zipfile
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent

# 归一化（剥 LabSentinel/ 前缀后）的必需条目——对照 build_portable.copy_assets 实际产物。
# 2026-11-11 校正（本地实机构建 + release_check 发现）：REQUIRED 未跟随政策变更——
#   T-406/Q-07：Mixkit 演示视频不再随包（版权风险，media/ 只留 SOURCES.md）；
#   Q-08：yolo11n_ppe.* 不随包（许可确认后按需下载）。
REQUIRED = [
    "runtime/python.exe",
    "runtime/pythonw.exe",
    "app/server/app/main.py",
    "app/server/.env.template",
    "app/VERSION",
    "app/web/index.html",
    "scripts/portable_launcher.py",
    "scripts/install_portable.py",
    "scripts/seed_demo.py",
    "启动.bat",
    "停止.bat",
    "安装.bat",
    "models/yolo11n.onnx",
]
# 缺失只警告：LabSentinel.exe（无 csc 降级）、卸载.bat（安装器生成）、ffmpeg/go2rtc
# （二进制组件被 .gitignore，CI 干净 checkout 需另行获取，2026-11-13 CI 实测）——
# bin/ls-analyzer.exe + bin/onnxruntime.dll（R2-304 --with-rust 实验产物）、media/SOURCES.md
OPTIONAL = ["LabSentinel.exe", "卸载.bat", "tools/ffmpeg.exe", "tools/go2rtc.exe",
            "bin/ls-analyzer.exe", "bin/onnxruntime.dll", "media/SOURCES.md"]

EXIT_OK, EXIT_FAIL = 0, 2


def log(m=""):
    print(m, flush=True)


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def find_latest_zip(dist: Path) -> Path | None:
    zips = sorted(dist.glob("LabSentinel-portable-*.zip"), key=lambda p: p.stat().st_mtime)
    return zips[-1] if zips else None


def zip_version(zf: zipfile.ZipFile, orig: dict[str, str]) -> str:
    """优先包内 app/VERSION；无则从 api.py 抓 x.y.z-tag；再退 0.1.0-alpha。orig=归一化→包内原名。"""
    for cand in ("app/VERSION", "LabSentinel/app/VERSION"):
        real = orig.get(cand)
        if real:
            v = zf.read(real).decode("ascii", "replace").strip()
            if v:
                return v
    api = ROOT / "server" / "app" / "api.py"
    if api.exists():
        m = re.search(r'"(\d+\.\d+\.\d+-[A-Za-z0-9]+)"', api.read_text(encoding="utf-8"))
        if m:
            return m.group(1)
    return "0.1.0-alpha"


def git_info() -> tuple[str | None, bool]:
    def run(*args):
        try:
            r = subprocess.run(["git", *args], cwd=str(ROOT), capture_output=True, text=True, timeout=15)
            return r.stdout.strip() if r.returncode == 0 else None
        except (OSError, subprocess.TimeoutExpired):
            return None

    commit = run("rev-parse", "--short", "HEAD")
    dirty = bool(run("status", "--porcelain"))
    return commit, dirty


def analyze_zip(zip_path: Path) -> dict:
    """读 zip 目录（不解压内容，VERSION 除外），返回必需/可选缺失与元数据。"""
    with zipfile.ZipFile(zip_path) as zf:
        infos = [i for i in zf.infolist() if not i.is_dir()]
        names: set[str] = set()
        orig: dict[str, str] = {}
        for i in infos:
            n = i.filename.replace("\\", "/")
            parts = n.split("/", 1)
            norm = parts[1] if len(parts) == 2 and parts[0] == "LabSentinel" else n
            names.add(norm)
            orig.setdefault(norm, n)
        version = zip_version(zf, orig)
    missing_required = [e for e in REQUIRED if e not in names]
    missing_optional = [e for e in OPTIONAL if e not in names]
    return {"version": version, "zip_entry_count": len(infos),
            "missing_required": missing_required, "missing_optional": missing_optional,
            "entries_total": len(names)}


def build_manifest(zip_path: Path, out_dir: Path) -> int:
    if not zip_path.exists():
        log(f"[失败] zip 不存在: {zip_path}")
        return EXIT_FAIL
    info = analyze_zip(zip_path)
    size = zip_path.stat().st_size
    sha = sha256_file(zip_path)
    commit, dirty = git_info()
    manifest = {
        "version": info["version"],
        "zip": zip_path.name,
        "size_bytes": size,
        "sha256": sha,
        "entries": info["entries_total"],
        "zip_entry_count": info["zip_entry_count"],
        "missing_required": info["missing_required"],
        "missing_optional": info["missing_optional"],
        "git_commit": commit,
        "git_dirty": dirty,
        "generated_at": datetime.now(timezone.utc).astimezone().isoformat(timespec="seconds"),
    }
    out_dir.mkdir(parents=True, exist_ok=True)
    sums = out_dir / "SHA256SUMS.txt"
    sums.write_text(f"{sha}  {zip_path.name}\n", encoding="ascii")
    mf = out_dir / "RELEASE-MANIFEST.json"
    mf.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    log(f"[manifest] {zip_path.name}")
    log(f"  version={manifest['version']} size={size / 1e6:.1f}MB entries={manifest['entries']}"
        f" zip_entries={manifest['zip_entry_count']}")
    log(f"  sha256={sha}")
    log(f"  git={commit} dirty={dirty}")
    if info["missing_required"]:
        log(f"  [失败] 缺必需条目 {len(info['missing_required'])}: {info['missing_required']}")
        rc = EXIT_FAIL
    else:
        log("  [OK] 必需条目齐全")
        rc = EXIT_OK
    if info["missing_optional"]:
        log(f"  [警告] 可选条目缺失 {len(info['missing_optional'])}: {info['missing_optional']}")
    log(f"  输出: {sums}")
    log(f"        {mf}")
    return rc


def verify(target: Path) -> int:
    mf = target if target.is_file() else target / "RELEASE-MANIFEST.json"
    if not mf.exists():
        log(f"[失败] 找不到 manifest: {mf}")
        return EXIT_FAIL
    m = json.loads(mf.read_text(encoding="utf-8"))
    dist = mf.parent
    zip_path = dist / m["zip"]
    if not zip_path.exists():
        log(f"[失败] zip 不存在: {zip_path}")
        return EXIT_FAIL
    sha = sha256_file(zip_path)
    size = zip_path.stat().st_size
    if sha != m["sha256"] or size != m["size_bytes"]:
        log(f"[失败] 校验不一致: sha256 {sha == m['sha256']}, size {size == m['size_bytes']}")
        return EXIT_FAIL
    log(f"OK {zip_path.name} sha256/size 与 manifest 一致")
    return EXIT_OK


def main() -> int:
    ap = argparse.ArgumentParser(description="发布产物清单与校验和（任务58）")
    ap.add_argument("--zip", type=Path, help="指定 zip（默认取 Resources/dist/ 最新便携包）")
    ap.add_argument("--out", type=Path, default=ROOT / "Resources" / "dist", help="输出目录（默认 Resources/dist/）")
    ap.add_argument("--verify", type=Path, help="校验模式：manifest 文件或其所在目录")
    a = ap.parse_args()
    if a.verify:
        return verify(a.verify)
    zp = a.zip or find_latest_zip(ROOT / "Resources" / "dist")
    if not zp:
        log("[失败] Resources/dist/ 下没有 LabSentinel-portable-*.zip")
        return EXIT_FAIL
    return build_manifest(zp, a.out)


if __name__ == "__main__":
    sys.exit(main())
