"""T-402/Q-08：按 scripts/assets.lock.json 下载并逐源 SHA256 校验资产。

用法（标准库）:
    python scripts/fetch_assets.py --name yolo11n_ppe.onnx --out models/yolo11n_ppe.onnx
    python scripts/fetch_assets.py --all --out-dir models/
    python scripts/fetch_assets.py --list

铁律：
  - 任何源下载后必须 sha256 与 size 双校验，不匹配即删除残件换下一源；
  - mirror_urls 仅在 LS_ALLOW_MIRROR_PROXY=1 时启用（哈希匹配即可证明内容）；
  - 不写死凭据；下载 URL 全部来自锁文件（https）。
"""
import argparse
import hashlib
import json
import os
import sys
import urllib.request
from pathlib import Path

# CI 控制台（GitHub runner PowerShell）默认 GBK，中文日志打印即 charmap 炸
# （2026-11-13 release.yml fetch base model 步骤实测）——stdout/stderr 重配 UTF-8
if sys.stdout.encoding and sys.stdout.encoding.lower() != "utf-8":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

ROOT = Path(__file__).resolve().parent.parent
LOCK = ROOT / "scripts" / "assets.lock.json"


def load_lock() -> dict:
    return json.loads(LOCK.read_text(encoding="utf-8"))["assets"]


def allowed_hosts(assets: dict) -> set:
    """从锁文件收集允许的下载域名（URL 必须逐字来自锁文件，防篡改外链）。"""
    import urllib.parse
    hosts = set()
    for spec in assets.values():
        for u in list(spec.get("urls", [])) + list(spec.get("mirror_urls", [])):
            try:
                p = urllib.parse.urlsplit(u)
                if p.scheme == "https" and p.hostname:
                    hosts.add(p.hostname.lower())
            except ValueError:
                pass
    return hosts


def check_url(url: str, allowed: set) -> str:
    """下载前校验：https + 锁文件域名白名单 + 无凭证/查询串/片段。"""
    import urllib.parse
    p = urllib.parse.urlsplit(url)
    if p.scheme != "https" or (p.hostname or "").lower() not in allowed:
        raise ValueError(f"URL 不在锁文件域名白名单: {url}")
    if p.username or p.password or p.query or p.fragment:
        raise ValueError("URL 不允许携带凭证/查询串/片段")
    return url


def verify(path: Path, sha256: str, size: int) -> bool:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest() == sha256.lower() and path.stat().st_size == size


def download(name: str, out: Path) -> int:
    assets = load_lock()
    allowed = allowed_hosts(assets)
    spec = assets.get(name)
    if spec is None:
        print(f"[fetch] 未知资产: {name}")
        return 2
    urls = list(spec.get("urls", []))
    if os.environ.get("LS_ALLOW_MIRROR_PROXY") == "1":
        urls += list(spec.get("mirror_urls", []))
    out.parent.mkdir(parents=True, exist_ok=True)
    for url in urls:
        try:
            check_url(url, allowed)  # SSRF 防护：只允许锁文件白名单域名的 https
        except ValueError as e:
            print(f"[fetch] URL 被拒绝: {e}")
            continue
        if not url.startswith("https://"):
            continue
        tmp = out.with_suffix(out.suffix + ".part")
        try:
            print(f"[fetch] {name} <- {url}")
            with urllib.request.urlopen(url, timeout=60) as r, tmp.open("wb") as w:
                while True:
                    chunk = r.read(1 << 20)
                    if not chunk:
                        break
                    w.write(chunk)
            if verify(tmp, spec["sha256"], spec["size"]):
                tmp.replace(out)
                print(f"[fetch] OK {out} ({out.stat().st_size} B, sha256 校验通过)")
                return 0
            print("[fetch] 校验不匹配，删除残件换源")
        except (OSError, ValueError) as e:
            print(f"[fetch] 源失败: {e}")
        finally:
            tmp.unlink(missing_ok=True)
    print(f"[fetch] 所有源均失败: {name}")
    return 1


def main() -> int:
    ap = argparse.ArgumentParser(description="按 assets.lock.json 下载校验资产（Q-08）")
    ap.add_argument("--name", help="资产名（如 yolo11n_ppe.onnx）")
    ap.add_argument("--out", type=Path, help="输出路径")
    ap.add_argument("--all", action="store_true", help="下载全部资产（CI/资产库预热）")
    ap.add_argument("--out-dir", type=Path, default=None, help="--all 的输出目录")
    ap.add_argument("--list", action="store_true", help="列出资产名与哈希")
    a = ap.parse_args()
    if a.list:
        for n, s in load_lock().items():
            print(f"{n}\t{s['sha256'][:16]}…\t{s['size']} B")
        return 0
    if a.all:
        out_dir = a.out_dir or (ROOT / "Resources" / "assets-cache")
        rc = 0
        for n in load_lock():
            rc |= download(n, out_dir / n)
        return rc
    if not a.name or not a.out:
        print("[fetch] 需要 --name 与 --out（或 --all / --list）")
        return 2
    return download(a.name, a.out)


if __name__ == "__main__":
    sys.exit(main())
