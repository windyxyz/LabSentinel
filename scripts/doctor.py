"""便携包/源码 自检工具（任务70）：一条命令拿到"哪项坏了、怎么修"。全部只读。

用法（标准库，任意 Python 3.10+）:
    python scripts/doctor.py                      # 自动探测：runtime/python.exe 存在=便携包模式
    python scripts/doctor.py --root <包根/项目根> --port 8000 --json

检查项：运行时/程序文件/模型/媒体工具/配置/数据库/端口/磁盘/写权限。
退出码：有 FAIL → 2；只有 WARN 或全 PASS → 0。不打印任何密钥/令牌/RTSP 密码。
"""
import argparse
import json
import os
import shutil
import socket
import sqlite3
import sys
import tempfile
import time
from pathlib import Path

MIN_MODEL_BYTES = 1_000_000  # 测试可 monkeypatch（假模型文件用小体积）
MIN_DISK_GB = 5.0


def log(m=""):
    print(m, flush=True)


def detect_mode(root: Path) -> str:
    return "portable" if (root / "runtime" / "python.exe").exists() else "source"


def check(name, ok, detail="", advice="", level=None):
    """level: PASS/WARN/FAIL；ok=True→PASS，否则用 level（默认 FAIL）。"""
    status = "PASS" if ok else (level or "FAIL")
    return {"name": name, "status": status, "detail": detail, "advice": advice}


def _ro_uri(path) -> str:
    """与 server/app/store.ro_uri 保持一致（任务76）：as_uri 编码空格/中文/# 等特殊字符。"""
    return Path(path).resolve().as_uri() + "?mode=ro"


