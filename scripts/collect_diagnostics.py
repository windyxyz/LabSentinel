"""一键诊断包（任务61）：收集可公开的脱敏诊断信息到 zip，用于报障支持。

用法（标准库）:
    python scripts/collect_diagnostics.py --out diag.zip [--data-dir server/data] [--log-lines 500] [--force]

打包：README.txt / system.txt / config.txt（脱敏）/ counts.txt / cameras.txt（脱敏 source）/
      logs/server.log.tail.txt（最后 N 行）。
绝不包含：数据库整库、截图、视频、.env 原文件、人脸数据。
"""
import argparse
import ctypes
import os
import re
import sqlite3
import subprocess
import sys
import zipfile
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
_SENSITIVE = re.compile(r"TOKEN|PASSWORD|SECRET|KEY", re.IGNORECASE)
_URL_CRED = re.compile(r"(?i)(\w[\w+.-]*://)([^/@\s:]+):([^@\s]+)@")


def redact_value(v: str) -> str:
    """值级脱敏：敏感键名 → ***；URL 凭证 user:pass@ → ***:***@。"""
    s = _URL_CRED.sub(r"\1***:***@", str(v))
    return s


def redact_line(key: str, value: str) -> str:
    if _SENSITIVE.search(key):
        return f"{key}=***"
    return f"{key}={redact_value(value)}"


def sanitize_source(src: str) -> str:
    """摄像头 source 只留 协议://主机（file 路径保留盘符后的文件名）。"""
    m = re.match(r"^([a-zA-Z][\w+.-]*)://(?:[^/@\s:]+:[^@\s]+@)?([^/\s]+)", str(src or ""))
    if m:
        return f"{m.group(1)}://{m.group(2)}"
    return "***" if src else ""


def sys_memory_bytes() -> int | None:
    class MEMORYSTATUSEX(ctypes.Structure):
        _fields_ = [("dwLength", ctypes.c_ulong), ("dwMemoryLoad", ctypes.c_ulong),
                    ("ullTotalPhys", ctypes.c_ulonglong), ("ullAvailPhys", ctypes.c_ulonglong),
                    ("ullTotalPageFile", ctypes.c_ulonglong), ("ullAvailPageFile", ctypes.c_ulonglong),
                    ("ullTotalVirtual", ctypes.c_ulonglong), ("ullAvailVirtual", ctypes.c_ulonglong),
                    ("ullAvailExtendedVirtual", ctypes.c_ulonglong)]
    try:
        st = MEMORYSTATUSEX()
        st.dwLength = ctypes.sizeof(MEMORYSTATUSEX)
        ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(st))
        return st.ullTotalPhys
    except Exception:  # noqa: BLE001
        return None


def git_commit() -> str:
    try:
        r = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=str(ROOT),
                           capture_output=True, text=True, timeout=10)
        return r.stdout.strip() if r.returncode == 0 else "(unknown)"
    except (OSError, subprocess.TimeoutExpired):
        return "(unknown)"


def main() -> int:
    ap = argparse.ArgumentParser(description="LabSentinel 一键诊断包（任务61）")
    ap.add_argument("--out", type=Path, default=Path("diag.zip"))
    ap.add_argument("--data-dir", type=Path, default=ROOT / "server" / "data")
    ap.add_argument("--log-lines", type=int, default=500)
    ap.add_argument("--force", action="store_true", help="覆盖已存在的 --out")
    a = ap.parse_args()
    if a.out.exists() and not a.force:
        print(f"[失败] {a.out} 已存在（--force 覆盖）")
        return 1

    data_dir = a.data_dir
    db = data_dir / "labsentinel.db"
    log_file = data_dir / "logs" / "server.log"

    system = [
        f"generated_at={datetime.now(timezone.utc).astimezone().isoformat(timespec='seconds')}",
        f"platform={sys.platform}",
        f"python={sys.version.split()[0]}",
        f"cpu_count={os.cpu_count()}",
        f"memory_total_bytes={sys_memory_bytes() or '(unknown)'}",
        f"git_commit={git_commit()}",
    ]
    # 生效配置：settings（pydantic-settings，含 .env 值）以 LS_<字段> 形式输出并脱敏
    config_lines = ["# 当前生效配置（脱敏；TOKEN/PASSWORD/SECRET/KEY 一律 ***，URL 凭证已抹除）"]
    try:
        sys.path.insert(0, str(ROOT / "server"))
        from app.config import settings  # noqa: E402
        for k, v in settings.model_dump().items():
            config_lines.append(redact_line(f"LS_{k.upper()}", str(v)))
    except Exception as e:  # noqa: BLE001
        config_lines.append(f"(settings 读取失败: {e})")

    counts_lines = []
    if db.exists():
        con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
        try:
            tables = [r[0] for r in con.execute(
                "select name from sqlite_master where type='table' and name not like 'sqlite_%'")]
            for t in tables:
                n = con.execute(f"select count(*) from {t}").fetchone()[0]
                counts_lines.append(f"{t}={n}")
        finally:
            con.close()
    else:
        counts_lines.append(f"(db 不存在: {db})")

    cams_lines = ["id\tname\ttype\tstatus\tsource"]
    if db.exists():
        con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
        try:
            for cid, name, typ, status, src in con.execute(
                    "select id,name,type,status,source from cameras"):
                cams_lines.append(f"{cid}\t{name}\t{typ}\t{status}\t{sanitize_source(src)}")
        except sqlite3.OperationalError as e:
            cams_lines.append(f"(查询失败: {e})")
        finally:
            con.close()

    if log_file.exists():
        tail = log_file.read_text(encoding="utf-8", errors="replace").splitlines()[-a.log_lines:]
        log_txt = "\n".join(tail) or "(空日志)"
    else:
        log_txt = f"(日志文件不存在: {log_file})"

    readme = """LabSentinel 诊断包内容说明（任务61）
- system.txt: 平台/Python/CPU/内存/版本/git
- config.txt: 生效配置（已脱敏：TOKEN/PASSWORD/SECRET/KEY=***，URL 凭证抹除）
- counts.txt: 数据库各表行数（只读计数，不含内容）
- cameras.txt: 摄像头列表（source 只含协议+主机，不含账号密码）
- logs/server.log.tail.txt: 服务日志最后 N 行
不含：数据库整库、截图、视频、.env 原文件、人脸数据。
"""
    entries = {
        "README.txt": readme,
        "system.txt": "\n".join(system) + "\n",
        "config.txt": "\n".join(config_lines) + "\n",
        "counts.txt": "\n".join(counts_lines) + "\n",
        "cameras.txt": "\n".join(cams_lines) + "\n",
        "logs/server.log.tail.txt": log_txt + "\n",
    }
    a.out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(a.out, "w", zipfile.ZIP_DEFLATED) as z:
        for name, text in entries.items():
            z.writestr(name, text)
    size = a.out.stat().st_size
    print(f"[OK] 诊断包 {a.out}（{len(entries)} 条目，{size / 1024:.1f}KB）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
