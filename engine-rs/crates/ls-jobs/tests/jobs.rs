//! ls-jobs 集成测试：与 Python `retention.py` / `backup_data.py` / `escalation.py` 的
//! 关键语义逐条对齐（清理范围、路径安全、宽限期、紧急三档、备份 zip 兼容、升级提醒）。

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use ls_jobs::backup::{self, BackupConfig};
use ls_jobs::escalation::{self, EscalationConfig, EscalationRow};
use ls_jobs::retention::{self, RetentionConfig};
use ls_store::Store;
use rusqlite::Connection;

fn make_db(dir: &Path) -> PathBuf {
    let path = dir.join("data").join("labsentinel.db");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let c = Connection::open(&path).unwrap();
    for stmt in [
        "CREATE TABLE schema_version(version INTEGER PRIMARY KEY, applied_at TEXT)",
        "INSERT INTO schema_version VALUES(13,'2026-01-01T00:00:00')",
        "CREATE TABLE events(id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT, camera_id INTEGER, \
         type TEXT, level INTEGER, track_id INTEGER, detail TEXT, snapshot TEXT, video TEXT, \
         status TEXT DEFAULT 'new', handled_by TEXT, handled_ts TEXT, escalated_ts TEXT)",
        "CREATE TABLE alert_deliveries(id INTEGER PRIMARY KEY AUTOINCREMENT, event_id INTEGER, \
         channel TEXT, status TEXT DEFAULT 'pending', attempts INTEGER DEFAULT 0, \
         last_error TEXT, updated_ts TEXT)",
        "CREATE TABLE audit_log(id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, \
         request_id TEXT, actor TEXT, action TEXT NOT NULL, target TEXT, status_code INTEGER, \
         detail TEXT)",
        "CREATE TABLE folder_images(id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER NOT NULL, \
         path TEXT NOT NULL, name TEXT, mtime_ns INTEGER NOT NULL DEFAULT 0, \
         size INTEGER NOT NULL DEFAULT 0, ts TEXT NOT NULL, processed_ts TEXT NOT NULL, \
         width INTEGER, height INTEGER, persons INTEGER DEFAULT 0, in_zone INTEGER DEFAULT 0, \
         ppe_no INTEGER DEFAULT 0, ppe_wearing INTEGER DEFAULT 0, zones TEXT, violations TEXT, \
         key INTEGER DEFAULT 0, violation INTEGER DEFAULT 0, event_id INTEGER, snapshot TEXT, \
         error TEXT)",
        "CREATE TABLE alert_silences(id INTEGER PRIMARY KEY AUTOINCREMENT, scope TEXT, target TEXT, \
         reason TEXT, created_ts TEXT, until_ts TEXT, created_by TEXT)",
    ] {
        c.execute(stmt, []).unwrap();
    }
    drop(c);
    path
}

fn insert_event(
    db: &Path,
    ts: &str,
    type_: &str,
    level: i64,
    snapshot: Option<&str>,
    video: Option<&str>,
    status: &str,
) -> i64 {
    let c = Connection::open(db).unwrap();
    c.execute(
        "INSERT INTO events(ts,camera_id,type,level,track_id,detail,snapshot,video,status) \
         VALUES(?,1,?,?,7,'测试事件',?,?,?)",
        rusqlite::params![ts, type_, level, snapshot, video, status],
    )
    .unwrap();
    c.last_insert_rowid()
}

fn touch_old(path: &Path, age_s: u64) {
    let f = fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(SystemTime::now() - Duration::from_secs(age_s))
        .unwrap();
}

const GB100: f64 = 100.0;

#[test]
fn retention_deletes_expired_events_and_files_only_inside_data_dir() {
    let dir = tempfile::tempdir().unwrap();
    let db = make_db(dir.path());
    let data = dir.path().join("data");
    let snaps = data.join("snapshots");
    fs::create_dir_all(&snaps).unwrap();
    let inside = snaps.join("a.jpg");
    fs::write(&inside, b"12345").unwrap();
    let outside = dir.path().join("outside.jpg");
    fs::write(&outside, b"xyz").unwrap();
    let kept = snaps.join("keep.jpg");
    fs::write(&kept, b"keep").unwrap();

    insert_event(
        &db,
        "2000-01-01T00:00:00",
        "enter",
        2,
        Some(inside.to_str().unwrap()),
        Some(outside.to_str().unwrap()),
        "new",
    );
    insert_event(
        &db,
        "2999-01-01T00:00:00",
        "enter",
        2,
        Some(kept.to_str().unwrap()),
        None,
        "new",
    );

    let store = Store::open(db.to_str().unwrap()).unwrap();
    let cfg = RetentionConfig::for_data_dir(&data);
    let stats = retention::run_cleanup_with(&store, &cfg, &|_| GB100).unwrap();

    assert_eq!(stats.deleted_events, 1);
    assert!(!inside.exists(), "data_dir 内的过期证据应删除");
    assert!(outside.exists(), "data_dir 外的文件绝不能删");
    assert!(kept.exists(), "未过期事件的证据应保留");
    assert_eq!(store.count_events().unwrap(), 1);
}

