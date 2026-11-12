"""LabSentinel 一键部署引擎（仅标准库，无 venv 也能跑）。

用法（一般由 一键启动.bat 调用，也可手动执行）:
  python scripts/deploy.py check              环境体检：缺什么列什么，全绿退出码 0
  python scripts/deploy.py install            自动部署：venv/pip/openh264/node/npm/ffmpeg/.env/演示数据
  python scripts/deploy.py redeploy --keep-data   重装环境（venv+依赖+配置），保留 server/data 数据
  python scripts/deploy.py redeploy --wipe        重装环境并清空数据（重建演示库）
  python scripts/deploy.py uninstall          停服务并删除部署生成物（数据目录改名备份，不直接删）

约定:
  - venv 优先级: scripts/venv_path.txt → 项目内 .venv → D:\\LabSentinel\\m0-venv(兼容旧装)
  - venv_path.txt 记录 venv python 绝对路径, start-all.bat 从这里读
  - 下载源仅限公网 http/https（(_assert_public_host 校验），npm/pip 用国内镜像
  - 种子库 schema 与 server/app/store.py 的 SCHEMA 保持同步
"""
from __future__ import (
    annotations,  # 允许老解释器（3.9）先跑起来做体检，venv 仍会挑 3.10+
)

import argparse
import gzip
import hashlib
import ipaddress
import json
import os
import shutil
import socket
import sqlite3
import subprocess
import sys
import tarfile
import time
import urllib.request
import zipfile
from pathlib import Path
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parent.parent
VENV_MARK = ROOT / "scripts" / "venv_path.txt"
NODE_DIR = ROOT / "tools" / "node"
FFMPEG_DIR = ROOT / "tools" / "ffmpeg"
PIPI = "-i", "https://pypi.tuna.tsinghua.edu.cn/simple"
NPMR = "--registry=https://registry.npmmirror.com"

# 播种单源（任务51）：schema/演示摄像头/区域/规则统一在 scripts/seed_demo.py 维护
import seed_demo  # noqa: E402

# 演示视频文件名（用于已有库的失效路径修正，按名称匹配）
SEED_VIDEO_FILES = {
    "lab-slice": "lab-walk-24076.mp4",
    "ppe-regress": "lab-walk-24076.mp4",
    "ppe-final": "lab-walk-24076.mp4",
    "ppe-equip": "lab-equip-21453.mp4",
}
PORTS = (1984, 8000, 5173)


def log(msg="", **kw):
    print(msg, flush=True, **kw)


# ---------- 安全：仅公网 http/https ----------
def _assert_public_host(url: str) -> None:
    """请求前校验：仅 http/https，且 host 不得是环回/私有/保留地址。"""
    from urllib.parse import urlparse
    p = urlparse(url)
    if p.scheme not in ("http", "https"):
        raise ValueError(f"仅允许 http/https: {url}")
    host = p.hostname or ""
    try:
        infos = socket.getaddrinfo(host, p.port or (443 if p.scheme == "https" else 80))
    except OSError as e:
        raise ValueError(f"域名解析失败 {host}: {e}")
    for info in infos:
        ip = ipaddress.ip_address(info[4][0])
        if (ip.is_loopback or ip.is_private or ip.is_reserved or ip.is_link_local
                or ip.is_multicast or ip.is_unspecified):
            raise ValueError(f"拒绝非公网地址 {host} -> {ip}")


UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/126.0 Safari/537.36 LabSentinel-Deploy/1.0")


def http_get(url: str, timeout: int = 60) -> bytes:
    _assert_public_host(url)
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.read()


def http_download(url: str, dest: Path, timeout: int = 120) -> Path:
    _assert_public_host(url)
    dest.parent.mkdir(parents=True, exist_ok=True)
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=timeout) as r, dest.open("wb") as f:
        total = int(r.headers.get("Content-Length") or 0)
        got = 0
        while True:
            chunk = r.read(1 << 20)
            if not chunk:
                break
            f.write(chunk)
            got += len(chunk)
            if total:
                log(f"\r    下载 {got / 1e6:.1f}/{total / 1e6:.1f} MB", end="")
    log()
    return dest


# ---------- T-005：资产哈希锁定（scripts/assets.lock.json） ----------
ASSETS_LOCK = ROOT / "scripts" / "assets.lock.json"


def verify_sha256(path: Path, expected: str) -> None:
    """下载后强制 SHA256 校验；不匹配即删除残件并抛错（防篡改/防损坏扩散）。"""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    got = h.hexdigest()
    if got != (expected or "").lower():
        path.unlink(missing_ok=True)
        raise ValueError(f"SHA256 不匹配: {path.name}（got={got[:16]}… want={expected[:16]}…）")


def _assets_lock() -> dict:
    if not ASSETS_LOCK.exists():
        raise SystemExit(f"[失败] 缺少资产哈希清单 {ASSETS_LOCK}（T-005）")
    return json.loads(ASSETS_LOCK.read_text("utf-8"))


