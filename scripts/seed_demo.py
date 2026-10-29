"""演示数据播种单源模块（任务51）：deploy.py 与 portable_launcher.py 共用，消除两边漂移。

只用标准库。schema 必须与 server/app/store.py 保持同步（历史约定：deploy.py 注释同款）。

用法:
    from seed_demo import seed_database
    n = seed_database(db_path, media_dir)   # 返回插入的摄像头数（空库才播种，否则 0）

自测:
    python scripts/seed_demo.py --selftest  # 临时库：4 路/区域/规则 + 幂等 + 用户数据不覆盖
"""
import sqlite3
from pathlib import Path

# 与 server/app/store.py 的 SCHEMA 保持同步（cameras/zones/rules/events/face_consents/alert_deliveries）
SCHEMA = """
CREATE TABLE IF NOT EXISTS cameras(
  id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, source TEXT, type TEXT, status TEXT DEFAULT 'stopped');
CREATE TABLE IF NOT EXISTS zones(
  id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, name TEXT, polygon TEXT);
CREATE TABLE IF NOT EXISTS rules(
  id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, zone_id INTEGER,
  behavior TEXT, level INTEGER, cooldown_s INTEGER,
  time_start TEXT, time_end TEXT, threshold INTEGER, enabled INTEGER);
CREATE TABLE IF NOT EXISTS events(
  id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT, camera_id INTEGER, type TEXT,
  level INTEGER, track_id INTEGER, detail TEXT, snapshot TEXT, video TEXT,
  status TEXT DEFAULT 'new', handled_by TEXT, handled_ts TEXT);
CREATE TABLE IF NOT EXISTS face_consents(
  subject TEXT PRIMARY KEY, ts TEXT);
CREATE TABLE IF NOT EXISTS alert_deliveries(
  id INTEGER PRIMARY KEY AUTOINCREMENT, event_id INTEGER, channel TEXT,
  status TEXT DEFAULT 'pending', attempts INTEGER DEFAULT 0,
  last_error TEXT, updated_ts TEXT);
CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts);
CREATE INDEX IF NOT EXISTS idx_events_camera ON events(camera_id);
"""

# (摄像头名, media 目录下的演示视频文件名, 区域名)
SEED_CAMERAS = [
    ("lab-slice", "lab-walk-24076.mp4", "reagent-area"),
    ("ppe-regress", "lab-walk-24076.mp4", "reagent"),
    ("ppe-final", "lab-walk-24076.mp4", "reagent"),
    ("ppe-equip", "lab-equip-21453.mp4", "bench"),
]
SEED_ZONES = {
    "reagent-area": "[[520, 180], [1180, 180], [1180, 620], [520, 620]]",
    "reagent": "[[520, 180], [1180, 180], [1180, 620], [520, 620]]",
    "bench": "[[100, 50], [1180, 50], [1180, 650], [100, 650]]",
}


def seed_database(db_path, media_dir) -> int:
    """空库播种 4 路演示摄像头 + 区域 + 规则；已有摄像头则不动。

    返回插入的摄像头数（0 = 已有数据跳过 / media 缺失跳过）。
    db_path 的父目录不存在时会自动创建；media_dir 指向演示视频所在目录。
    """
    db_path = Path(db_path)
    media_dir = Path(media_dir)
    db_path.parent.mkdir(parents=True, exist_ok=True)
    db = sqlite3.connect(str(db_path))
    try:
        db.executescript(SCHEMA)
        n = db.execute("select count(*) from cameras").fetchone()[0]
        if n:
            return 0
        if not all((media_dir / fn).exists() for _, fn, _ in SEED_CAMERAS):
            return 0
        inserted = 0
        for name, fn, zname in SEED_CAMERAS:
            cid = db.execute("insert into cameras(name, source, type) values(?,?,'file')",
                             (name, str((media_dir / fn).as_posix()))).lastrowid
            zid = db.execute("insert into zones(camera_id, name, polygon) values(?,?,?)",
                             (cid, zname, SEED_ZONES[zname])).lastrowid
            for behavior, level in (("enter", 2), ("enter_no_ppe", 3)):
                db.execute(
                    "insert into rules(camera_id, zone_id, behavior, level, cooldown_s,"
                    " time_start, time_end, threshold, enabled) values(?,?,?,?,5,'00:00','23:59',0,1)",
                    (cid, zid, behavior, level))
            inserted += 1
        db.commit()
        return inserted
    finally:
        db.close()


def _selftest() -> int:
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        media = tmp / "media"
        media.mkdir()
        for _, fn, _ in SEED_CAMERAS:
            (media / fn).write_bytes(b"x")
        db = tmp / "seed.db"
        n1 = seed_database(db, media)
        db2 = sqlite3.connect(str(db))
        cams = db2.execute("select count(*) from cameras").fetchone()[0]
        zones = db2.execute("select count(*) from zones").fetchone()[0]
        rules = db2.execute("select count(*) from rules").fetchone()[0]
        db2.close()
        n2 = seed_database(db, media)  # 幂等
        print(f"首次播种 {n1} 路 | cams={cams} zones={zones} rules={rules} | 二次返回 {n2}")
        assert (n1, cams, zones, rules, n2) == (4, 4, 4, 8, 0)
        # 用户数据不覆盖
        db3 = sqlite3.connect(str(db))
        db3.execute("delete from cameras")
        db3.execute("insert into cameras(name, source, type) values('user-cam','X:/c.mp4','file')")
        db3.commit()
        db3.close()
        n3 = seed_database(db, media)
        assert n3 == 0, "用户数据被覆盖！"
        print("用户数据不覆盖 ✓")
        # schema 可被 Store 打开（可选）：能执行 PRAGMA 即视为结构成立
        db4 = sqlite3.connect(str(db))
        db4.execute("pragma integrity_check").fetchone()
        db4.close()
    print("selftest OK")
    return 0


if __name__ == "__main__":
    import sys
    sys.exit(_selftest() if "--selftest" in sys.argv else 0)
