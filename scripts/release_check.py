"""发布前终检（任务74）：文档-仓库一致性检查。

链接/敏感信息/TODO/必需文件为**纯只读**；`--zip` 存在时会刷新 dist/ 下它自己的输出产物
（SHA256SUMS.txt / RELEASE-MANIFEST.json，均为本工具链的生成物，不触碰源码）。

用法（标准库）:
    python scripts/release_check.py                # 全仓检查（无 zip 则跳过该项）
    python scripts/release_check.py --zip dist/LabSentinel-portable-20261029.zip --json

退出码：有 FAIL → 2；只有 WARN → 0。
"""
import argparse
import hashlib
import json
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# 密钥模式 → 说明；allowlist：测试/示例中的假值（按行匹配后豁免）
SECRET_PATTERNS = [
    (re.compile(r"\bsk-[A-Za-z0-9]{20,}"), "OpenAI 风格 key"),
    (re.compile(r"\bAKIA[0-9A-Z]{16}\b"), "AWS AccessKey"),
    (re.compile(r"\bghp_[A-Za-z0-9]{30,}\b"), "GitHub token"),
    (re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"), "私钥块"),
    (re.compile(r"\beyJ[A-Za-z0-9_-]{30,}\.eyJ"), "JWT"),
]
ALLOWLIST_LINE = re.compile(
    r"(your-secret-token|test-secret|tok60|tok65|your-hmac-secret|示例|占位|placeholder)",
    re.IGNORECASE)
LS_TOKEN_SET = re.compile(r"^#?\s*LS_ADMIN_TOKEN=.+$")

TEXT_SUFFIXES = {".md", ".py", ".js", ".ts", ".vue", ".bat", ".txt", ".yml", ".yaml",
                 ".json", ".html", ".css", ".example"}
SKIP_DIRS = {"dist", "node_modules", ".git", "m0-spike", "runs", "datasets", "logs",
             "效果演示", "Resources", "__pycache__", ".venv", "docs" + os.sep + "assets"}
LINK_RE = re.compile(r"(?<!\!)\[[^\]]*\]\(([^)\s]+)\)|!\[[^\]]*\]\(([^)\s]+)\)")


def log(m=""):
    print(m, flush=True)


def results_add(res, name, status, detail, advice=""):
    res.append({"name": name, "status": status, "detail": detail, "advice": advice})


def iter_text_files(root: Path):
    for dirpath, dirs, files in os.walk(root):
        rel_dir = Path(dirpath).relative_to(root)
        if any(part in SKIP_DIRS for part in rel_dir.parts):
            dirs[:] = []
            continue
        for f in files:
            p = Path(dirpath) / f
            if p.suffix.lower() in TEXT_SUFFIXES or f in ("LICENSE", "NOTICE", ".env.example"):
                yield p


def md_targets(root: Path):
    out = [root / "README.md"]
    docs = root / "docs"
    if docs.exists():
        out += sorted(docs.glob("*.md"))
        t = docs / "50-tasks"
        if t.exists():
            out += sorted(t.glob("*.md"))
    return [p for p in out if p.exists()]


def strip_code_blocks(text: str) -> str:
    out, in_fence = [], False
    for ln in text.splitlines():
        if ln.lstrip().startswith("```"):
            in_fence = not in_fence
            out.append("")
            continue
        out.append("" if in_fence else ln)
    return "\n".join(out)


def check_links(res) -> None:
    broken, ok_n = [], 0
    for md in md_targets(ROOT):
        base = md.parent
        text = strip_code_blocks(md.read_text(encoding="utf-8", errors="replace"))
        for m in LINK_RE.finditer(text):
            target = (m.group(1) or m.group(2) or "").strip()
            if not target or target.startswith(("http://", "https://", "mailto:", "#",
                                                "data:", "<")):
                continue
            path_part = target.split("#")[0].strip()
            if not path_part:
                continue  # 纯锚点
            dest = Path(path_part)
            if dest.is_absolute():
                broken.append((md, target, "绝对路径链接"))
                continue
            resolved = (base / dest)
            # docs 内互链常写 ../docs/x 或相对自身目录，两种解析位置都试
            if not resolved.exists():
                alt = ROOT / dest
                if not alt.exists():
                    broken.append((md, target, "目标不存在"))
                    continue
            ok_n += 1
    if broken:
        preview = "; ".join(f"{p.relative_to(ROOT)}→{t}" for p, t, _ in broken[:8])
        results_add(res, "相对链接", "FAIL",
                    f"断链 {len(broken)} 条（OK {ok_n}）：{preview}{'…' if len(broken) > 8 else ''}",
                    "修复或删除断链；docs/50-tasks 历史报告可标注")
    else:
        results_add(res, "相对链接", "PASS", f"{ok_n} 条全部可达")


def _extract(p: Path, patterns) -> list[str]:
    try:
        text = p.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return []
    out = []
    for pat in patterns:
        m = pat.search(text)
        if m:
            out.append(m.group(0))
    return out


def check_versions(res) -> None:
    ver_strings = set()
    main_py = _extract(ROOT / "server" / "app" / "main.py", [re.compile(r'version="([^"]+)"')])
    api_py = _extract(ROOT / "server" / "app" / "api.py", [re.compile(r'"(\d+\.\d+\.\d+-[A-Za-z0-9]+)"')])
    ver_strings.update(main_py, api_py)
    notes = ROOT / "docs" / "release-notes-v0.1.0-alpha.md"
    ann = ROOT / "docs" / "发布公告.md"
    note_vers = _extract(notes, [re.compile(r"v?\d+\.\d+\.\d+-[A-Za-z0-9]+")]) if notes.exists() else []
    ann_vers = _extract(ann, [re.compile(r"v?\d+\.\d+\.\d+-[A-Za-z0-9]+")]) if ann.exists() else []
    code_versions = {v for v in ver_strings if not v.startswith("v")}
    doc_versions = set(note_vers + ann_vers)
    if not code_versions:
        results_add(res, "版本一致性", "FAIL", "未在 main.py/api.py 中找到版本串", "检查 version 定义")
        return
    norm_doc = {v.lstrip("v") for v in doc_versions}
    allowed_alias = {"0.1.0-alpha"}  # 规格允许发布候选另名（任务74）
    mismatch = norm_doc - code_versions - allowed_alias
    if mismatch:
        results_add(res, "版本一致性", "WARN",
                    f"文档版本 {sorted(mismatch)} 与代码 {sorted(code_versions)} 不同"
                    "（发布候选另名属预期则忽略）", "确认公告/notes 的版本口径")
    else:
        results_add(res, "版本一致性", "PASS",
                    f"代码 {sorted(code_versions)}；文档 {sorted(doc_versions) or '（未写版本号）'}")


def check_secrets(res) -> None:
    hits = []
    for p in iter_text_files(ROOT):
        # 105 补丁 P1-3：本用例的假 sk- 串是测试夹具（"真密钥→FAIL"测试数据），豁免扫描；
        # 文件级豁免而非行级 allowlist，防止未来有人在测试里塞真密钥后绕过
        if p.name == "test_release_check.py":
            continue
        try:
            for i, ln in enumerate(p.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
                if ALLOWLIST_LINE.search(ln):
                    continue
                for pat, kind in SECRET_PATTERNS:
                    if pat.search(ln):
                        hits.append(f"{p.relative_to(ROOT)}:{i} [{kind}] {ln.strip()[:60]}")
                if LS_TOKEN_SET.match(ln) and "your-secret" not in ln and "#" not in ln.split("=")[0]:
                    val = ln.split("=", 1)[1].strip()
                    if val and val != "***":
                        hits.append(f"{p.relative_to(ROOT)}:{i} [LS_ADMIN_TOKEN 非空] {val[:6]}***")
        except OSError:
            continue
    if hits:
        results_add(res, "敏感信息", "FAIL", f"{len(hits)} 处：" + "；".join(hits[:5]),
                    "立即移除/轮换；确认为测试假值则加 allowlist")
    else:
        results_add(res, "敏感信息", "PASS", "未发现密钥模式（allowlist 已豁免示例占位）")


def check_todos(res) -> None:
    found = []
    for p in iter_text_files(ROOT):
        if p.name == "release_check.py":
            continue  # 本脚本文档串含 TODO 字样，排除自身
        if p.suffix not in (".py", ".js", ".vue", ".ts"):
            continue
        try:
            for i, ln in enumerate(p.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
                if re.search(r"\b(TODO|FIXME|XXX)\b", ln) and "noqa" not in ln:
                    found.append(f"{p.relative_to(ROOT)}:{i}")
        except OSError:
            continue
    results_add(res, "TODO/FIXME", "WARN" if found else "PASS",
                f"{len(found)} 处：" + ", ".join(found[:8]) + ("…" if len(found) > 8 else ""),
                "发布前逐条判断（允许合理遗留）" if found else "")


def check_required_files(res) -> None:
    required = ["LICENSE", "NOTICE", "README.md", "server/.env.example",
                "server/requirements.txt", "web/package.json",
                "docs/55-handoff.md", "docs/运维手册.md", "docs/发布核对报告.md"]
    missing = [f for f in required if not (ROOT / f).exists()]
    if missing:
        results_add(res, "必需文件", "FAIL", "缺：" + ", ".join(missing), "补齐后重跑")
    else:
        results_add(res, "必需文件", "PASS", f"{len(required)} 项齐全")


def check_zip(res, zip_path: Path | None) -> None:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import release_manifest as rm
    zp = zip_path or rm.find_latest_zip(ROOT / "Resources" / "dist")
    if zp is None:
        results_add(res, "便携包 manifest", "WARN", "Resources/dist/ 下没有 zip（未构建或未带 --zip）", "先构建")
        return
    rc = rm.build_manifest(zp, ROOT / "Resources" / "dist")  # 幂等：重算 sha 并刷新 manifest
    info = json.loads((ROOT / "Resources" / "dist" / "RELEASE-MANIFEST.json").read_text(encoding="utf-8"))
    status = "PASS" if rc == 0 else "FAIL"
    results_add(res, "便携包 manifest", status,
                f"{zp.name} sha256={info['sha256'][:12]}… 缺必需={len(info['missing_required'])} "
                f"缺可选={info['missing_optional']}",
                "必需条目缺失按 58 报告处理" if rc != 0 else "")


def main() -> int:
    global ROOT
    ap = argparse.ArgumentParser(description="发布前终检（任务74，只读）")
    ap.add_argument("--root", type=Path, default=ROOT)
    ap.add_argument("--zip", type=Path, help="便携包 zip（缺省自动取 dist/ 最新）")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()
    ROOT = a.root
    res: list[dict] = []
    check_links(res)
    check_versions(res)
    check_secrets(res)
    check_todos(res)
    check_required_files(res)
    check_zip(res, a.zip)
    if a.json:
        print(json.dumps(res, ensure_ascii=False, indent=2))
    else:
        log("== LabSentinel 发布终检 ==")
        for r in res:
            mark = {"PASS": "✓", "WARN": "⚠", "FAIL": "✗"}[r["status"]]
            log(f" [{mark}] {r['status']:<4} {r['name']}  {r['detail']}")
            if r["advice"]:
                log(f"        ↳ {r['advice']}")
    has_fail = any(r["status"] == "FAIL" for r in res)
    return 2 if has_fail else 0


if __name__ == "__main__":
    sys.exit(main())