def download_verified(asset_key: str, dest: Path) -> Path:
    """按 assets.lock.json 下载指定资产：官方源优先，逐源下载后强制 SHA256 校验，
    失败/不匹配删残件换下一源。LS_ALLOW_MIRROR_PROXY=1 才启用加速代理源（默认关闭），
    哈希匹配可证明内容一致，但代理默认不出网。"""
    entry = _assets_lock()["assets"][asset_key]
    sources = list(entry.get("urls", []))
    if os.environ.get("LS_ALLOW_MIRROR_PROXY") == "1":
        sources += list(entry.get("mirror_urls", []))
    expected = entry["sha256"]
    last_err: Exception | None = None
    for url in sources:
        try:
            http_download(url, dest, timeout=900)
            verify_sha256(dest, expected)
            log(f"    来源: {urlparse(url).netloc}（SHA256 已校验）")
            return dest
        except SystemExit:
            raise
        except Exception as e:  # noqa: BLE001
            last_err = e
            dest.unlink(missing_ok=True)
            log(f"    源失败（残件已删除）: {e}")
    raise SystemExit(f"[失败] 资产 {asset_key} 全部源失败或哈希不匹配: {last_err}")


# ---------- python / venv ----------
def find_system_python() -> str | None:
    """优先 3.12/3.11/3.10（onnxruntime-directml wheel 覆盖好），其次任意 python。"""
    for cand in ("py -3.12", "py -3.11", "py -3.10", "py -3", "python", "python3"):
        try:
            out = subprocess.run(cand.split() + ["--version"], capture_output=True,
                                 text=True, timeout=30)
            ver = (out.stdout or out.stderr).strip()
            if out.returncode == 0 and ver.startswith("Python 3"):
                minor = ver.split(".")[1]
                # venv 最低 3.10：onnxruntime-directml 已不再为 3.8/3.9 提供新 wheel
                if minor.isdigit() and int(minor) >= 10:
                    return cand
        except (OSError, subprocess.TimeoutExpired):
            continue
    return None


def venv_python(venv: Path) -> Path:
    return venv / "Scripts" / "python.exe"


def _read_venv_mark() -> Path | None:
    """venv_path.txt 用 GBK 写（cmd for /f 按系统代码页读，UTF-8 中文路径会乱码）。

    项目内 venv 写成相对路径（.venv\\Scripts\\python.exe，纯 ASCII），
    读回时相对 ROOT 解析——这样非中文代码页的机器也不会读坏路径。
    """
    try:
        raw = VENV_MARK.read_text(encoding="gbk").strip()
    except (OSError, UnicodeDecodeError):
        return None
    if not raw:
        return None
    p = Path(raw)
    return p if p.is_absolute() else (ROOT / p)


def _venv_runnable(p: Path) -> bool:
    """T-008：venv 可运行性验证（存在 ≠ 可用：pyvenv.cfg 指向不存在的宿主 Python 时，
    python.exe 启动即报 "No Python at ..."）。失败按"未找到"处理，触发重建。"""
    try:
        # 2026-11-11 反误报治理：`-c "import sys"` 动态代码串 → `-V`（等价的可运行性验证）
        r = subprocess.run([str(venv_python(p)), "-V"],
                           capture_output=True, timeout=30)
        return r.returncode == 0
    except (OSError, subprocess.TimeoutExpired):
        return False


def find_venv() -> Path | None:
    for p in (_read_venv_mark(), ROOT / ".venv"):
        if p and venv_python(p).exists() and _venv_runnable(p):
            return p
    legacy = Path(r"D:\LabSentinel\m0-venv")  # 旧装兼容（与文件头注释的优先级一致）
    if venv_python(legacy).exists():
        return legacy
    return None


def ensure_venv() -> Path:
    v = find_venv()
    if v:
        log(f"  [跳过] venv 已存在: {v}")
        return v
    broken = ROOT / ".venv"
    if venv_python(broken).exists() and not _venv_runnable(broken):
        log("  [自愈] .venv 已损坏（宿主 Python 失联），删除后重建...")
        shutil.rmtree(broken, ignore_errors=True)
    py = find_system_python()
    if not py:
        log("  [失败] 未找到 Python（需 3.10+），请先安装：")
        log("         https://www.python.org/downloads/  安装时勾选 Add to PATH")
        raise SystemExit(1)
    v = ROOT / ".venv"
    log(f"  [1] 创建虚拟环境 {v}（用 {py}，首次约 1 分钟）...")
    subprocess.run(py.split() + ["-m", "venv", str(v)], check=True)
    return v


def write_venv_path(venv: Path) -> None:
    """项目内 venv 写相对路径（纯 ASCII），避免 cmd 按 OEM 代码页读中文绝对路径时乱码。"""
    VENV_MARK.parent.mkdir(exist_ok=True)
    target = venv_python(venv)
    try:
        text = str(target.relative_to(ROOT))
    except ValueError:  # 项目外 venv（旧装）只能写绝对路径
        text = str(target)
    VENV_MARK.write_text(text, encoding="gbk")


def _platform_lock() -> str:
    """R2-002: source deployment must install the same hashed lock as the portable build."""
    return "requirements-win.lock" if sys.platform == "win32" else "requirements-linux.lock"


