"""T-005 回归：资产哈希清单结构与下载源卫生（防 http:// 明文下载源回归）。"""
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LOCK = ROOT / "scripts" / "assets.lock.json"


def test_assets_lock_schema():
    lock = json.loads(LOCK.read_text("utf-8"))
    assert lock.get("assets"), "缺少 assets 条目"
    for key, entry in lock["assets"].items():
        assert re.fullmatch(r"[0-9a-f]{64}", entry["sha256"]), f"{key}: sha256 必须是 64 位十六进制"
        assert entry.get("urls"), f"{key}: 缺少官方源 urls"
        for url in entry["urls"]:
            assert url.startswith("https://"), f"{key}: 官方源必须 https（禁 http 明文）: {url}"
        assert isinstance(entry.get("size"), int) and entry["size"] > 0, key


def test_scripts_have_no_http_download_source():
    for name in ("deploy.py", "build_portable.py"):
        text = (ROOT / "scripts" / name).read_text("utf-8")
        assert "http://" not in text, f"{name} 出现 http:// 明文下载源"


def test_mirror_proxy_is_gated_and_verified():
    """加速代理源必须默认关闭（LS_ALLOW_MIRROR_PROXY=1 才启用）且同样过 SHA256。"""
    deploy_text = (ROOT / "scripts" / "deploy.py").read_text("utf-8")
    assert "LS_ALLOW_MIRROR_PROXY" in deploy_text
    assert "verify_sha256(dest, expected)" in deploy_text
    lock = json.loads(LOCK.read_text("utf-8"))
    for entry in lock["assets"].values():
        for url in entry.get("mirror_urls", []):
            assert url.startswith("https://"), url