#[test]
fn retention_orphan_grace_reference_and_limit() {
    let dir = tempfile::tempdir().unwrap();
    let db = make_db(dir.path());
    let data = dir.path().join("data");
    let snaps = data.join("snapshots");
    fs::create_dir_all(&snaps).unwrap();
    let orphan = snaps.join("orphan.jpg");
    let fresh = snaps.join("fresh.jpg");
    let referenced = snaps.join("referenced.jpg");
    for p in [&orphan, &fresh, &referenced] {
        fs::write(p, b"jpegdata").unwrap();
    }
    touch_old(&orphan, 7200);
    touch_old(&referenced, 7200);
    insert_event(
        &db,
        "2999-01-01T00:00:00",
        "enter",
        2,
        Some(referenced.to_str().unwrap()),
        None,
        "new",
    );

    let store = Store::open(db.to_str().unwrap()).unwrap();
    let cfg = RetentionConfig::for_data_dir(&data);
    let stats = retention::run_cleanup_with(&store, &cfg, &|_| GB100).unwrap();

    assert_eq!(stats.deleted_orphans, 1);
    assert!(!orphan.exists(), "1h 宽限外的无引用媒体应删除");
    assert!(fresh.exists(), "宽限期内新文件必须保留");
    assert!(referenced.exists(), "仍被事件引用的媒体必须保留");
}

#[test]
fn retention_preview_is_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let db = make_db(dir.path());
    let data = dir.path().join("data");
    let snaps = data.join("snapshots");
    fs::create_dir_all(&snaps).unwrap();
    let f = snaps.join("old.jpg");
    fs::write(&f, b"1234").unwrap();
    insert_event(
        &db,
        "2000-01-01T00:00:00",
        "enter",
        2,
        Some(f.to_str().unwrap()),
        None,
        "new",
    );
    let store = Store::open(db.to_str().unwrap()).unwrap();
    let cfg = RetentionConfig::for_data_dir(&data);
    let preview = retention::preview_cleanup_with(&store, &cfg, &|_| GB100).unwrap();
    assert_eq!(preview["events"]["count"], 1);
    assert_eq!(preview["events"]["bytes"], 4);
    assert!(!preview["would_emergency_clean"].as_bool().unwrap());
    assert!(f.exists(), "预演绝不删除文件");
    assert_eq!(store.count_events().unwrap(), 1);
}

#[test]
fn retention_emergency_tier1_clears_handled_video_only() {
    let dir = tempfile::tempdir().unwrap();
    let db = make_db(dir.path());
    let data = dir.path().join("data");
    let videos = data.join("videos");
    let snaps = data.join("snapshots");
    fs::create_dir_all(&videos).unwrap();
    fs::create_dir_all(&snaps).unwrap();
    let video = videos.join("v.mp4");
    let snap = snaps.join("s.jpg");
    fs::write(&video, b"video").unwrap();
    fs::write(&snap, b"snap").unwrap();
    let handled = insert_event(
        &db,
        "2999-01-01T00:00:00",
        "enter",
        2,
        Some(snap.to_str().unwrap()),
        Some(video.to_str().unwrap()),
        "acked",
    );
    let pending = insert_event(&db, "2999-01-01T00:00:01", "enter", 3, None, None, "new");

    let calls = AtomicUsize::new(0);
    let free = |_p: &Path| -> f64 {
        if calls.fetch_add(1, Ordering::SeqCst) < 2 {
            0.0
        } else {
            GB100
        }
    };
    let store = Store::open(db.to_str().unwrap()).unwrap();
    let cfg = RetentionConfig::for_data_dir(&data);
    let stats = retention::run_cleanup_with(&store, &cfg, &free).unwrap();
    assert_eq!(stats.deleted_events, 0, "档①只删文件，不删行");

    let got = store.event_by_id(handled).unwrap().unwrap();
    assert!(got.video.is_none(), "已处置事件的 video 引用应置 NULL");
    assert!(got.snapshot.is_some(), "同一轮不应删 snapshot（档②）");
    assert!(!video.exists());
    assert!(snap.exists());
    assert!(
        store.event_by_id(pending).unwrap().is_some(),
        "未处置事件不受影响"
    );
}