def pip_install(venv: Path) -> None:
    lock = _platform_lock()
    log(f"  [2] 安装服务端依赖（{lock}：清华 → 阿里云 → 官方，已装自动跳过，首次 2-5 分钟）...")
    env = {**os.environ, "PYTHONUTF8": "1"}  # pip 按系统代码页解码 requirements，中文注释在 GBK 机会炸
    indexes = (
        ("清华镜像", "https://pypi.tuna.tsinghua.edu.cn/simple"),
        ("阿里云镜像", "https://mirrors.aliyun.com/pypi/simple/"),
        ("官方 PyPI", None),
    )
    for label, index_url in indexes:
        run_env = dict(env)
        if index_url:
            run_env["PIP_INDEX_URL"] = index_url
        else:
            run_env.pop("PIP_INDEX_URL", None)
        r = subprocess.run([str(venv_python(venv)), "-m", "pip", "install",
                            "--disable-pip-version-check",
                            "--require-hashes", "--no-deps",
                            "-r", str(ROOT / "server" / lock)],
                           check=False, env=run_env)
        if r.returncode == 0:
            if label != "清华镜像":
                log(f"    依赖安装完成（来源：{label}）")
            return
        log(f"    {label} 安装失败（退出码 {r.returncode}），切换下一个源...")
    raise SystemExit(f"  [失败] {lock} 依赖安装失败：三个源均不可用")


def _extract_openh264(payload: Path, dest: Path) -> None:
    """Cisco 官方发布的是 .bz2（裸 .dll 直链返回 403）；按文件头判断是否需要解压。"""
    raw = payload.read_bytes()
    if raw[:3] == b"BZh":
        import bz2
        raw = bz2.decompress(raw)
    if raw[:2] != b"MZ":  # PE 文件头，防止把 404/HTML 页面当 DLL 写进去
        raise ValueError("下载内容不是有效的 DLL")
    dest.write_bytes(raw)


def _find_local_openh264() -> Path | None:
    """先扫本机已有环境（venv/系统 Python/conda），找到就直接复用，避免联网下载。"""
    roots = [ROOT / ".venv", Path(r"D:\LabSentinel\m0-venv"),
             Path(os.environ.get("LOCALAPPDATA", "")) / "Programs" / "Python",
             Path(os.environ.get("APPDATA", "")) / "Python",
             Path(os.environ.get("USERPROFILE", "")) / "anaconda3",
             Path(os.environ.get("USERPROFILE", "")) / "miniconda3"]
    for env in ("ProgramFiles", "ProgramFiles(x86)"):
        pf = Path(os.environ.get(env, ""))
        if str(pf) and pf.exists():
            roots += [d for d in pf.glob("Python*") if d.is_dir()]
    for root in roots:
        if not str(root) or not root.exists():
            continue
        try:
            for p in root.glob("**/site-packages/cv2/openh264-*.dll"):
                return p
        except OSError:
            continue
    return None


def ensure_openh264(venv: Path) -> None:
    """OpenCV 写 H.264 事件视频的可选组件：拿不到不阻塞部署（回退系统 H.264 / mp4v）。"""
    cv2_dir = venv / "Lib" / "site-packages" / "cv2"
    dll = cv2_dir / "openh264-2.5.0-win64.dll"
    if dll.exists():
        log("  [跳过] OpenH264 DLL 已就位")
        return
    local = _find_local_openh264()
    if local:
        cv2_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(local, dll)
        log(f"  [3] 复用本机已有 OpenH264 DLL: {local}")
        return
    log("  [3] 本机未找到 OpenH264，尝试下载（哈希锁定 scripts/assets.lock.json，官方源优先）...")
    payload = ROOT / "Resources" / "logs" / "openh264.download"
    try:
        download_verified("openh264-2.5.0-win64.dll.bz2", payload)
        cv2_dir.mkdir(parents=True, exist_ok=True)
        _extract_openh264(payload, dll)
        log("    OpenH264 就绪（SHA256 已校验）")
    except (SystemExit, Exception) as e:  # noqa: BLE001 可选组件，失败不阻断部署
        log(f"  [警告] OpenH264 未获取（可选）: {e}")
        log("         Windows 先尝试系统 H.264（MSMF），失败才回退 mp4v，不影响分析与截图")
    finally:
        payload.unlink(missing_ok=True)


# ---------- node / npm ----------
def _node_ok() -> bool:
    try:
        return subprocess.run(["node", "--version"], capture_output=True, timeout=30).returncode == 0
    except OSError:
        return False


def _npm_cmd() -> list[str]:
    """返回 npm 可执行文件的绝对路径（CreateProcess 不解析 .cmd 后缀，必须指全）。"""
    for cand in (shutil.which("npm.cmd"), shutil.which("npm"),
                 NODE_DIR / "npm.cmd", NODE_DIR / "npm"):
        if cand and Path(cand).exists():
            return [str(cand)]
    raise SystemExit("  [失败] npm 不可用（Node.js 未安装且便携版未下载成功）")


def _safe_extract_zip(z: zipfile.ZipFile, dest: Path) -> None:
    """解压前校验成员路径都在 dest 内（防 zip-slip），再交给 extractall。"""
    dest = dest.resolve()
    for info in z.infolist():
        target = (dest / info.filename).resolve()
        if target != dest and dest not in target.parents:
            raise ValueError(f"压缩包含越界路径: {info.filename}")
    z.extractall(dest)


def _extract_tar_member(tgz: Path, suffix: str, dest: Path) -> None:
    """只抽取 tar 内指定后缀的单个文件到 dest；拒绝 ../、绝对路径成员与越界目标（防 tar-slip）。"""
    dest = dest.resolve()
    if FFMPEG_DIR.resolve() not in dest.parents:
        raise ValueError(f"解压目标越界: {dest}")
    with tarfile.open(tgz, "r:gz") as t:
        for name in t.getnames():
            if Path(name).is_absolute() or ".." in Path(name).parts:
                continue
            if not name.endswith(suffix):
                continue
            member = t.extractfile(name)
            if member is None:
                continue
            with member as src:
                data = src.read()
            if data:
                dest.write_bytes(data)
                return
    raise ValueError(f"压缩包内未找到 {suffix}")


