"""数据备份/恢复 CLI（任务62）：SQLite backup API 在线快照 + 媒体目录 + 校验 + 保守恢复。

用法（标准库）:
    python scripts/backup_data.py backup  --data-dir <data> --out <zip> [--no-media] [--force]
    python scripts/backup_data.py verify  --zip <zip>
    python scripts/backup_data.py restore --zip <zip> --data-dir <dir> [--no-media] [--force]

语义：
  backup —— sqlite3.Connection.backup() 热快照（运行中服务可直接备份，一致性由 SQLite 保证），
            打包 labsentinel.db + snapshots/videos/annotation_pool；manifest.json 记录哈希与计数。
  verify —— 只校验：manifest 可解析、db sha256 匹配、PRAGMA integrity_check、条目路径安全；不落盘 data。
  restore—— 先 verify；目标已有 db 未 --force 拒绝；--force 先把现有 db 转存 .bak-<ts>（保留最近 3 个）；
            只恢复包内条目，不删目标里多出的文件（绝不误删用户新数据）。
"""
import argparse
import hashlib
import json
import sqlite3
import sys
import time
import zipfile
from pathlib import Path

MEDIA_DIRS = ("snapshots", "videos", "annotation_pool")
TOOL_VERSION = "backup-1.0"
EXIT_OK, EXIT_FAIL, EXIT_INTEGRITY = 0, 1, 2


def log(m=""):
    print(m, flush=True)


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def _snapshot_db(src: Path, dst: Path) -> tuple[bool, str]:
    """热备份 src → dst 并做 integrity_check，返回 (ok, 文本)。"""
    src_con = sqlite3.connect(f"file:{src}?mode=ro", uri=True)
    try:
        dst_con = sqlite3.connect(dst)
        try:
            src_con.backup(dst_con)
            row = dst_con.execute("PRAGMA integrity_check").fetchone()
            ok = row and row[0] == "ok"
            return (bool(ok), row[0] if row else "no result")
        finally:
            dst_con.close()
    finally:
        src_con.close()


def _check_integrity_bytes(db_bytes: bytes) -> tuple[bool, str]:
    import tempfile
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        f.write(db_bytes)
        tmp = Path(f.name)
    try:
        con = sqlite3.connect(tmp)
        try:
            row = con.execute("PRAGMA integrity_check").fetchone()
            return (bool(row) and row[0] == "ok", row[0] if row else "no result")
        finally:
            con.close()
    finally:
        tmp.unlink(missing_ok=True)


def _safe_entry(name: str) -> bool:
    """zip 条目路径安全：拒绝绝对路径/盘符/.. 穿越。"""
    n = name.replace("\\", "/")
    if n.startswith("/") or len(n) > 1 and n[1] == ":":
        return False
    return ".." not in n.split("/")


def cmd_backup(a) -> int:
    data_dir = a.data_dir
    db = data_dir / "labsentinel.db"
    if not db.exists():
        log(f"[失败] 数据库不存在: {db}")
        return EXIT_FAIL
    if a.out.exists() and not a.force:
        log(f"[失败] {a.out} 已存在（--force 覆盖）")
        return EXIT_FAIL
    import tempfile
    snap = Path(tempfile.mkdtemp()) / "labsentinel.db"
    try:
        ok, detail = _snapshot_db(db, snap)
        if not ok:
            log(f"[失败] 快照 integrity_check 未过: {detail}")
            return EXIT_INTEGRITY
        db_bytes = snap.read_bytes()
        media: list[Path] = []
        if not a.no_media:
            for d in MEDIA_DIRS:
                base = data_dir / d
                if base.exists():
                    media.extend(p for p in base.rglob("*") if p.is_file())
        media_bytes = sum(p.stat().st_size for p in media)
        manifest = {
            "created_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            "source_data_dir": str(data_dir),
            "db_bytes": len(db_bytes),
            "db_sha256": sha256_bytes(db_bytes),
            "media_files": len(media),
            "media_bytes": media_bytes,
            "integrity_check": detail,
            "tool_version": TOOL_VERSION,
        }
        if a.out.parent:
            a.out.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(a.out, "w", zipfile.ZIP_DEFLATED) as z:
            z.writestr("manifest.json", json.dumps(manifest, ensure_ascii=False, indent=2))
            z.writestr("labsentinel.db", db_bytes)
            for p in media:
                rel = p.relative_to(data_dir).as_posix()
                z.write(p, f"media/{rel}")
        n_entries = 2 + len(media)
        total = a.out.stat().st_size
        log(f"[OK] 备份 {a.out}（{n_entries} 文件，zip {total / 1e6:.1f}MB；db {len(db_bytes) / 1e6:.1f}MB"
            f" + 媒体 {len(media)} 个/{media_bytes / 1e6:.1f}MB，integrity={detail}）")
        return EXIT_OK
    finally:
        import shutil
        shutil.rmtree(snap.parent, ignore_errors=True)


