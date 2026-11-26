"""核对 Rust 控制面与 OpenAPI 登记的 HTTP 方法和路径。

用法（仓库根）:
    python -I scripts/rust_route_coverage.py                     # 打印覆盖率与缺口
    python -I scripts/rust_route_coverage.py --exe <labsentinel-server>
    python -I scripts/rust_route_coverage.py --strict           # 文档与程序完全一致

- 文档来自 docs/openapi.json，运行程序提供 `--dump-routes`。
- 按 HTTP 方法比较路径。OpenAPI 的 {id} 与 axum 的 :id 归一化为同一段。
- WebSocket 的 GET 握手同样参与两侧登记的对账。
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OPENAPI = ROOT / "docs" / "openapi.json"


def _norm_segment(seg: str) -> str:
    if seg.startswith(":") or (seg.startswith("{") and seg.endswith("}")):
        return "{param}"
    return seg


def norm_path(path: str) -> str:
    parts = [_norm_segment(s) for s in path.split("/") if s]
    # 去掉 /api 前缀差异：两侧行为不同时统一保留（openapi 是 /api/...，axum nest /api）
    return "/" + "/".join(parts)


def openapi_routes() -> dict[str, list[str]]:
    spec = json.loads(OPENAPI.read_text(encoding="utf-8"))
    out: dict[str, list[str]] = {}
    for path, methods in spec.get("paths", {}).items():
        for m in methods:
            if m.upper() in ("GET", "POST", "PUT", "PATCH", "DELETE"):
                out.setdefault(norm_path(path), []).append(m.upper())
    return out


def rust_routes(exe: Path) -> set[tuple[str, str]]:
    r = subprocess.run([str(exe), "--dump-routes"], capture_output=True, text=True,
                       timeout=30, env={**os.environ, "LS_LOG_FILE": "0"},
                       creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    if r.returncode != 0:
        raise SystemExit(f"[coverage] --dump-routes 失败: {r.stderr[-300:]}")
    data = json.loads(r.stdout)
    out = set()
    for route in data.get("routes", []):
        out.add((route["method"].upper(), norm_path(route["path"])))
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description="Rust 控制面路由覆盖率（R2-307）")
    ap.add_argument("--exe", type=Path, default=None,
                    help="labsentinel-server 可执行文件（默认 CARGO_TARGET_DIR/debug）")
    ap.add_argument("--fail-under", type=float, default=None,
                    help="文档路由的实现率低于该值时退出 1")
    ap.add_argument("--strict", action="store_true",
                    help="任一方法和路径只存在于一侧时退出 1")
    a = ap.parse_args()
    exe = a.exe
    if exe is None:
        target = Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "engine-rs" / "target")))
        exe = target / "debug" / ("labsentinel-server.exe" if os.name == "nt" else "labsentinel-server")
    if not exe.is_file():
        print(f"[coverage] 未找到 {exe}（先 cargo build -p ls-server）")
        return 2
    want = {(m, p) for p, methods in openapi_routes().items() for m in methods}
    got = rust_routes(exe)
    total = len(want)
    hit = len(want & got)
    missing = sorted(f"{m} {p}" for m, p in want - got)
    extra = sorted(f"{m} {p}" for m, p in got - want)
    pct = round(hit * 100.0 / total, 1) if total else 0.0
    print(f"Rust 已实现的 OpenAPI 路由 {hit}/{total} = {pct}%")
    print(f"OpenAPI 已登记的 Rust 路由 {hit}/{len(got)}")
    for title, rows in (("缺少实现", missing), ("缺少文档", extra)):
        print(f"{title} {len(rows)} 条")
        for row in rows:
            print(f"  - {row}")
    if a.strict and (missing or extra or not want):
        return 1
    if a.fail_under is not None and pct < a.fail_under:
        print(f"[coverage] FAIL：{pct}% < {a.fail_under}%")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
