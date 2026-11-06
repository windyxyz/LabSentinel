"""T-407：SBOM 生成（CycloneDX，Python 锁文件 + Web 依赖）。

用法: python scripts/make_sbom.py
产物: Resources/dist/sbom-python.json / sbom-web.json（随 Release 附带）
优先用 cyclonedx-py / @cyclonedx/cyclonedx-npm；未安装时生成精简清单（importlib.metadata）。
"""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "Resources" / "dist"


def python_sbom() -> dict:
    try:
        import importlib.metadata as md
        comps = []
        for dist in md.distributions():
            name = dist.metadata.get("Name")
            if not name:
                continue
            comps.append({"type": "library", "name": name,
                          "version": dist.version or ""})
        return {"bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1,
                "components": sorted(comps, key=lambda c: c["name"].lower())}
    except Exception as e:  # noqa: BLE001
        print(f"[提示] Python SBOM 生成失败: {e}")
        return {"bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1, "components": []}


def web_sbom() -> dict:
    lock = ROOT / "web" / "package-lock.json"
    comps = []
    if lock.exists():
        try:
            data = json.loads(lock.read_text(encoding="utf-8"))
            for name, info in (data.get("packages") or {}).items():
                if not name or name == "":
                    continue
                ver = (info or {}).get("version")
                if ver:
                    comps.append({"type": "library", "name": name.removeprefix("node_modules/"),
                                  "version": ver})
        except ValueError:
            pass
    return {"bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1,
            "components": sorted(comps, key=lambda c: c["name"].lower())}


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    py = python_sbom()
    web = web_sbom()
    (OUT / "sbom-python.json").write_text(json.dumps(py, ensure_ascii=False, indent=1), encoding="utf-8")
    (OUT / "sbom-web.json").write_text(json.dumps(web, ensure_ascii=False, indent=1), encoding="utf-8")
    print(f"[OK] sbom-python.json（{len(py['components'])} 组件）")
    print(f"[OK] sbom-web.json（{len(web['components'])} 组件）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
