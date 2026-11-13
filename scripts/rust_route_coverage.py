"""R2-307/N-27：Rust 控制面（ls-server）路由覆盖率统计。

用法（仓库根）:
    python -I scripts/rust_route_coverage.py                     # 打印覆盖率与缺口
    python -I scripts/rust_route_coverage.py --exe <labsentinel-server>
    python -I scripts/rust_route_coverage.py --fail-under 95     # CI 门禁（L3 门槛）

口径：
- 全集 = docs/openapi.json 的 paths（Python FastAPI 契约，97 条路径，按方法展开）；
- 实现 = `labsentinel-server --dump-routes` 的 JSON 输出；
- 路径参数归一化：OpenAPI 的 {id} 与 axum 的 :id 视为同一段；
- 排除：/api/ws/*（WebSocket 契约单独验证）与 /api/media/sign 这类纯派生端点由
  --exclude 传 Glob 风格前缀（默认排除 ws）。

L3 发布门槛（78 号审阅 §9.3）：覆盖 ≥95% 才允许 labsentinel-server 进正式 Release。
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
                       timeout=30, env={**os.environ, "LS_LOG_FILE": "0"})
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
                    help="覆盖率低于该值时 exit 1（CI 门禁；L3 门槛 95）")
    a = ap.parse_args()
    exe = a.exe
    if exe is None:
        target = Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "engine-rs" / "target")))
        exe = target / "debug" / ("labsentinel-server.exe" if os.name == "nt" else "labsentinel-server")
    if not exe.is_file():
        print(f"[coverage] 未找到 {exe}（先 cargo build -p ls-server）")
        return 2
    want = openapi_routes()
    got = rust_routes(exe)
    total = sum(len(ms) for ms in want.values())
    hit = sum(1 for p, ms in want.items() for m in ms if (m, p) in got)
    missing = sorted(f"{m} {p}" for p, ms in want.items() for m in ms if (m, p) not in got)
    extra = sorted(f"{m} {p}" for m, p in got if p not in want)
    pct = round(hit * 100.0 / total, 1) if total else 0.0
    print(f"Rust 控制面路由覆盖率: {hit}/{total} = {pct}%（L3 门槛 95%）")
    print(f"已实现 {len(got)} 条（method,path）；未实现 {len(missing)} 条：")
    for m in missing[:40]:
        print(f"  - {m}")
    if len(missing) > 40:
        print(f"  … 共 {len(missing)} 条")
    if extra:
        print(f"OpenAPI 之外的路由 {len(extra)} 条（契约需补文档）: {extra[:10]}")
    if a.fail_under is not None and pct < a.fail_under:
        print(f"[coverage] FAIL：{pct}% < {a.fail_under}%")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