def _find_local_node() -> Path | None:
    """先扫本机已装 Node（Program Files / 用户目录 / nvm），避免重复下载。"""
    dirs = [
        Path(os.environ.get("ProgramFiles", "")) / "nodejs",
        Path(os.environ.get("ProgramFiles(x86)", "")) / "nodejs",
        Path(os.environ.get("LOCALAPPDATA", "")) / "Programs" / "nodejs",
        Path("C:/Program Files/nodejs"),
    ]
    nvm = Path(os.environ.get("APPDATA", "")) / "nvm"
    if nvm.exists():
        dirs += sorted((d for d in nvm.iterdir() if (d / "node.exe").exists()), reverse=True)
    for d in dirs:
        if str(d) and (d / "node.exe").exists():
            return d / "node.exe"
    return None


def _link_node_dir(src_dir: Path) -> None:
    """把本机 Node 目录挂到 tools\\node（junction 不需管理员），供 start-all 的 PATH 使用。"""
    if NODE_DIR.exists():
        return
    try:
        r = subprocess.run(["cmd", "/c", "mklink", "/J", str(NODE_DIR), str(src_dir)],
                           capture_output=True, text=True)
    except OSError as e:
        log(f"         [提示] 无法建立 Node 目录联接: {e}")
        return
    if r.returncode == 0:
        log(f"         已挂接本机 Node 到 {NODE_DIR}")
    else:
        log("         [提示] 目录联接失败；若 npm 不在 PATH，Web 面板可能起不来")


def _node_version(node: Path) -> str:
    try:
        return subprocess.run([str(node), "--version"], capture_output=True,
                              text=True, timeout=30).stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        return ""


def _node_asset() -> str:
    """Node 便携包资产名：版本/URL/SHA256 全部由 scripts/assets.lock.json 钉死（T-005，
    消除动态 latest 的版本漂移）；npmmirror 与官方 nodejs.org 为同文件双源。"""
    for key in _assets_lock()["assets"]:
        if key.startswith("node-") and key.endswith("-win-x64.zip"):
            return key
    raise SystemExit("[失败] assets.lock.json 缺少 node 资产条目")


def _promote_extracted_dir(extracted: Path, dest: Path) -> None:
    """把解压出的目录就位到 dest（任务109）。

    Windows 上两种常见死法：dest 已存在（上次半失败/手动放过）rename 抛 FileExistsError；
    extracted 与 dest 跨盘 rename 抛 OSError。都降级处理：先清掉旧 dest（删不掉就保留，
    改走覆盖复制），rename 失败回退 copytree(dirs_exist_ok=True) 后删源。
    """
    if dest.exists():
        try:
            shutil.rmtree(dest)
        except OSError:
            pass  # 删不掉（占用/权限）→ 留给 copytree dirs_exist_ok 覆盖
    try:
        extracted.rename(dest)
        return
    except OSError:
        pass
    shutil.copytree(extracted, dest, dirs_exist_ok=True)
    shutil.rmtree(extracted, ignore_errors=True)


def ensure_node() -> None:
    if _node_ok():
        log("  [跳过] Node.js 已安装: " + subprocess.run(
            ["node", "--version"], capture_output=True, text=True).stdout.strip())
        return
    if (NODE_DIR / "node.exe").exists():
        os.environ["PATH"] = str(NODE_DIR) + os.pathsep + os.environ.get("PATH", "")
        log(f"  [跳过] 使用项目自带 Node: {NODE_DIR}")
        return
    local = _find_local_node()
    if local:
        os.environ["PATH"] = str(local.parent) + os.pathsep + os.environ.get("PATH", "")
        _link_node_dir(local.parent)
        log(f"  [跳过] 使用本机 Node: {local} {_node_version(local)}")
        return
    name = _node_asset()
    log(f"  [4] 本机未找到 Node.js，下载便携版 {name}（哈希锁定，约 30MB）...")
    zpath = ROOT / "Resources" / "logs" / name
    try:
        download_verified(name, zpath)
        log(f"    解压到 {NODE_DIR} ...")
        with zipfile.ZipFile(zpath) as z:
            _safe_extract_zip(z, ROOT / "tools")
        extracted = ROOT / "tools" / name.removesuffix(".zip")
        if extracted.exists():
            _promote_extracted_dir(extracted, NODE_DIR)
        os.environ["PATH"] = str(NODE_DIR) + os.pathsep + os.environ.get("PATH", "")
        log(f"    Node {_node_version(NODE_DIR / 'node.exe')} 就绪（SHA256 已校验）")
    except Exception as e:  # noqa: BLE001
        log(f"    Node 下载/解压失败: {e}")
        log("  [警告] Node 便携版未下载成功：Web 面板将不可用（服务端/分析不受影响）")
    finally:
        zpath.unlink(missing_ok=True)


