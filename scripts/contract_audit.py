# -*- coding: utf-8 -*-
"""LS-045: 前端-后端路由契约静态审计 + 安装器 BOM 门禁。

无需编译二进制即可在 CI web 作业运行：
1. 解析 `engine-rs/crates/ls-server/src/router.rs` 中的 `ROUTE_INVENTORY` 与 WS 路由；
2. 解析 `web/src/api.js` 中所有 `req(METHOD, '/api/...')` 与 `fetch('/api/...')` 调用；
3. 归一化路径参数后，断言前端调用的每一条 API 在后端均有对应 `(METHOD, PATH)`；
4. 检查 `deploy/windows/installer.iss` 必须带 UTF-8 BOM（防止 Inno Setup 7 中文乱码）。
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ROUTER_RS = ROOT / "engine-rs" / "crates" / "ls-server" / "src" / "router.rs"
API_JS = ROOT / "web" / "src" / "api.js"
INSTALLER_ISS = ROOT / "deploy" / "windows" / "installer.iss"


def norm_path(raw: str) -> str:
    path = raw.split("?", 1)[0].strip()
    segs = []
    for seg in path.split("/"):
        if not seg:
            continue
        if seg.startswith(":") or (seg.startswith("{") and seg.endswith("}")) or "${" in seg:
            segs.append("{param}")
        else:
            segs.append(seg)
    return "/" + "/".join(segs)


def parse_backend_routes() -> set[tuple[str, str]]:
    text = ROUTER_RS.read_text(encoding="utf-8")
    routes: set[tuple[str, str]] = set()
    for m, p in re.findall(r'\(\s*"(GET|POST|PUT|PATCH|DELETE)"\s*,\s*"([^"]+)"\s*\)', text):
        routes.add((m.upper(), norm_path(p)))
    # Also allow parameterized dynamic plugin action route: /api/plugins/:pid/:action
    if ("POST", "/api/plugins/{param}/enable") in routes:
        routes.add(("POST", "/api/plugins/{param}/{param}"))
    return routes


def parse_frontend_calls() -> list[tuple[str, str, str]]:
    text = API_JS.read_text(encoding="utf-8")
    calls: list[tuple[str, str, str]] = []
    # req('GET', '/api/...') or req('POST', `/api/...`)
    for m, raw in re.findall(r"req\(\s*['\"](GET|POST|PUT|PATCH|DELETE)['\"]\s*,\s*['\"`]([^'\"`]+)['\"`]", text):
        if raw.startswith("/api/"):
            calls.append((m.upper(), norm_path(raw), raw))
    # fetch('/api/...')
    for raw in re.findall(r"fetch\(\s*['\"`](/api/[^'\"`]+)['\"`]", text):
        calls.append(("GET", norm_path(raw), raw))
    return calls


def check_installer_bom() -> list[str]:
    if not INSTALLER_ISS.is_file():
        return [f"缺少安装脚本: {INSTALLER_ISS}"]
    raw = INSTALLER_ISS.read_bytes()
    if not raw.startswith(b"\xef\xbb\xbf"):
        return [f"{INSTALLER_ISS.relative_to(ROOT)} 缺少 UTF-8 BOM（Inno Setup 7 中文页面将乱码）"]
    return []


def main() -> int:
    backend = parse_backend_routes()
    frontend = parse_frontend_calls()
    errors: list[str] = []

    if not backend:
        errors.append("未能从 router.rs 解析出任何后端路由")
    if not frontend:
        errors.append("未能从 web/src/api.js 解析出任何前端 API 调用")

    seen: set[tuple[str, str]] = set()
    for method, np, raw in frontend:
        if (method, np) in seen:
            continue
        seen.add((method, np))
        if (method, np) not in backend:
            errors.append(f"前端调用未在后端 ROUTE_INVENTORY 中找到: {method} {raw} (norm={np})")

    errors.extend(check_installer_bom())

    if errors:
        print("[contract_audit] FAIL:")
        for e in errors:
            print(f"  - {e}")
        return 1

    print(
        f"[contract_audit] PASS: 前端 {len(seen)} 条唯一 API 全部匹配后端 "
        f"({len(backend)} 条路由)；installer.iss UTF-8 BOM 正常。"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
