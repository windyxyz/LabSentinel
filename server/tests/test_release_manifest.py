"""单元测试：任务58 发布产物清单 scripts/release_manifest.py（必需/可选条目、sha256、--verify）。
运行: pytest tests/test_release_manifest.py -q"""
import os
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "scripts"))

import pytest  # noqa: E402

import release_manifest as rm  # noqa: E402

REQUIRED_MIN = rm.REQUIRED  # 全部必需条目


def make_zip(path: Path, drop: tuple[str, ...] = ()) -> None:
    with zipfile.ZipFile(path, "w") as z:
        for name in REQUIRED_MIN:
            if name in drop:
                continue
            content = b"2099.01" if name == "app/VERSION" else b"x"
            z.writestr(f"LabSentinel/{name}", content)


def test_full_zip_manifest_ok(tmp_path):
    zp = tmp_path / "LabSentinel-portable-test.zip"
    make_zip(zp)
    out = tmp_path / "out"
    rc = rm.build_manifest(zp, out)
    assert rc == 0
    import json
    m = json.loads((out / "RELEASE-MANIFEST.json").read_text(encoding="utf-8"))
    assert m["zip"] == zp.name
    assert m["version"] == "2099.01"
    assert m["size_bytes"] == zp.stat().st_size
    assert m["missing_required"] == []
    assert "LabSentinel.exe" in m["missing_optional"]  # 测试包没编 exe → 可选缺失
    import hashlib
    assert m["sha256"] == hashlib.sha256(zp.read_bytes()).hexdigest()
    sums = (out / "SHA256SUMS.txt").read_text(encoding="ascii").strip()
    assert sums == f"{m['sha256']}  {zp.name}"


def test_missing_required_exit2(tmp_path):
    zp = tmp_path / "LabSentinel-portable-test.zip"
    make_zip(zp, drop=("app/server/app/main.py",))
    out = tmp_path / "out"
    rc = rm.build_manifest(zp, out)
    assert rc == 2
    import json
    m = json.loads((out / "RELEASE-MANIFEST.json").read_text(encoding="utf-8"))
    assert "app/server/app/main.py" in m["missing_required"]


def test_verify_ok_and_tampered(tmp_path):
    out = tmp_path / "out"
    out.mkdir()
    zp = out / "LabSentinel-portable-test.zip"  # verify 约定 manifest 与 zip 同目录
    make_zip(zp)
    assert rm.build_manifest(zp, out) == 0
    assert rm.verify(out) == 0
    # 篡改：往 zip 追加一个字节
    data = zp.read_bytes()
    zp.write_bytes(data + b"\x00")
    assert rm.verify(out) == 2


def test_find_latest_zip(tmp_path):
    (tmp_path / "LabSentinel-portable-20260101.zip").write_bytes(b"")
    newer = tmp_path / "LabSentinel-portable-20260102.zip"
    newer.write_bytes(b"")
    import os
    os.utime(tmp_path / "LabSentinel-portable-20260101.zip", (1, 1))
    assert rm.find_latest_zip(tmp_path) == newer
    assert rm.find_latest_zip(tmp_path / "nope") is None