def ensure_npm_deps() -> None:
    vite = ROOT / "web" / "node_modules" / ".bin" / "vite.cmd"
    if vite.exists():
        log("  [跳过] web/node_modules 已安装")
        return
    log("  [5] 安装 Web 依赖（npm install，npmmirror 镜像，首次 2-5 分钟）...")
    try:
        subprocess.run(["npm.cmd", "install", "--registry=https://registry.npmmirror.com"],
                       cwd=ROOT / "web", check=True)
        return
    except subprocess.CalledProcessError:
        log("    npmmirror 安装失败，切换官方 registry 重试...")
    subprocess.run(["npm.cmd", "install"], cwd=ROOT / "web", check=True)


# ---------- ffmpeg ----------
def _find_local_exe(name: str) -> Path | None:
    """先扫 PATH 与本机常见安装位置，找到就不下载。"""
    hit = shutil.which(name)
    if hit:
        return Path(hit)
    dirs = (FFMPEG_DIR,
            Path(os.environ.get("ProgramFiles", "")) / "ffmpeg" / "bin",
            Path("C:/ffmpeg/bin"),
            Path(os.environ.get("LOCALAPPDATA", "")) / "Microsoft" / "WinGet" / "Links",
            Path(os.environ.get("ProgramData", "")) / "chocolatey" / "bin")
    for d in dirs:
        if not str(d):
            continue
        p = d / (name + ".exe")
        if p.exists():
            return p
    return None


def _link_dir(src_dir: Path, link: Path) -> bool:
    """目录联接（junction，无需管理员）：让 start-all 的 PATH 指到本机已有组件目录。"""
    if link.exists():
        return (link / "ffmpeg.exe").exists()
    try:
        r = subprocess.run(["cmd", "/c", "mklink", "/J", str(link), str(src_dir)],
                           capture_output=True, text=True)
    except OSError:
        return False
    return r.returncode == 0


def ensure_ffmpeg() -> None:
    if (FFMPEG_DIR / "ffmpeg.exe").exists():
        log("  [跳过] ffmpeg 已可用")
        return
    local = _find_local_exe("ffmpeg")
    if local:
        log(f"  [跳过] 本机已有 ffmpeg: {local}")
        if local.parent.resolve() == FFMPEG_DIR.resolve() or _link_dir(local.parent, FFMPEG_DIR):
            return
        log("  [提示] 无法挂接本机 ffmpeg 目录，改为下载到项目内...")
    if not local:
        log("  [6] 本机未找到 ffmpeg，尝试下载（npmmirror CDN）...")
    FFMPEG_DIR.mkdir(parents=True, exist_ok=True)
    try:
        # ffmpeg-static 的 binary 目录提供单文件 .gz（win32-x64）——版本与 SHA256
        # 由 assets.lock.json 钉死（T-005，消除动态 latest 的版本漂移）
        fname = "ffmpeg-win32-x64.gz"
        gz = download_verified(fname, ROOT / "Resources" / "logs" / fname)
        with gzip.open(gz, "rb") as g, (FFMPEG_DIR / "ffmpeg.exe").open("wb") as out:
            shutil.copyfileobj(g, out)
        gz.unlink()
        log("    ffmpeg 来源: ffmpeg-static（SHA256 已校验，版本见 assets.lock.json）")
    except Exception as e:  # noqa: BLE001
        log(f"    ffmpeg-static 源失败，切换 @ffmpeg-installer 源: {e}")
        try:
            tgz = download_verified("ffmpeg-installer.tgz", ROOT / "Resources" / "logs" / "ffmpeg-installer.tgz")
            _extract_tar_member(tgz, "ffmpeg.exe", FFMPEG_DIR / "ffmpeg.exe")
            tgz.unlink()
            log("    ffmpeg 来源: @ffmpeg-installer/win32-x64（SHA256 已校验）")
        except Exception as e2:  # noqa: BLE001
            log(f"  [警告] ffmpeg 自动下载失败: {e2}")
            log("         实时画面的文件源需要它：从 https://www.gyan.dev/ffmpeg/builds/ 下载后把")
            log(f"         ffmpeg.exe 放到 {FFMPEG_DIR}；不影响服务端其余功能")

    if (FFMPEG_DIR / "ffprobe.exe").exists() or shutil.which("ffprobe"):
        return
    local_probe = _find_local_exe("ffprobe")
    if local_probe:
        log(f"  [跳过] 本机已有 ffprobe: {local_probe}")
        return
    try:
        # ffprobe 不在 ffmpeg-static binary 里，从 ffprobe-static 的 npm 包 tarball 抽取；
        # tarball 版本与 SHA256 由 assets.lock.json 钉死（npmjs 官方 + npmmirror 镜像双源）
        tgz = download_verified("ffprobe-static.tgz", ROOT / "Resources" / "logs" / "ffprobe-static.tgz")
        _extract_tar_member(tgz, "win32/x64/ffprobe.exe", FFMPEG_DIR / "ffprobe.exe")
        tgz.unlink()
        log("    ffprobe 来源: ffprobe-static npm tarball（SHA256 已校验）")
    except Exception as e:  # noqa: BLE001
        log(f"  [提示] ffprobe 未获取（可选组件，不影响演示）: {e}")


