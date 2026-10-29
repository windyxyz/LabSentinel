"""单元测试：任务74 release_check.py（链接/密钥/必需文件，临时目录隔离）。"""
import importlib.util
import json
import os
import sys
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "release_check", os.path.join(os.path.dirname(__file__), "..", "..", "scripts",
                                  "release_check.py"))
rc_mod = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(rc_mod)


def make_repo(tmp_path: Path) -> Path:
    r = tmp_path / "repo"
    (r / "docs").mkdir(parents=True)
    (r / "server").mkdir()
    (r / "web").mkdir()
    (r / "README.md").write_text("[指南](docs/guide.md) 与 [外链](https://a.b) 与 [锚点](#x)", encoding="utf-8")
    (r / "docs" / "guide.md").write_text("内容", encoding="utf-8")
    (r / "docs" / "broken.md").write_text("[没了](nope.md) 与 ![图](missing.png)", encoding="utf-8")
    (r / "docs" / "fenced.md").write_text("```bash\n[假链接](nope2.md)\n```\n真内容", encoding="utf-8")
    (r / "server" / "app").mkdir()
    (r / "server" / "app" / "main.py").write_text('version="0.1.0-m1"', encoding="utf-8")
    (r / "server" / "app" / "api.py").write_text('return "0.1.0-m1"', encoding="utf-8")
    for f in ("LICENSE", "NOTICE", "server/.env.example", "server/requirements.txt",
              "web/package.json"):
        (r / f).write_text("x", encoding="utf-8")
    return r


def run_checks(root):
    old = rc_mod.ROOT
    rc_mod.ROOT = root
    try:
        res = []
        rc_mod.check_links(res)
        rc_mod.check_versions(res)
        rc_mod.check_secrets(res)
        rc_mod.check_required_files(res)
        return res
    finally:
        rc_mod.ROOT = old


def by_name(res, name):
    return next(x for x in res if x["name"] == name)


def test_links_pass_fail_and_ignores(tmp_path):
    r = make_repo(tmp_path)
    res = run_checks(r)
    links = by_name(res, "相对链接")
    assert links["status"] == "FAIL"
    assert "nope.md" in links["detail"] and "missing.png" in links["detail"]
    assert "nope2.md" not in links["detail"]  # 代码块内忽略
    assert "https" not in links["detail"]  # 外链忽略
    # 修好断链 → PASS
    (r / "docs" / "nope.md").write_text("x", encoding="utf-8")
    (r / "docs" / "missing.png").write_bytes(b"x")
    links2 = by_name(run_checks(r), "相对链接")
    assert links2["status"] == "PASS"


def test_versions_warn_on_doc_mismatch(tmp_path):
    r = make_repo(tmp_path)
    (r / "docs" / "release-notes-v0.1.0-alpha.md").write_text("# v0.9.9-beta notes", encoding="utf-8")
    res = run_checks(r)
    ver = by_name(res, "版本一致性")
    assert ver["status"] == "WARN" and "0.9.9" in ver["detail"]
    (r / "docs" / "release-notes-v0.1.0-alpha.md").write_text("# v0.1.0-alpha notes", encoding="utf-8")
    ver2 = by_name(run_checks(r), "版本一致性")
    assert ver2["status"] == "PASS"


def test_secrets_fail_and_allowlist(tmp_path):
    r = make_repo(tmp_path)
    (r / "server" / "app" / "leak.py").write_text('KEY = "sk-abcdefghijklmnopqrstuvwxyz123456"',
                                                  encoding="utf-8")
    res = run_checks(r)
    assert by_name(res, "敏感信息")["status"] == "FAIL"
    # allowlist 命中（示例占位）→ 不报
    (r / "server" / "app" / "leak.py").write_text('TOKEN = "your-secret-token-example"',
                                                  encoding="utf-8")
    assert by_name(run_checks(r), "敏感信息")["status"] == "PASS"


def test_required_files_fail(tmp_path):
    r = make_repo(tmp_path)
    (r / "LICENSE").unlink()
    assert by_name(run_checks(r), "必需文件")["status"] == "FAIL"
