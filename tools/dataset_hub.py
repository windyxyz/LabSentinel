"""LS-056：多源公开数据集 / 现成权重下载器（开发支具，不进发行包）。

用法：
    python tools/dataset_hub.py list
    python tools/dataset_hub.py download lab-ppe --out Resources/datasets/_downloads
    HF_ENDPOINT=https://hf-mirror.com python tools/dataset_hub.py download hf-ppe-detection

纪律：
- 仅下载登记表（tools/datasets_registry.json）中的 https 源；不写死凭据。
- 逐项校验 sha256（登记了才校验）与文件大小；失败删除残件。
- 纯标准库；不 import 产品运行时。
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = ROOT / "tools" / "datasets_registry.json"


def load_registry(path: Path = REGISTRY) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def find_entry(registry: dict, name: str) -> dict | None:
    for bucket in ("datasets", "weights"):
        for e in registry.get(bucket, []):
            if e.get("name") == name:
                return e
    return None


def resolve_url(entry: dict, hf_endpoint: str | None = None) -> str | None:
    """把登记项解析为可下载 URL；local 返回 None（无需下载）。"""
    t = entry.get("type")
    if t == "local":
        return None
    if t == "direct":
        return entry.get("url")
    if t == "huggingface":
        base = (hf_endpoint or os.environ.get("HF_ENDPOINT") or "https://huggingface.co").rstrip("/")
        repo = entry.get("repo", "")
        file = entry.get("file", "")
        if not repo or not file:
            raise ValueError(f"huggingface 条目缺少 repo/file: {entry.get('name')}")
        prefix = "" if entry.get("kind") == "model" else "datasets/"
        return f"{base}/{prefix}{repo}/resolve/main/{urllib.parse.quote(file)}"
    if t == "roboflow":
        # Roboflow 需要 API Key：仅拼接导出页，实际下载交给用户浏览器
        return entry.get("url")
    raise ValueError(f"未知类型: {t}")


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def download(entry: dict, out_dir: Path, hf_endpoint: str | None = None) -> Path:
    url = resolve_url(entry, hf_endpoint)
    name = entry.get("name", "dataset")
    out_dir.mkdir(parents=True, exist_ok=True)
    if url is None:
        local = ROOT / entry.get("path", "")
        print(f"[local] {name}: {local}")
        return local
    if entry.get("type") == "roboflow":
        raise SystemExit(f"请在 Roboflow 导出 YOLO ZIP（需要账户），或使用已有本地副本：{url}")
    if urllib.parse.urlsplit(url).scheme != "https":
        raise ValueError("下载地址必须使用 HTTPS")
    suffix = Path(urllib.parse.urlsplit(url).path).suffix or ".bin"
    dest = out_dir / f"{name}{suffix}"
    tmp = dest.with_suffix(dest.suffix + ".part")
    print(f"[download] {name} <- {url}")
    try:
        with urllib.request.urlopen(url, timeout=60) as r, tmp.open("wb") as f:
            while chunk := r.read(1 << 20):
                f.write(chunk)
            expected = r.headers.get("Content-Length")
        size = tmp.stat().st_size
        if not size or (expected and size != int(expected)):
            raise ValueError("下载文件为空或未完整接收")
        want = (entry.get("sha256") or "").lower()
        if want and sha256_file(tmp) != want:
            raise ValueError("下载文件 SHA256 不匹配")
        tmp.replace(dest)
    finally:
        tmp.unlink(missing_ok=True)
    print(f"[ok] {dest}（{dest.stat().st_size / 1048576:.1f} MB）")
    return dest


def cmd_list(registry: dict) -> None:
    for bucket, title in (("datasets", "数据集"), ("weights", "现成权重")):
        print(f"== {title} ==")
        for e in registry.get(bucket, []):
            print(
                f"  {e.get('name'):24s} {e.get('type'):12s} {e.get('license','?'):12s} {e.get('title','')}"
            )


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="LabSentinel 公开数据集/权重下载器")
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list", help="列出登记表")
    dl = sub.add_parser("download", help="下载指定条目")
    dl.add_argument("name")
    dl.add_argument("--out", default=str(ROOT / "Resources" / "datasets" / "_downloads"))
    args = ap.parse_args(argv)
    registry = load_registry()
    if args.cmd == "list":
        cmd_list(registry)
        return 0
    entry = find_entry(registry, args.name)
    if not entry:
        print(f"[失败] 登记表无此条目: {args.name}", file=sys.stderr)
        return 2
    download(entry, Path(args.out))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
