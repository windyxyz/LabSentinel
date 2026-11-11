"""生成 opentip.kaspersky.com 误报申诉材料（全量 SHA256 + 提交模板）。

用法: python scripts/opentip_hashes.py [--zip <便携包zip>]
     缺省取 Resources/dist/ 下最新便携包 zip（与 release_manifest 同口径）。

输出: docs/opentip-申诉材料-<版本>.md（重打包/发版后重跑即可更新哈希）。
"""
import argparse
import hashlib
import sys
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))

KEYS = [
    ("LabSentinel/LabSentinel.exe",
     "启动器（C# .NET Framework 4，winexe，本仓库 scripts/launcher_exe/LabSentinelLauncher.cs 编译）"),
    ("LabSentinel/runtime/python.exe",
     "CPython 3.12 官方 embed（python.org 发行，PSF Authenticode 签名）"),
    ("LabSentinel/runtime/pythonw.exe", "同上（无控制台变体）"),
    ("LabSentinel/scripts/portable_launcher.py",
     "启动器脚本（标准库：uvicorn 子进程托管 + 回环健康检查）"),
    ("LabSentinel/tools/go2rtc.exe",
     "go2rtc 官方发行（MIT，github.com/AlexxIT/go2rtc）"),
    ("LabSentinel/tools/ffmpeg.exe", "ffmpeg（GPL/LGPL 构建，BtbN 发行）"),
    ("LabSentinel/bin/ls-analyzer.exe",
     "本项目 Rust 分析器（AGPL-3.0，engine-rs 源码构建）"),
    ("LabSentinel/bin/onnxruntime.dll",
     "ONNX Runtime 官方发行（MIT，DirectML EP 版）"),
]

REASON = """## 申诉理由（可直接粘贴）

LabSentinel 是开源（AGPL-3.0）本地实验室安全监控软件：摄像头画面经本地推理
（ONNX Runtime）检测未穿实验服等违规并产生告警，全部数据留在本机。
便携包的运行形态为「启动器 EXE → runtime\\pythonw.exe 托管本地 uvicorn 服务
（仅监听 127.0.0.1）→ 拉起 go2rtc.exe 做 WebRTC 网关」。我们理解该行为组合
会命中启发式（本地监听 + 拉起第三方二进制 + 无签名启动器），但：
1) 启动链无动态代码执行（无 -c/exec/runpy），唯一的 urlopen 是对 127.0.0.1 的
   健康检查；2) Python 运行时为 python.org 官方签名发行；3) 全部源码开源可审计：
   https://github.com/Windy-LabSentinel/LabSentinel
实测 Kaspersky 会在构建产物目录（含 runtime\\python.exe 等官方签名文件）上触发
「HEUR:Trojan」类判定并静默删除文件。请复核以下文件并加白。"""


def latest_zip() -> Path:
    import release_manifest as rm
    zp = rm.find_latest_zip(ROOT / "Resources" / "dist")
    if zp is None:
        raise SystemExit("[失败] Resources/dist/ 下没有便携包 zip（先 build_portable.py --zip）")
    return zp


def main() -> int:
    ap = argparse.ArgumentParser(description="opentip 误报申诉材料生成器")
    ap.add_argument("--zip", type=Path, default=None)
    a = ap.parse_args()
    zp = a.zip or latest_zip()
    version = (ROOT / "VERSION").read_text("ascii").strip()

    z = zipfile.ZipFile(zp)
    names = z.namelist()
    lines = [
        f"# Kaspersky Open False Positive 申诉材料（LabSentinel {version} 便携包）",
        "",
        "提交地址：https://opentip.kaspersky.com/（Submit False Positive / False Alarm）",
        "",
        REASON,
        "",
        "## 文件与 SHA256（完整）",
        "",
    ]
    missing = []
    for key, desc in KEYS:
        if key not in names:
            missing.append(key)
            continue
        data = z.read(key)
        h = hashlib.sha256(data).hexdigest()
        lines.append(f"- `{key.split('/', 1)[1]}`  \n  SHA256: `{h}`  \n"
                     f"  大小: {len(data)} 字节  \n  说明: {desc}")
    z.close()
    lines += [
        "",
        "## 附加信息",
        "",
        f"- 版本：{version}（未签名预览版；代码签名证书在办，到位后随版签名）",
        "- 行为白皮书：仓库内 docs/杀毒软件误报说明.md（特征对照表）",
        f"- 来源 zip：{zp.name}",
        "- 联系：维护者（见仓库 README）",
    ]
    out = ROOT / "docs" / f"opentip-申诉材料-{version}.md"
    out.write_text("\n".join(lines), encoding="utf-8")
    print(f"[OK] {out}")
    if missing:
        print(f"[提示] zip 中缺失（跳过）: {missing}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