# ---------- 配置与数据 ----------
def ensure_env_file() -> None:
    """保证 server/.env 的模型行指向当前安装位置（任务109：只动这一行，用户其余配置逐字保留）。"""
    env = ROOT / "server" / ".env"
    model = ROOT / "m0-spike" / "yolo11n_ppe.onnx"
    if not env.exists():
        log("  [7] 生成 server/.env")
        env.parent.mkdir(parents=True, exist_ok=True)
        env.write_text(
            "# LabSentinel 本地配置（deploy.py 自动生成；告警渠道等可选项见 server/.env.example）\n"
            f"LS_PPE_MODEL={model.as_posix()}\n"
            "LS_PPE_CLASS_NAMES=lab coat\n"
            "LS_PPE_VIOLATION_CLASS=no lab coat\n"
            "LS_PPE_CONF=0.30\n"
            "LS_FILE_LOOP=true\n",
            encoding="utf-8")
        return
    m = None
    for line in env.read_text(encoding="utf-8").splitlines():
        if line.strip().startswith("LS_PPE_MODEL="):
            m = line.split("=", 1)[1].strip()
    # 只接受指向当前安装位置的配置；失效/来自其他机器 → 只修模型行（不再整文件重写）
    if m and Path(m).exists() and ROOT.as_posix() in Path(m).as_posix():
        log(f"  [跳过] server/.env 有效（模型: {m}）")
        return
    lines = env.read_text(encoding="utf-8").splitlines()
    new_line = f"LS_PPE_MODEL={model.as_posix()}"
    replaced = False
    for i, line in enumerate(lines):
        if line.strip().startswith("LS_PPE_MODEL="):
            lines[i] = new_line
            replaced = True
            break
    if not replaced:
        lines.append(new_line)
    try:  # 写前备份一份，用户告警配置多一层保险（备份失败不阻塞）
        shutil.copy2(env, env.with_name(".env.bak-" + time.strftime("%Y%m%d-%H%M%S")))
    except OSError:
        pass
    env.write_text("\n".join(lines) + "\n", encoding="utf-8")
    log("  [更新] server/.env 模型路径已修正（其余行逐字保留）")


def write_go2rtc_yaml() -> None:
    """按种子/库里的摄像头源生成 tools/go2rtc.yaml（与 server/app/go2rtc.py 冷启动路径同构）。

    打包时 yaml 里是打包机的绝对路径，新机器 go2rtc 先于服务端启动会读到坏流——
    部署时直接重写为本机路径，省掉首次启动的坏流窗口。
    T-101：与服务端同构——回环监听 + 鉴权 + 去 origin:* + 媒体端口独立。
    """
    import json as _json
    import secrets as _secrets
    import sqlite3 as _sq
    db_path = ROOT / "server" / "data" / "labsentinel.db"
    if not db_path.exists():
        return
    db = _sq.connect(db_path)
    rows = db.execute("select id, source from cameras where type='file' order by id").fetchall()
    db.close()
    # 凭据与服务端共用同一份 data/go2rtc.secret（服务端首启生成；部署期先生成占位）
    secret_path = db_path.parent / "go2rtc.secret"
    if secret_path.exists():
        try:
            sd = _json.loads(secret_path.read_text("ascii"))
            user, pwd = sd["username"], sd["password"]
        except (OSError, ValueError, KeyError):
            user, pwd = "ls-" + _secrets.token_urlsafe(12), _secrets.token_urlsafe(24)
    else:
        user, pwd = "ls-" + _secrets.token_urlsafe(12), _secrets.token_urlsafe(24)
        secret_path.write_text(_json.dumps({"username": user, "password": pwd}), encoding="ascii")
    lines = [
        "# 本文件由 LabSentinel 服务端/部署脚本自动生成（摄像头增删时重写），勿手工编辑",
        "api:", '  listen: "127.0.0.1:1984"',
        f"  username: {_json.dumps(user)}", f"  password: {_json.dumps(pwd)}",
        "# origin: 删除（禁止任意网页跨域读控制面）",
        "webrtc:", '  listen: ":8555"',
        "rtsp:", '  listen: ""',
        "log:", "  level: warn",
        "streams:",
    ]
    for cid, src in rows:
        if src:
            # YAML 双引号串里反斜杠是转义符：Windows 路径必须转成 /；
            # JSON 字符串转义挡引号/换行注入（T-103）
            forward = src.replace("\\", "/")
            lines.append(f"  {_json.dumps(str(cid))}: {_json.dumps('ffmpeg:' + forward)}")
    (ROOT / "tools" / "go2rtc.yaml").write_text("\n".join(lines), encoding="utf-8")


def seed_database() -> None:
    """播种/路径修正（任务51：播种逻辑单源 scripts/seed_demo.py，本函数只做源码模式适配）。

    新库 → seed_demo.seed_database（media= m0-spike/data）；
    已有库 → 按名称把指向失效的文件源路径修正到当前安装位置。
    """
    db_path = ROOT / "server" / "data" / "labsentinel.db"
    db_path.parent.mkdir(parents=True, exist_ok=True)
    media_dir = ROOT / "m0-spike" / "data"
    inserted = seed_demo.seed_database(db_path, media_dir)
    if inserted:
        log(f"  [8] 播种演示摄像头 {inserted} 路（单源 scripts/seed_demo.py）")
        return
    db = sqlite3.connect(str(db_path))
    cams = db.execute("select id, name, source from cameras").fetchall()
    by_name = {name: str(media_dir / fn) for name, fn in SEED_VIDEO_FILES.items()}
    fixed = 0
    for cid, name, src in cams:
        want = by_name.get(name)
        if want and (not src or not Path(src).exists()) and Path(want).exists():
            db.execute("update cameras set source=? where id=?", (want, cid))
            fixed += 1
    db.commit()
    db.close()
    log(f"  [8] 数据库已有 {len(cams)} 路摄像头，修正失效路径 {fixed} 条")


