"""T-401 / LS-045：版本同步脚本——仓库根 VERSION 为唯一来源，同步更新
web/package.json、engine-rs/Cargo.toml 与 deploy/windows/installer.iss。

用法：python scripts/bump_version.py 0.5.0
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    ver = sys.argv[1].strip()
    if not ver or not ver[0].isdigit():
        print("[失败] 版本号必须以数字开头（SemVer）")
        return 2
    (ROOT / "VERSION").write_text(ver + "\n", encoding="ascii")

    pj = ROOT / "web" / "package.json"
    data = json.loads(pj.read_text(encoding="utf-8"))
    data["version"] = ver
    pj.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    cargo = ROOT / "engine-rs" / "Cargo.toml"
    ctext = cargo.read_text(encoding="utf-8")
    ctext2, n = re.subn(
        r'(\[workspace\.package\][^\[]*?version\s*=\s*")[^"]+(")',
        rf'\g<1>{ver}\g<2>',
        ctext,
        count=1,
        flags=re.S,
    )
    if n:
        cargo.write_text(ctext2, encoding="utf-8")

    iss = ROOT / "deploy" / "windows" / "installer.iss"
    if iss.is_file():
        itext = iss.read_text(encoding="utf-8-sig")
        itext2, n2 = re.subn(
            r'(#define\s+LsAppVersion\s+")[^"]+(")',
            rf'\g<1>{ver}\g<2>',
            itext,
            count=1,
        )
        if n2:
            iss.write_text(itext2, encoding="utf-8-sig")

    print(f"[OK] VERSION={ver}；web/package.json、engine-rs/Cargo.toml、installer.iss 已同步")
    return 0


if __name__ == "__main__":
    sys.exit(main())
