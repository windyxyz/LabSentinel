"""发布前终检（任务74）：文档-仓库一致性检查。

链接/敏感信息/TODO/必需文件为**纯只读**；`--zip` 存在时会刷新 dist/ 下它自己的输出产物
（SHA256SUMS.txt / RELEASE-MANIFEST.json，均为本工具链的生成物，不触碰源码）。

用法（标准库）:
    python scripts/release_check.py                # 全仓检查（无 zip 则跳过该项）
    python scripts/release_check.py --zip dist/LabSentinel-portable-20261029.zip --json

退出码：有 FAIL → 2；只有 WARN → 0。
"""
import argparse
import json
import os
import re
import sys
from datetime import datetime
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
             "效果演示", "Resources", "__pycache__", ".venv", "docs" + os.sep + "assets",
             # 训练 venv 误建到仓库根（.gitignore 已忽略；第三方代码的 TODO 不属于本项目）
             "LabSentinel"}
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
        # 2026-11-13 docs 重组后必须递归：guide/dev/releases/adr/plan 子目录
        # 曾完全不在扫描范围，「断链 0 FAIL」对子目录失真（一致性审查 H2）。
        # archive/ 是历史快照（旧链接按重组政策豁免），不参与扫描。
        out += sorted(p for p in docs.rglob("*.md")
                      if "archive" not in p.relative_to(docs).parts)
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
                    "修复或删除断链；已归档的历史报告可标注为纯文本")
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
    # T-401/T-409：版本单一来源 = 仓库根 VERSION；web/package.json 必须一致（单条结果判定）
    version = ""
    try:
        version = (ROOT / "VERSION").read_text("ascii").strip()
    except OSError:
        results_add(res, "版本一致性", "FAIL", "仓库根缺 VERSION（T-401 单一来源）",
                    "python scripts/bump_version.py <ver>")
        return
    problems: list[str] = []
    try:
        web_ver = json.loads((ROOT / "web" / "package.json").read_text(encoding="utf-8")).get("version")
        if web_ver != version:
            problems.append(f"web/package.json={web_ver} != {version}")
    except (OSError, ValueError):
        problems.append("web/package.json 读取/解析失败")

    ver_strings = set()
    main_py = _extract(ROOT / "server" / "app" / "main.py", [re.compile(r'version="([^"]+)"')])
    api_py = _extract(ROOT / "server" / "app" / "api.py", [re.compile(r'"(\d+\.\d+\.\d+-[A-Za-z0-9]+)"')])
    ver_strings.update(main_py, api_py)
    notes = ROOT / "docs" / "archive" / "release-notes-v0.1.0-alpha.md"
    ann = ROOT / "docs" / "archive" / "发布公告.md"
    note_vers = _extract(notes, [re.compile(r"v?\d+\.\d+\.\d+-[A-Za-z0-9]+")]) if notes.exists() else []
    ann_vers = _extract(ann, [re.compile(r"v?\d+\.\d+\.\d+-[A-Za-z0-9]+")]) if ann.exists() else []
    code_versions = {v for v in ver_strings if not v.startswith("v")}
    doc_versions = set(note_vers + ann_vers)
    norm_doc = {v.lstrip("v") for v in doc_versions}
    allowed_alias = {"0.1.0-alpha"}  # 规格允许发布候选另名（任务74）
    doc_mismatch = norm_doc - code_versions - allowed_alias
    if doc_mismatch:
        problems.append(f"文档版本 {sorted(doc_mismatch)} 与代码不同（确认公告口径）")
    if problems:
        results_add(res, "版本一致性", "FAIL" if any("package.json" in p for p in problems) else "WARN",
                    f"VERSION={version}；问题：{'；'.join(problems)}",
                    "scripts/bump_version.py <ver> 同步各处")
    else:
        results_add(res, "版本一致性", "PASS",
                    f"VERSION={version}（单源；main/api 运行时读取，"
                    f"文档 {sorted(doc_versions) or '未写版本号'}）")


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
    # 2026-11 文档清理：历史交接/核对报告已删除，必需文件只保留现行文档
    required = ["LICENSE", "NOTICE", "README.md", "server/.env.example",
                "server/requirements.txt", "web/package.json",
                "docs/guide/运维手册.md", "docs/guide/部署指南.md", "docs/archive/验证记录-S7-L1黄金语料对齐.md"]
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
    # ADR-0010：setup.exe 双形态（可选产物——本机无 ISCC 时缺省，WARN 不阻断）。
    # 实际命名是版本号（LabSentinel-setup-<ver>[-dml].exe，cpu 无后缀），不是 zip 的日期名。
    dist = zp.parent
    version = (ROOT / "VERSION").read_text("ascii").strip()
    setups = sorted(dist.glob(f"LabSentinel-setup-{version}*.exe"))
    setup = setups[0] if setups else dist / f"LabSentinel-setup-{version}.exe"
    if setup.is_file():
        sz = setup.stat().st_size / 1e6
        results_add(res, "安装包 setup.exe", "PASS", f"{setup.name} {sz:.1f}MB（ADR-0010 双形态）")
    else:
        results_add(res, "安装包 setup.exe", "WARN",
                    f"{setup.name} 不存在（--installer 未编译或 ISCC 缺失）",
                    "CI（choco install innosetup）或本地 Inno Setup 编译后重跑")