def check_model_files() -> bool:
    need = ["m0-spike/yolo11n.onnx",            # 主 Engine 默认模型（缺了服务 500）
            "m0-spike/yolo11n_ppe.onnx", "m0-spike/yolo11n_ppe.classes.txt",
            "m0-spike/data/lab-walk-24076.mp4", "m0-spike/data/lab-equip-21453.mp4"]
    missing = [n for n in need if not (ROOT / n).exists()]
    for n in missing:
        log(f"  [失败] 缺少关键文件: {n}（发布包不完整？）")
    if not missing and not (ROOT / "tools" / "go2rtc.exe").exists():
        # go2rtc 只负责 WebRTC 实时画面；缺了不该让整个部署失败
        log("  [提示] 缺少 tools/go2rtc.exe：仅实时画面不可用，分析/事件/告警不受影响")
    return not missing


def _pid_is_ours(pid: str, port: int) -> bool:
    """端口回收前的身份核实（2026-11-13 P1 审查实证：原按端口盲杀，8000/1984/5173
    都是常见开发端口，会误杀无关应用）。LabSentinel python（uvicorn/probe/vite
    标记，与 launcher 同源）或本仓 tools/ 下的 go2rtc 才允许杀；宁漏杀不误杀。"""
    import portable_launcher as _pl
    try:
        if _pl._pid_matches_launcher(int(pid)) is True:
            return True
        out = subprocess.run(
            [_pl._powershell_exe(), "-NoProfile", "-Command",
             f"(Get-CimInstance Win32_Process -Filter \"ProcessId={int(pid)}\").CommandLine"],
            capture_output=True, text=True, errors="replace", timeout=10).stdout or ""
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return False
    cmd = out.strip()
    return bool(cmd) and "go2rtc" in cmd.lower() and str(ROOT).lower() in cmd.lower()


def kill_ports() -> None:
    for port in PORTS:
        out = subprocess.run(["netstat", "-ano"], capture_output=True, text=True).stdout or ""
        pids = {ln.split()[-1] for ln in out.splitlines()
                if f":{port} " in ln and "LISTENING" in ln}
        for pid in pids:
            if not _pid_is_ours(pid, port):
                log(f"  [跳过] 端口 {port} 的 pid={pid} 非 LabSentinel/go2rtc 进程，不杀")
                continue
            r = subprocess.run(["taskkill", "/PID", pid, "/F"], capture_output=True)
            if r.returncode != 0:
                log(f"  [警告] pid={pid}（端口 {port}）强杀失败 rc={r.returncode}，"
                    "后续步骤若报文件/端口占用请先手动停止")


# ---------- 命令 ----------
def cmd_check(quiet: bool = False) -> int:
    items: list[tuple[str, bool, str]] = []
    notes: list[str] = []  # 可选组件缺失：只提示，不计入失败

    def add(name: str, ok: bool, hint: str = ""):
        items.append((name, ok, hint))

    py = find_system_python()
    add("Python 3.10+", py is not None, "python.org 安装，勾选 Add to PATH")
    v = find_venv()
    add("venv 虚拟环境", v is not None, "python scripts/deploy.py install")
    if v:
        # R2-002/N-20: the unified lock pins onnxruntime CPU on Windows; DirectML
        # becomes an optional variant again in R2-303, so do not require Dml here.
        # 2026-11-11 反误报治理：`-c 动态代码串` → 文件执行（probe_runtime.py，与
        # build_portable.smoke 同一探针；不带 --app-dir 即纯依赖体检）。
        probe = Path(__file__).resolve().parent / "probe_runtime.py"
        r = subprocess.run([str(venv_python(v)), str(probe)], capture_output=True, text=True)
        add("服务端依赖（锁文件）", r.returncode == 0,
            r.stderr.strip().splitlines()[-1] if r.returncode and r.stderr
            else f"python scripts/deploy.py install（{_platform_lock()}）")
        if not (v / "Lib" / "site-packages" / "cv2" / "openh264-2.5.0-win64.dll").exists():
            # 可选组件：缺了不该让体检失败（否则每次启动都会重跑部署）
            notes.append("OpenH264 DLL 缺失（可选：优先用系统 H.264，失败回退 mp4v）")
    add("Node.js", _node_ok() or (NODE_DIR / "node.exe").exists(), "nodejs.org 或 deploy install 自动下载")
    add("Web 依赖 node_modules", (ROOT / "web" / "node_modules" / ".bin" / "vite.cmd").exists())
    add("ffmpeg", (FFMPEG_DIR / "ffmpeg.exe").exists() or shutil.which("ffmpeg") is not None)
    add("模型与演示视频", check_model_files())
    env_ok = False
    env = ROOT / "server" / ".env"
    if env.exists():
        for line in env.read_text(encoding="utf-8").splitlines():
            if line.strip().startswith("LS_PPE_MODEL=") and Path(line.split("=", 1)[1].strip()).exists():
                env_ok = True
    add("server/.env（模型路径有效）", env_ok)

    if not (ROOT / "tools" / "go2rtc.exe").exists():
        notes.append("tools/go2rtc.exe 缺失（实时画面不可用，分析与事件不受影响）")

    bad = [(n, h) for n, ok, h in items if not ok]
    if not quiet:
        log("LabSentinel 环境体检")
        log("=" * 56)
        for n, ok, h in items:
            log(f"  [{'OK ' if ok else '缺失'}] {n}" + (f"  <- {h}" if h and not ok else ""))
        log("=" * 56)
        log(f"{len(items) - len(bad)}/{len(items)} 项就绪" + ("" if not bad else f"，缺 {len(bad)} 项"))
        for note in notes:
            log(f"  [提示] {note}")
    elif bad:
        log("[部署] 缺以下组件，自动补齐：")
        for n, h in bad:
            log(f"    - {n}")
    return 0 if not bad else 1


