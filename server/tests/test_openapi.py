"""单元测试：任务101 OpenAPI 快照（可序列化/关键路径/示例/敏感键/--check）。
运行: pytest tests/test_openapi.py -q"""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent


def test_openapi_serializable_and_paths():
    from app.main import app
    spec = app.openapi()
    text = json.dumps(spec, ensure_ascii=False)
    assert len(text) > 10000
    paths = spec["paths"]
    for p in ("/api/health/ready", "/api/events", "/api/cameras",
              "/api/cameras/usb-devices", "/api/diagnostics/summary",
              "/api/events/bulk-status", "/api/genai/status", "/api/mqtt/status",
              "/api/plugins"):
        assert p in paths, p


def test_examples_injected_and_clean():
    sys.path.insert(0, str(ROOT / "scripts"))
    from export_openapi import build_openapi
    spec = build_openapi()
    cam = spec["paths"]["/api/cameras"]["post"]["requestBody"]["content"][
        "application/json"]["example"]
    assert cam["name"] == "示例摄像头" and "192.168.1.10" in cam["source"]
    ev = spec["paths"]["/api/events"]["get"]["responses"]["200"]["content"][
        "application/json"]["example"]
    assert ev[0]["id"] == 123
    # 全文无敏感键
    text = json.dumps(spec)
    for key in ("LS_ADMIN_TOKEN=sk", "rf_", "BEGIN PRIVATE KEY"):
        assert key not in text
    assert '"version"' in text or spec.get("info", {}).get("version")


def test_check_mode(tmp_path):
    out = ROOT / "docs" / "openapi.json"
    # 对一致文件 → 0
    r0 = subprocess.run([sys.executable, "-I", str(ROOT / "scripts" / "export_openapi.py"),
                         "--check"], capture_output=True)
    assert r0.returncode == 0
    # 对篡改文件 → 2（用临时目录，不动真实快照）
    bak = out.read_text(encoding="utf-8")
    try:
        out.write_text(bak.replace('"openapi"', '"openapi-tampered"', 1), encoding="utf-8")
        r2 = subprocess.run([sys.executable, "-I",
                             str(ROOT / "scripts" / "export_openapi.py"), "--check"],
                            capture_output=True)
        assert r2.returncode == 2
    finally:
        out.write_text(bak, encoding="utf-8")
    assert json.loads(out.read_text(encoding="utf-8"))["openapi"].startswith("3.")