def check_t409(res) -> None:
    """T-409：发布门禁强化——manifest 来源、依赖锁 ASCII、无 http:// 下载源、
    NOTICE 覆盖许可证、签名产物核验提示。"""
    # 1) manifest git_commit == HEAD 且 git_dirty=false（有 manifest 时）
    manifest = ROOT / "Resources" / "dist" / "RELEASE-MANIFEST.json"
    if manifest.is_file():
        try:
            m = json.loads(manifest.read_text(encoding="utf-8"))
            head = _git_out(["rev-parse", "--short", "HEAD"])
            dirty = m.get("git_dirty", True)
            commit = str(m.get("git_commit", ""))
            ok = (not dirty) and bool(head) and commit.startswith(head)
            results_add(res, "manifest 来源（T-409）",
                        "PASS" if ok else "WARN",
                        f"git_commit={commit} HEAD={head} git_dirty={dirty}",
                        "发布产物须来自干净树且 commit 与 HEAD 一致" if not ok else "")
        except (OSError, ValueError) as e:
            results_add(res, "manifest 来源（T-409）", "WARN", f"manifest 解析失败: {e}", "")
    # 2) requirements*.txt/lock 纯 ASCII（GBK 机器 pip 解码兼容）
    bad_ascii = []
    for req in sorted((ROOT / "server").glob("requirements*.txt")) + \
            sorted((ROOT / "server").glob("requirements*.lock")):
        try:
            req.read_bytes().decode("ascii")
        except UnicodeDecodeError:
            bad_ascii.append(str(req.relative_to(ROOT)))
    results_add(res, "依赖锁 ASCII（T-409）",
                "FAIL" if bad_ascii else "PASS",
                "非 ASCII: " + ", ".join(bad_ascii) if bad_ascii else "全部 requirements/lock 纯 ASCII",
                "GBK 机器 pip 解码会崩，转纯 ASCII" if bad_ascii else "")
    # 3) 无 http:// 明文下载源（本机地址例外）
    http_hits = []
    for p in (ROOT / "scripts" / "assets.lock.json", ROOT / "scripts" / "fetch_assets.py",
              ROOT / "scripts" / "deploy.py"):
        if not p.is_file():
            continue
        for i, line in enumerate(p.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
            if re.search(r"[\"']http://(?!127\.0\.0\.1|localhost)", line):
                http_hits.append(f"{p.relative_to(ROOT)}:{i}")
    results_add(res, "下载源 HTTPS（T-409）",
                "FAIL" if http_hits else "PASS",
                "明文源: " + ", ".join(http_hits) if http_hits else "无 http:// 明文下载源",
                "改用 https" if http_hits else "")
    # 4) NOTICE 覆盖主要第三方组件许可证
    notice = ROOT / "NOTICE"
    if not notice.is_file():
        results_add(res, "NOTICE 覆盖（T-409）", "FAIL", "NOTICE 缺失", "补齐第三方声明")
    else:
        text = notice.read_text(encoding="utf-8", errors="replace")
        missing = [x for x in ("opencv", "onnxruntime", "fastapi", "supervision")
                   if x.lower() not in text.lower()]
        results_add(res, "NOTICE 覆盖（T-409）",
                    "WARN" if missing else "PASS",
                    "NOTICE 未提及: " + ", ".join(missing) if missing else "主要第三方组件均有声明",
                    "补齐 SBOM 组件许可证" if missing else "")
    # 5) 签名产物（证书办妥前草稿发布豁免 → WARN 不阻断）
    sums = ROOT / "Resources" / "dist" / "SHA256SUMS.txt"
    if sums.is_file():
        has_sig = (ROOT / "Resources" / "dist" / "SHA256SUMS.txt.minisig").is_file()
        results_add(res, "产物签名（T-409）", "PASS" if has_sig else "WARN",
                    "SHA256SUMS 已 minisign 签名" if has_sig else "SHA256SUMS 未签名（草稿发布可豁免）",
                    "证书就绪后 minisign -Sm" if not has_sig else "")
    # 6) Q-13 书面豁免：表内每行必须有到期日；过期豁免按 FAIL 阻断发布
    exc = ROOT / "SECURITY-EXCEPTIONS.md"
    if not exc.is_file():
        results_add(res, "漏洞豁免登记（Q-13）", "WARN", "SECURITY-EXCEPTIONS.md 缺失",
                    "按模板补书面豁免文件（当前无豁免）")
    else:
        today = datetime.now().date()
        rows = [ln for ln in exc.read_text(encoding="utf-8", errors="replace").splitlines()
                if ln.startswith("|") and "---" not in ln and "组件 / 版本" not in ln
                and "示例，勿留作真实豁免" not in ln]
        expired, malformed = [], []
        for ln in rows[1:]:  # 跳过表头
            cells = [c.strip() for c in ln.strip().strip("|").split("|")]
            if len(cells) < 8:
                malformed.append(ln[:60])
                continue
            try:
                due = datetime.strptime(cells[6], "%Y-%m-%d").date()
                if due < today:
                    expired.append(cells[0])
            except ValueError:
                malformed.append(cells[0][:40])
        if expired:
            results_add(res, "漏洞豁免登记（Q-13）", "FAIL",
                        "过期豁免: " + ", ".join(expired), "升级依赖或删除豁免行")
        elif malformed:
            results_add(res, "漏洞豁免登记（Q-13）", "WARN",
                        f"{len(malformed)} 行格式/到期日不完整", "补齐理由/受影响路径/到期日")
        else:
            results_add(res, "漏洞豁免登记（Q-13）", "PASS",
                        f"豁免表 {max(0, len(rows) - 1)} 行，均在有效期内")


def _git_out(args: list[str]) -> str:
    """只读 git 查询；非仓库/不可用时返回空串（调用方按 WARN 判定）。"""
    try:
        import subprocess
        r = subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True,
                           text=True, errors="replace", timeout=10)
        return r.stdout.strip() if r.returncode == 0 else ""
    except Exception:  # noqa: BLE001  只读查询失败不致命
        return ""


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
    check_t409(res)
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
