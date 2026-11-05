"""T-401：版本同步脚本——仓库根 VERSION 为唯一来源，web/package.json 由它生成。

用法：python scripts/bump_version.py 0.3.0
（server/app 经 version.read_version 运行时读 VERSION；app/VERSION 由构建落包）
"""
import json
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
    print(f"[OK] VERSION={ver}；web/package.json 已同步")
    return 0


if __name__ == "__main__":
    sys.exit(main())