def _load_and_verify(zp: Path) -> tuple[dict, zipfile.ZipFile]:
    if not zp.exists():
        raise FileNotFoundError(f"zip 不存在: {zp}")
    zf = zipfile.ZipFile(zp)
    names = zf.namelist()
    bad = [n for n in names if not _safe_entry(n)]
    if bad:
        zf.close()
        raise ValueError(f"发现不安全条目路径: {bad[:5]}")
    if "manifest.json" not in names or "labsentinel.db" not in names:
        zf.close()
        raise ValueError("缺少 manifest.json 或 labsentinel.db")
    m = json.loads(zf.read("manifest.json"))
    for k in ("db_bytes", "db_sha256", "tool_version"):
        if k not in m:
            zf.close()
            raise ValueError(f"manifest 缺字段: {k}")
    db_bytes = zf.read("labsentinel.db")
    if len(db_bytes) != m["db_bytes"] or sha256_bytes(db_bytes) != m["db_sha256"]:
        zf.close()
        raise ValueError("db 大小/SHA256 与 manifest 不一致（zip 损坏或被篡改）")
    ok, detail = _check_integrity_bytes(db_bytes)
    if not ok:
        zf.close()
        raise ValueError(f"db integrity_check 失败: {detail}")
    return m, zf


def cmd_verify(a) -> int:
    try:
        m, zf = _load_and_verify(a.zip)
    except (ValueError, FileNotFoundError, zipfile.BadZipFile, json.JSONDecodeError) as e:
        log(f"[失败] {e}")
        return EXIT_FAIL
    media_n = sum(1 for n in zf.namelist() if n.startswith("media/"))
    zf.close()
    log(f"[OK] {a.zip.name}: db {m['db_bytes'] / 1e6:.1f}MB sha256 前缀 {m['db_sha256'][:12]}…"
        f" integrity={m.get('integrity_check')} 媒体条目 {media_n}（创建于 {m.get('created_at')}）")
    return EXIT_OK


def _prune_baks(db: Path, keep: int = 3) -> None:
    baks = sorted(db.parent.glob("labsentinel.db.bak-*"))
    for old in baks[:-keep] if len(baks) > keep else []:
        old.unlink(missing_ok=True)


def cmd_restore(a) -> int:
    try:
        m, zf = _load_and_verify(a.zip)
    except (ValueError, FileNotFoundError, zipfile.BadZipFile, json.JSONDecodeError) as e:
        log(f"[失败] {e}")
        return EXIT_FAIL
    data_dir = a.data_dir
    db = data_dir / "labsentinel.db"
    data_dir.mkdir(parents=True, exist_ok=True)
    if db.exists():
        if not a.force:
            log(f"[失败] 目标已有数据库 {db}（--force 覆盖，现有 db 会转存 .bak-<时间戳>）")
            zf.close()
            return EXIT_FAIL
        bak = db.with_name(f"labsentinel.db.bak-{time.strftime('%Y%m%d-%H%M%S')}")
        db.replace(bak)
        _prune_baks(db)
        log(f"  现有 db 已转存 {bak.name}")
    db.write_bytes(zf.read("labsentinel.db"))
    ok, detail = _check_integrity_bytes(db.read_bytes())
    if not ok:
        log(f"[失败] 恢复后 integrity_check 失败: {detail}")
        zf.close()
        return EXIT_INTEGRITY
    restored_media = 0
    if not a.no_media:
        for name in zf.namelist():
            if not name.startswith("media/") or name.endswith("/"):
                continue
            rel = name[len("media/"):]
            target = data_dir / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(zf.read(name))
            restored_media += 1
    zf.close()
    log(f"[OK] 恢复完成: db {m['db_bytes'] / 1e6:.1f}MB（integrity={detail}）"
        + (f" + 媒体 {restored_media} 个" if not a.no_media else "（--no-media 跳过媒体）")
        + f" → {data_dir}")
    log("  注：未删除目标中备份没有的文件（保守恢复）")
    return EXIT_OK


def main() -> int:
    ap = argparse.ArgumentParser(description="LabSentinel 数据备份/恢复（任务62）")
    sub = ap.add_subparsers(dest="cmd", required=True)

    b = sub.add_parser("backup", help="热备份数据目录到 zip")
    b.add_argument("--data-dir", type=Path, required=True)
    b.add_argument("--out", type=Path, required=True)
    b.add_argument("--no-media", action="store_true", help="只备份 db（跳过截图/视频/标注池）")
    b.add_argument("--force", action="store_true")

    v = sub.add_parser("verify", help="校验备份 zip（不落盘 data）")
    v.add_argument("--zip", type=Path, required=True)

    r = sub.add_parser("restore", help="从备份 zip 恢复到数据目录")
    r.add_argument("--zip", type=Path, required=True)
    r.add_argument("--data-dir", type=Path, required=True)
    r.add_argument("--no-media", action="store_true")
    r.add_argument("--force", action="store_true")

    a = ap.parse_args()
    return {"backup": cmd_backup, "verify": cmd_verify, "restore": cmd_restore}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