#[test]
fn backup_zip_layout_verify_and_python_tool_compat() {
    let dir = tempfile::tempdir().unwrap();
    let db = make_db(dir.path());
    insert_event(&db, "2026-01-01T00:00:00", "enter", 2, None, None, "new");
    let data = dir.path().join("data");
    let snaps = data.join("snapshots");
    fs::create_dir_all(&snaps).unwrap();
    fs::write(snaps.join("x.jpg"), b"media").unwrap();

    let mut cfg = BackupConfig::for_data_dir(&data);
    cfg.include_media = true;
    let out = dir.path().join("out.zip");
    let outcome = backup::run_backup(&cfg, &out).unwrap();
    assert_eq!(outcome.entries, 3);
    assert_eq!(outcome.media_files, 1);
    assert_eq!(outcome.integrity_check, "ok");

    let mut archive = zip::ZipArchive::new(fs::File::open(&out).unwrap()).unwrap();
    let names: Vec<String> = archive
        .file_names()
        .filter_map(Result::ok)
        .map(|s| s.to_string())
        .collect();
    assert!(names.contains(&"manifest.json".to_string()));
    assert!(names.contains(&"labsentinel.db".to_string()));
    assert!(names.contains(&"media/snapshots/x.jpg".to_string()));
    let mut mtext = String::new();
    archive
        .by_name("manifest.json")
        .unwrap()
        .read_to_string(&mut mtext)
        .unwrap();
    let m: serde_json::Value = serde_json::from_str(&mtext).unwrap();
    for key in [
        "created_at",
        "source_data_dir",
        "db_bytes",
        "db_sha256",
        "media_files",
        "media_bytes",
        "integrity_check",
        "tool_version",
    ] {
        assert!(m.get(key).is_some(), "manifest 缺字段 {key}");
    }
    assert_eq!(m["db_sha256"].as_str().unwrap().len(), 64);

    let report = backup::verify_zip(&out).unwrap();
    assert_eq!(report.media_entries, 1);
    assert_eq!(report.integrity_check, "ok");

    // 跨语言兼容：Python backup_data.py verify 必须能读 Rust 产出的 zip。
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let script = repo.join("scripts").join("backup_data.py");
    if script.exists() {
        let attempt = std::process::Command::new("python")
            .arg("-I")
            .arg(&script)
            .args(["verify", "--zip"])
            .arg(&out)
            .output();
        match attempt {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let stderr = String::from_utf8_lossy(&o.stderr);
                assert!(
                    o.status.success(),
                    "Python verify 拒绝 Rust 备份: {stdout} {stderr}"
                );
            }
            Err(_) => eprintln!("跳过：本机无 python，跨语言 zip 兼容未在本轮验证"),
        }
    }
}

#[test]
fn backup_auto_prune_keeps_latest() {
    let dir = tempfile::tempdir().unwrap();
    let db = make_db(dir.path());
    insert_event(&db, "2026-01-01T00:00:00", "enter", 2, None, None, "new");
    let data = dir.path().join("data");
    let mut cfg = BackupConfig::for_data_dir(&data);
    cfg.keep = 1;
    let first = backup::run_auto_backup(&cfg).unwrap();
    let first_path = PathBuf::from(&first.path);
    // 挪成"很旧"的备份，确保第二轮清理能命中
    let old = backup::backup_dir(&cfg).join("auto-20000101-000000.zip");
    fs::rename(&first_path, &old).unwrap();
    touch_old(&old, 86_400 * 365);
    let second = backup::run_auto_backup(&cfg).unwrap();
    let files = backup::auto_files(&cfg);
    assert_eq!(files.len(), 1, "keep=1 只保留最新一份");
    assert!(files[0].ends_with(&second.name));
    assert!(!old.exists());
}