def run_checks(root: Path, port: int) -> list[dict]:
    mode = detect_mode(root)
    results = []

    def first(*paths):
        for p in paths:
            if p and Path(p).exists():
                return Path(p)
        return None

    server_main = first(root / "app" / "server" / "app" / "main.py", root / "server" / "app" / "main.py")
    web_index = first(root / "app" / "web" / "index.html", root / "web" / "dist" / "index.html")
    models = first(root / "models", root / "m0-spike")
    ppe = first((models or root) / "yolo11n_ppe.onnx", (models or root) / "yolo11n.onnx")
    classes = first((models or root) / "yolo11n_ppe.classes.txt", (models or root) / "yolo11n.classes.txt")
    data_root = Path(os.environ.get("LOCALAPPDATA", "")) / "LabSentinel" \
        if os.environ.get("LOCALAPPDATA") else root / "_userdata" / "LabSentinel"
    data_dir = data_root / "data"
    db = first(data_dir / "labsentinel.db", root / "server" / "data" / "labsentinel.db")

    # 1 运行时
    if mode == "portable":
        ok = (root / "runtime" / "python.exe").exists() and (root / "runtime" / "pythonw.exe").exists()
        results.append(check("运行时(runtime)", ok, f"mode=portable root={root}",
                             "缺 runtime——重新解压便携包或重新构建" if not ok else ""))
    else:
        ok = sys.version_info >= (3, 10)
        results.append(check("Python 版本", ok, f"{sys.version.split()[0]}（源码模式）",
                             "需要 Python 3.10+" if not ok else ""))
    # 2 程序文件
    miss = [str(p) for p in (server_main, web_index) if p is None]
    results.append(check("程序文件", not miss,
                         f"server_main={'√' if server_main else '×'} web_dist={'√' if web_index else '×'}",
                         "缺 " + ", ".join(miss) + "——源码模式先 npm run build；便携包重新解压" if miss else ""))
    # 3 模型
    if ppe is None:
        results.append(check("模型文件", False, "未找到 yolo11n_ppe.onnx / yolo11n.onnx",
                             "把模型放到 models/（便携包）或 m0-spike/（源码），见 docs/45"))
    else:
        size_ok = ppe.stat().st_size > MIN_MODEL_BYTES
        cls_ok = classes is not None and len(classes.read_text(encoding="utf-8", errors="replace").splitlines()) > 0
        results.append(check("模型文件", size_ok and cls_ok,
                             f"{ppe.name} {ppe.stat().st_size / 1e6:.1f}MB classes={'√' if cls_ok else '×'}",
                             "模型过小或词表缺失——重新下载/导出模型（docs/45）" if not (size_ok and cls_ok) else ""))
    # 4 媒体/工具
    media_ok = any((root / "media").glob("*.mp4")) if (root / "media").exists() else \
        any((root / "m0-spike" / "data").glob("*.mp4"))
    results.append(check("演示媒体", media_ok, "media/*.mp4" if media_ok else "无",
                         "缺失只影响首启播种演示（可忽略）" if not media_ok else "", level="WARN"))
    ffmpeg = first(root / "tools" / "ffmpeg" / "ffmpeg.exe", root / "tools" / "ffmpeg.exe")
    go2rtc = first(root / "tools" / "go2rtc.exe")
    tools_ok = ffmpeg is not None and go2rtc is not None
    results.append(check("工具(ffmpeg/go2rtc)", tools_ok,
                         f"ffmpeg={'√' if ffmpeg else '×'} go2rtc={'√' if go2rtc else '×'}",
                         "缺失会影响 USB 枚举/实时画面/视频转码（可后补）" if not tools_ok else "", level="WARN"))
    # 5 配置（只输出"已设置/未设置"，绝不输出值）
    # 任务119：按模式选配置——源码模式服务端读的是 root/server/.env；若优先读
    # %LOCALAPPDATA% 的旧便携 config.env（含指向已删除便携目录的绝对路径），
    # 会把 LS_MODEL_FP32 等判成不存在 → 假 FAIL。便携模式才以 config.env 为准。
    if mode == "portable":
        cfg = first(data_root / "config.env", root / "server" / ".env")
    else:
        cfg = first(root / "server" / ".env", data_root / "config.env")
    env_lines = {}
    if cfg and cfg.exists():
        for ln in cfg.read_text(encoding="utf-8", errors="replace").splitlines():
            ln = ln.strip()
            if ln.startswith("LS_") and "=" in ln:
                k, v = ln.split("=", 1)
                env_lines[k.strip()] = v.strip()
    bad_paths = []
    base = root / "app" / "server" if mode == "portable" else root / "server"
    for key in ("LS_WEB_DIST", "LS_PPE_MODEL", "LS_MODEL_FP32"):
        v = env_lines.get(key)
        if not v:
            continue
        p = Path(v)
        if not p.is_absolute():
            p = base / v  # 相对路径按 server 目录解析（与运行时一致）
        if not p.exists():
            bad_paths.append(key)
    results.append(check("配置", not bad_paths,
                         f"config={'√' if cfg else '×'} LS_ADMIN_TOKEN={'已设置' if env_lines.get('LS_ADMIN_TOKEN') else '未设置'}",
                         "配置里指向不存在文件的键：" + ", ".join(bad_paths) if bad_paths else ""))
    # 6 数据库
    if db is None:
        results.append(check("数据库", True, "尚无数据库（首次启动自动创建）", "", level="PASS"))
    else:
        try:
            con = sqlite3.connect(_ro_uri(db), uri=True)
            integrity = con.execute("PRAGMA integrity_check").fetchone()[0]
            cams = con.execute("select count(*) from cameras").fetchone()[0]
            evs = con.execute("select count(*) from events").fetchone()[0]
            con.close()
            ok = integrity == "ok"
            wal = Path(str(db) + "-wal")
            results.append(check("数据库", ok,
                                 f"{db.name} {db.stat().st_size / 1e6:.1f}MB integrity={integrity} "
                                 f"cameras={cams} events={evs} wal={wal.stat().st_size if wal.exists() else 0}B",
                                 "数据库损坏——用 scripts/backup_data.py 的最新备份恢复" if not ok else ""))
        except sqlite3.OperationalError as e:
            # 2026-10-30 审阅：服务正在写库时 integrity_check 会撞 busy——这是"忙"不是"坏"，
            # 原实现误报 FAIL（退出码 2）并建议删库重播种，纯属误导
            if "locked" in str(e).lower() or "busy" in str(e).lower():
                results.append(check("数据库", True, "数据库被占用（服务正在运行），跳过完整性检查",
                                     "", level="PASS"))
            else:
                results.append(check("数据库", False, f"打不开: {e}",
                                     "数据库损坏——用 scripts/backup_data.py 的最新备份恢复"))
        except sqlite3.Error as e:
            results.append(check("数据库", False, f"打不开: {e}",
                                 "数据库损坏——用最新备份恢复或删除该库重新播种（会丢事件）"))
    # 7 端口
    s = socket.socket()
    try:
        s.bind(("127.0.0.1", port))
        results.append(check(f"端口 {port}", True, "可绑定"))
    except OSError as e:
        results.append(check(f"端口 {port}", False, f"被占用: {e}",
                             "可能已有 LabSentinel 实例在跑——先访问该端口确认，或用 启动.bat 的自动换端口",
                             level="WARN"))
    finally:
        s.close()
    # 8 磁盘
    try:
        free_gb = shutil.disk_usage(str(root)).free / 1e9
        results.append(check("磁盘空间", free_gb >= MIN_DISK_GB, f"free {free_gb:.1f}GB",
                             "低于 5GB——清理磁盘，服务端会在低于阈值时自动紧急清理旧事件" if free_gb < MIN_DISK_GB else "",
                             level="WARN"))
    except OSError as e:
        results.append(check("磁盘空间", False, str(e), "", level="WARN"))
    # 9 写权限
    try:
        probe_dir = (data_dir if data_dir.exists() else root)
        fd, name = tempfile.mkstemp(prefix="doctor-", dir=str(probe_dir))
        os.close(fd)  # 不关句柄 Windows 下无法删除
        Path(name).unlink()
        results.append(check("写权限", True, f"{probe_dir} 可写"))
    except OSError as e:
        results.append(check("写权限", False, f"{data_dir} 不可写: {e}",
                             "检查目录权限/杀软拦截"))
    return results


def main() -> int:
    ap = argparse.ArgumentParser(description="LabSentinel 自检（任务70，只读）")
    ap.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    ap.add_argument("--port", type=int, default=8000)
    ap.add_argument("--json", action="store_true", help="机器可读输出")
    a = ap.parse_args()
    results = run_checks(a.root, a.port)
    if a.json:
        print(json.dumps({"mode": detect_mode(a.root), "results": results,
                          "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S")},
                         ensure_ascii=False, indent=2))
    else:
        log(f"== LabSentinel 自检（{detect_mode(a.root)} 模式）root={a.root} ==")
        for r in results:
            mark = {"PASS": "✓", "WARN": "⚠", "FAIL": "✗"}[r["status"]]
            log(f" [{mark}] {r['status']:<4} {r['name']}  {r['detail']}")
            if r["advice"]:
                log(f"        ↳ {r['advice']}")
        n_fail = sum(1 for r in results if r["status"] == "FAIL")
        n_warn = sum(1 for r in results if r["status"] == "WARN")
        log(f"== 汇总: {len(results) - n_fail - n_warn} PASS / {n_warn} WARN / {n_fail} FAIL ==")
    return 2 if any(r["status"] == "FAIL" for r in results) else 0


if __name__ == "__main__":
    sys.exit(main())