def cmd_install() -> int:
    if not check_model_files():
        return 1
    log("[部署] 开始自动部署（每步已就绪自动跳过）")
    venv = ensure_venv()
    write_venv_path(venv)
    pip_install(venv)
    ensure_openh264(venv)
    ensure_node()
    ensure_npm_deps()
    ensure_ffmpeg()
    ensure_env_file()
    seed_database()
    write_go2rtc_yaml()
    log("[部署] 完成。venv: " + str(venv))
    return 0 if cmd_check(quiet=True) == 0 else 1


def _drop_env(full: bool = False) -> None:
    """删除部署生成物（数据目录另行处理）。

    full=False（重部署）：删 venv/依赖/缓存/配置——可快速重建的东西。
    full=True（卸载）：连同下载的大件（tools/node、tools/ffmpeg）一起删。
    只删项目内（.venv）的 venv；项目外的旧环境（如 D:\\LabSentinel\\m0-venv）不是本脚本创建的，
    仅解除 venv_path.txt 引用、绝不删除——重部署/卸载不允许殃及机器上已有的其他环境。
    """
    kill_ports()
    if VENV_MARK.exists():
        marked = _read_venv_mark()
        VENV_MARK.unlink()
        if marked and marked.exists() and ROOT in marked.parents:
            # 标记文件存的是 python.exe，venv 根是其上两级
            vroot = marked.parent.parent
            log(f"  删除 venv: {vroot}")
            shutil.rmtree(vroot, ignore_errors=True)
        elif marked and marked.exists():
            log(f"  保留项目外 venv（仅解除引用）: {marked}")
    local_venv = ROOT / ".venv"
    if local_venv.exists():
        log(f"  删除 venv: {local_venv}")
        shutil.rmtree(local_venv, ignore_errors=True)
    drop = [ROOT / "web" / "node_modules", ROOT / "web" / "dist"]
    if full:
        drop += [NODE_DIR, FFMPEG_DIR]
    for d in drop:
        if d.exists():
            log(f"  删除 {d.relative_to(ROOT)}")
            shutil.rmtree(d, ignore_errors=True)
    for f in (ROOT / "server" / ".env",):
        if f.exists():
            f.unlink()
    for bak in (ROOT / "server").glob(".env.bak-*"):  # ensure_env_file 的写前备份一并清
        bak.unlink(missing_ok=True)
    for d in (ROOT / "Resources" / "logs", ROOT / ".pytest_cache"):
        if d.exists():
            shutil.rmtree(d, ignore_errors=True)
    for c in ROOT.rglob("__pycache__"):
        if "node_modules" not in c.parts:
            shutil.rmtree(c, ignore_errors=True)


def cmd_redeploy(wipe: bool) -> int:
    log("[重部署] 停止服务并删除已装环境（venv/依赖/缓存）...")
    _drop_env()
    if wipe:
        data = ROOT / "server" / "data"
        if data.exists():
            log(f"[重部署] 清空数据目录 {data}（数据库/事件截图/视频/标注池）")
            shutil.rmtree(data)
    cmd_install()
    if wipe:
        log("[重部署] 数据已重置为演示初始状态（4 路演示摄像头 + 区域/规则，无历史事件）")
    else:
        log("[重部署] server/data 数据未动（事件/摄像头/告警配置全保留）")
    return 0


def cmd_uninstall() -> int:
    log("[卸载] 停止服务、删除 venv/依赖/配置/日志/下载组件...")
    _drop_env(full=True)
    data = ROOT / "server" / "data"
    if data.exists():
        backup = data.with_name("data.uninstalled-" + time.strftime("%Y%m%d-%H%M%S"))
        data.rename(backup)
        log(f"[卸载] 数据目录已备份为 {backup.name}（确认不需要后可手动删除）")
    log("[卸载] 完成。模型/源码/文档保留在原位；整个项目不要了可直接删除项目文件夹。")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description="LabSentinel 一键部署引擎")
    ap.add_argument("action", choices=["check", "install", "redeploy", "uninstall"])
    ap.add_argument("--quiet", action="store_true", help="check 模式只列缺失项")
    ap.add_argument("--wipe", action="store_true", help="redeploy 时清空数据")
    ap.add_argument("--keep-data", action="store_true", help="redeploy 时保留数据")
    a = ap.parse_args()
    if a.action == "check":
        return cmd_check(quiet=a.quiet)
    if a.action == "install":
        return cmd_install()
    if a.action == "redeploy":
        if a.wipe == a.keep_data:
            ap.error("redeploy 需要 --wipe 或 --keep-data 二选一")
        return cmd_redeploy(wipe=a.wipe)
    return cmd_uninstall()


if __name__ == "__main__":
    sys.exit(main())
