"""T-408：生成签名更新清单 latest.json（T-110 消费）。

产物：
  {version, published_at, channel, platforms: {"windows-x64": {url, sha256, size, notes_url}}}
  以及 latest.json.minisig（若本机有 minisign 私钥；无则跳过并提示）
用法:
  python scripts/make_update_manifest.py --base-url https://github.com/windyxyz/LabSentinel/releases/download/v0.2.0
  python scripts/make_update_manifest.py --sign-key D:/keys/minisign.key
"""
import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / "Resources" / "dist"


def _sha256(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def find_artifact(pattern: str) -> Path | None:
    hits = sorted(DIST.glob(pattern), key=lambda p: p.stat().st_mtime, reverse=True)
    return hits[0] if hits else None


def build(base_url: str, channel: str) -> dict:
    ver = (ROOT / "VERSION").read_text("ascii").strip()
    platforms: dict[str, dict] = {}
    py_zip = find_artifact("LabSentinel-portable-*.zip")
    if py_zip:
        platforms["windows-x64"] = {
            "url": f"{base_url}/{py_zip.name}", "sha256": _sha256(py_zip),
            "size": py_zip.stat().st_size, "kind": "portable-python"}
    rust_zip = find_artifact("labsentinel-rust-*.zip")
    if rust_zip:
        platforms["windows-x64-rust"] = {
            "url": f"{base_url}/{rust_zip.name}", "sha256": _sha256(rust_zip),
            "size": rust_zip.stat().st_size, "kind": "single-binary"}
    if not platforms:
        raise SystemExit("[失败] Resources/dist 下没有可发布的 zip（先构建）")
    return {"version": ver, "published_at": datetime.now().isoformat(timespec="seconds"),
            "channel": channel, "min_app_version": "0.2.0",
            "notes_url": f"{base_url}/CHANGELOG.md", "platforms": platforms}


def sign(path: Path, key: str | None) -> None:
    exe = shutil.which("minisign") or (key and str(Path(key).with_name("minisign.exe")))
    if not exe or not key:
        print("[提示] 未提供 minisign（--sign-key + PATH 中的 minisign）——清单未签名")
        print("       部署 minisign 后重跑；客户端在公钥存在时强制验签（T-110）")
        return
    r = subprocess.run([exe, "-S", "-s", key, "-m", str(path)],
                       capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"[失败] minisign 签名失败: {r.stderr[:200]}")
    print(f"[签名] {path}.minisig")


def main() -> int:
    ap = argparse.ArgumentParser(description="生成更新清单 latest.json（T-408）")
    ap.add_argument("--base-url", required=True, help="发布资产基地址（https）")
    ap.add_argument("--channel", default="stable")
    ap.add_argument("--out", default=str(DIST / "latest.json"))
    ap.add_argument("--sign-key", default="", help="minisign 私钥路径（可选）")
    a = ap.parse_args()
    if not a.base_url.startswith("https://"):
        raise SystemExit("[失败] base-url 必须 https")
    data = build(a.base_url, a.channel)
    out = Path(a.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"[OK] {out}")
    print(json.dumps(data, ensure_ascii=False, indent=2))
    sign(out, a.sign_key or None)
    return 0


if __name__ == "__main__":
    sys.exit(main())