#[test]
fn backup_restore_is_conservative() {
    let dir = tempfile::tempdir().unwrap();
    let db = make_db(dir.path());
    insert_event(&db, "2026-01-01T00:00:00", "enter", 2, None, None, "new");
    let data = dir.path().join("data");
    let cfg = BackupConfig::for_data_dir(&data);
    let out = dir.path().join("snap.zip");
    backup::run_backup(&cfg, &out).unwrap();

    let c = Connection::open(&db).unwrap();
    c.execute(
        "INSERT INTO events(ts,camera_id,type,level,track_id,detail,status) \
         VALUES('2026-01-02T00:00:00',1,'enter',2,1,'备份后新增','new')",
        [],
    )
    .unwrap();
    drop(c);

    // 无 force：拒绝且不改动
    assert!(backup::restore_zip(&cfg, &out, false, false).is_err());
    let store = Store::open(db.to_str().unwrap()).unwrap();
    assert_eq!(store.count_events().unwrap(), 2);
    drop(store); // Windows 下占着连接会阻止 db 改名

    // force：转存 .bak 后恢复
    let report = backup::restore_zip(&cfg, &out, true, false).unwrap();
    assert_eq!(report.integrity_check, "ok");
    let store = Store::open(db.to_str().unwrap()).unwrap();
    assert_eq!(store.count_events().unwrap(), 1);
    let baks: Vec<_> = fs::read_dir(&data)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("labsentinel.db.bak-"))
        })
        .collect();
    assert_eq!(baks.len(), 1, "覆盖前必须转存 .bak");
}

#[test]
fn escalation_only_unhandled_high_level_once() {
    let dir = tempfile::tempdir().unwrap();
    let db = make_db(dir.path());
    let old = (chrono::Local::now() - chrono::TimeDelta::hours(2))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    let target = insert_event(&db, &old, "enter", 3, None, None, "new");
    insert_event(&db, &old, "enter", 1, None, None, "new"); // 级别不足
    insert_event(&db, &old, "enter", 3, None, None, "acked"); // 已处置
    let already = insert_event(&db, &old, "enter", 3, None, None, "new");
    let c = Connection::open(&db).unwrap();
    c.execute(
        "UPDATE events SET escalated_ts='2026-01-01T00:00:00' WHERE id=?",
        [already],
    )
    .unwrap();
    drop(c);

    let store = Store::open(db.to_str().unwrap()).unwrap();
    let cfg = EscalationConfig {
        after_min: 15,
        min_level: 3,
        batch: 20,
        ..EscalationConfig::default()
    };
    let seen = std::sync::Mutex::new(Vec::<String>::new());
    let send = |_row: &EscalationRow, text: &str| {
        seen.lock().unwrap().push(text.to_string());
        true
    };
    let out = escalation::run_once(&store, &cfg, None, Some(&send)).unwrap();
    assert_eq!(out.checked, 1);
    assert_eq!(out.escalated, 1);
    let texts = seen.lock().unwrap();
    assert!(texts[0].contains("⏰ 未处置提醒（已 120 分钟）"));
    assert!(texts[0].contains("进入安全区"));

    let c = Connection::open(&db).unwrap();
    let escalated: Option<String> = c
        .query_row(
            "SELECT escalated_ts FROM events WHERE id=?",
            [target],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        escalated.is_some(),
        "提醒后必须写 escalated_ts（失败也标记）"
    );
    drop(c);

    let again = escalation::run_once(&store, &cfg, None, Some(&send)).unwrap();
    assert_eq!(again.checked, 0, "同一事件不重复提醒");
}

#[test]
fn escalation_disabled_when_after_min_zero() {
    let dir = tempfile::tempdir().unwrap();
    let db = make_db(dir.path());
    let store = Store::open(db.to_str().unwrap()).unwrap();
    let cfg = EscalationConfig {
        after_min: 0,
        ..EscalationConfig::default()
    };
    let out = escalation::run_once(&store, &cfg, None, None).unwrap();
    assert_eq!(out.disabled, Some(true));
    assert_eq!(out.escalated, 0);
}

#[test]
fn render_alert_matches_python_default_template() {
    let row = EscalationRow {
        id: 1,
        ts: "2026-01-01T00:00:00".into(),
        camera_id: 3,
        type_: "enter".into(),
        level: 2,
        track_id: 7,
        detail: "张三进入 A 区".into(),
    };
    assert_eq!(
        escalation::render_alert(&row, escalation::DEFAULT_TEMPLATE),
        "⚠️ L2 进入安全区 · 2026-01-01T00:00:00\n摄像头 #3 · track 7\n张三进入 A 区"
    );
    let occ = EscalationRow {
        type_: "occupancy_over".into(),
        ..row
    };
    assert!(!escalation::render_alert(&occ, escalation::DEFAULT_TEMPLATE).contains("track"));
}

#[test]
fn jobs_scheduler_starts_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let db = make_db(dir.path());
    let cfg = ls_jobs::JobsConfig::default_for_db(&db);
    let jobs = std::sync::Arc::new(ls_jobs::Jobs::new(db.clone(), cfg));
    jobs.start();
    std::thread::sleep(Duration::from_millis(300));
    jobs.stop();
    // 首轮 retention 已执行（空库 → 统计存在）
    assert!(jobs.retention_last().is_some());
}
