use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime},
};

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use ls_store::Store;
use rusqlite::{Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{state::Shared, util::*};

const BATCH: usize = 500;
const POLICY: &str = "storage.retention";
const VULKAN_CACHE_DIR: &str = "runtime-cache/vulkan";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
enum Category {
    Logs,
    Cache,
    VulkanCache,
    Events,
    NormalChecks,
    Dataset,
    Backups,
}

impl Category {
    const ALL: [Self; 7] = [
        Self::Logs,
        Self::Cache,
        Self::VulkanCache,
        Self::Events,
        Self::NormalChecks,
        Self::Dataset,
        Self::Backups,
    ];
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Selection {
    category: Category,
    days: u16,
}

impl Selection {
    fn valid(self) -> bool {
        (1..=3650).contains(&self.days)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Confirm {
    selection: Selection,
    preview: String,
}

#[derive(Default)]
pub(crate) struct StorageRuntime(Mutex<()>);

#[derive(Clone, Serialize)]
struct FileItem {
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
}

#[derive(Default, Serialize)]
struct Plan {
    events: Vec<i64>,
    checks: Vec<i64>,
    samples: Vec<i64>,
    files: Vec<FileItem>,
}

impl Plan {
    fn records(&self) -> usize {
        self.events.len() + self.checks.len() + self.samples.len()
    }

    fn preview(&self, choice: Selection) -> String {
        sha256_hex(&serde_json::to_vec(&(choice, self)).expect("serializable storage preview"))
    }
}

fn file_item(root: &Path, path: &Path, dirs: &[&str]) -> Option<FileItem> {
    let path = root.join(path);
    let file = fs::symlink_metadata(&path).ok()?;
    if !file.is_file() {
        return None;
    }
    let real = path.canonicalize().ok()?;
    let parent = real.parent()?;
    if !dirs.iter().any(|dir| root.join(dir) == parent) {
        return None;
    }
    Some(FileItem {
        path: real,
        bytes: file.len(),
        modified: file.modified().ok()?,
    })
}

fn directory_files(root: &Path, dir: &str, name_ok: impl Fn(&str) -> bool) -> Vec<FileItem> {
    fs::read_dir(root.join(dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name().to_str().is_some_and(&name_ok))
        .filter_map(|entry| file_item(root, &entry.path(), &[dir]))
        .collect()
}

fn vulkan_cache_files(root: &Path) -> Vec<FileItem> {
    directory_files(root, VULKAN_CACHE_DIR, |name| {
        name.strip_prefix("ncnn-")
            .and_then(|name| name.strip_suffix(".dpapi"))
            .and_then(|name| name.split_once("-pipelines-"))
            .is_some_and(|(hash, gpu)| {
                hash.len() == 64
                    && hash.bytes().all(|c| c.is_ascii_hexdigit())
                    && !gpu.is_empty()
                    && gpu.bytes().all(|c| c.is_ascii_digit())
            })
    })
}

fn plan(store: &Store, root: &Path, selection: Selection) -> rusqlite::Result<Plan> {
    let days = i64::from(selection.days);
    let cutoff = ls_core::time::local_days_ago_iso(days);
    let conn = store.conn();
    let mut plan = Plan::default();
    let mut files = BTreeMap::new();
    let mut add_file = |path: &Path, dirs: &[&str]| {
        if let Some(file) = file_item(root, path, dirs) {
            files.insert(file.path.clone(), file);
        }
    };
    match selection.category {
        Category::Events => {
            let mut stmt = conn.prepare(
                "SELECT id,snapshot,video FROM events WHERE ts < ?1
                 AND status IN ('acked','false_positive','ignored')
                 AND NOT EXISTS(SELECT 1 FROM violation_episodes ep WHERE ep.id=events.episode_id AND ep.closed_ts IS NULL)
                 AND NOT EXISTS(SELECT 1 FROM alert_deliveries d WHERE d.event_id=events.id AND COALESCE(d.status,'pending') IN ('pending','retry','failed','sending','unconfirmed'))
                 AND NOT EXISTS(SELECT 1 FROM alert_outbox o WHERE o.event_id=events.id AND o.status IN ('pending','awaiting_review'))
                 ORDER BY id LIMIT ?2")?;
            for row in stmt.query_map(rusqlite::params![cutoff, BATCH as i64], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })? {
                let (id, snapshot, video) = row?;
                plan.events.push(id);
                for path in [snapshot, video].into_iter().flatten() {
                    add_file(Path::new(&path), &["snapshots", "videos"]);
                }
                add_file(Path::new(&format!("thumbs/ev_{id}.jpg")), &["thumbs"]);
            }
        }
        Category::NormalChecks => {
            let mut stmt = conn.prepare(
                "SELECT id,snapshot,path FROM folder_images WHERE processed_ts < ?1
                 AND COALESCE(violation,0)=0 AND error IS NULL AND
                 (snapshot IS NOT NULL OR event_id IS NOT NULL) ORDER BY id LIMIT ?2",
            )?;
            for row in stmt.query_map(rusqlite::params![cutoff, BATCH as i64], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })? {
                let (id, snapshot, original) = row?;
                plan.checks.push(id);
                if let Some(snapshot) = snapshot {
                    let path = Path::new(&snapshot);
                    if root
                        .join(path)
                        .canonicalize()
                        .ok()
                        .zip(root.join(original).canonicalize().ok())
                        .is_none_or(|(a, b)| a != b)
                    {
                        add_file(path, &["snapshots"]);
                    }
                }
                add_file(Path::new(&format!("thumbs/img_{id}.jpg")), &["thumbs"]);
            }
        }
        Category::Dataset => {
            let mut stmt = conn.prepare(
                "SELECT id,image_path FROM dataset_samples WHERE updated_at < ?1 ORDER BY id LIMIT ?2")?;
            for row in stmt.query_map(rusqlite::params![cutoff, BATCH as i64], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })? {
                let (id, path) = row?;
                plan.samples.push(id);
                add_file(Path::new(&path), &["dataset_pool"]);
            }
        }
        category => {
            let today_log = format!("server-{}.log", ls_core::time::local_today_compact());
            let mut candidates = match category {
                Category::Logs => directory_files(root, "logs", |name| {
                    name != today_log && name.starts_with("server-") && name.ends_with(".log")
                }),
                Category::Cache => directory_files(root, "thumbs", |name| {
                    (name.starts_with("ev_") || name.starts_with("img_")) && name.ends_with(".jpg")
                }),
                Category::VulkanCache => vulkan_cache_files(root),
                Category::Backups => directory_files(root, "backups", |name| {
                    name.starts_with("auto-") && name.ends_with(".zip")
                }),
                _ => unreachable!(),
            };
            candidates.sort_by(|a, b| (b.modified, &b.path).cmp(&(a.modified, &a.path)));
            if category == Category::Backups && !candidates.is_empty() {
                candidates.remove(0);
            }
            let age = Duration::from_secs(u64::from(selection.days) * 86_400);
            let now = SystemTime::now();
            for file in candidates
                .into_iter()
                .rev()
                .filter(|file| {
                    now.duration_since(file.modified)
                        .is_ok_and(|elapsed| elapsed >= age)
                })
                .take(BATCH)
            {
                files.insert(file.path.clone(), file);
            }
        }
    }
    if !files.is_empty() && plan.records() != 0 {
        exclude_retained_references(conn, root, &plan, &mut files)?;
    }
    plan.files = files.into_values().collect();
    Ok(plan)
}

fn exclude_retained_references(
    conn: &rusqlite::Connection,
    root: &Path,
    plan: &Plan,
    files: &mut BTreeMap<PathBuf, FileItem>,
) -> rusqlite::Result<()> {
    let ids = |ids: &[i64]| json!(ids).to_string();
    let sql = "SELECT snapshot FROM events WHERE id NOT IN (SELECT value FROM json_each(?1))
               UNION SELECT video FROM events WHERE id NOT IN (SELECT value FROM json_each(?1))
               UNION SELECT snapshot FROM folder_images WHERE id NOT IN (SELECT value FROM json_each(?2))
               UNION SELECT path FROM folder_images
               UNION SELECT image_path FROM dataset_samples WHERE id NOT IN (SELECT value FROM json_each(?3))
               UNION SELECT path FROM suppression_images";
    let mut stmt = conn.prepare(sql)?;
    for path in stmt.query_map(
        rusqlite::params![ids(&plan.events), ids(&plan.checks), ids(&plan.samples)],
        |row| row.get::<_, Option<String>>(0),
    )? {
        if let Some(real) = path?.and_then(|path| root.join(path).canonicalize().ok()) {
            files.remove(&real);
        }
    }
    Ok(())
}

fn row_cleanup(tx: &Transaction<'_>, plan: &Plan) -> rusqlite::Result<()> {
    for id in &plan.events {
        tx.execute(
            "UPDATE folder_images SET event_id=NULL WHERE event_id=?",
            [id],
        )?;
        tx.execute("DELETE FROM events WHERE id=?", [id])?;
    }
    for id in &plan.checks {
        tx.execute(
            "UPDATE folder_images SET snapshot=NULL,event_id=NULL WHERE id=?",
            [id],
        )?;
    }
    for id in &plan.samples {
        tx.execute("DELETE FROM dataset_samples WHERE id=?", [id])?;
    }
    Ok(())
}

#[derive(Default, Serialize)]
struct Cleanup {
    records: usize,
    files: usize,
    skipped_files: usize,
    freed_bytes: u64,
}

fn execute(
    st: &Shared,
    root: &Path,
    choice: Selection,
    preview: Option<&str>,
) -> Result<Option<Cleanup>, String> {
    let store = Store::open(&st.db_path).map_err(|e| e.to_string())?;
    let tx = Transaction::new_unchecked(store.conn(), TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let plan = plan(&store, root, choice).map_err(|e| e.to_string())?;
    if preview.is_some_and(|token| token != plan.preview(choice)) {
        return Ok(None);
    }
    row_cleanup(&tx, &plan).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    let mut result = Cleanup {
        records: plan.records(),
        ..Default::default()
    };
    for file in plan.files {
        if fs::symlink_metadata(&file.path).is_ok_and(|meta| {
            meta.is_file()
                && meta.len() == file.bytes
                && meta.modified().ok() == Some(file.modified)
                && file.path.canonicalize().ok() == Some(file.path.clone())
        }) && fs::remove_file(&file.path).is_ok()
        {
            result.files += 1;
            result.freed_bytes += file.bytes;
        } else {
            result.skipped_files += 1;
        }
    }
    Ok(Some(result))
}

async fn block(
    st: Shared,
    action: impl FnOnce(Shared, PathBuf) -> Response + Send + 'static,
) -> Response {
    tokio::task::spawn_blocking(move || {
        let Ok(root) = Path::new(&st.data_dir).canonicalize() else {
            return server_err();
        };
        action(st, root)
    })
    .await
    .unwrap_or_else(|_| server_err())
}

pub(crate) async fn usage(State(st): State<Shared>) -> Response {
    block(st, |st, root| {
        let directories = ["logs", "thumbs", "snapshots", "videos", "dataset_pool", "annotation_pool", "backups"];
        let group = |id: &str, files: Vec<FileItem>| {
            json!({"id": id, "files": files.len(), "bytes": files.iter().map(|file| file.bytes).sum::<u64>()})
        };
        let mut groups: Vec<_> = directories.iter().map(|&dir| {
            group(dir, directory_files(&root, dir, |_| true))
        }).collect();
        groups.push(group("vulkan_cache", vulkan_cache_files(&root)));
        let db_bytes: Vec<_> = [st.db_path.clone(), format!("{}-wal", st.db_path)]
            .iter().filter_map(|path| fs::metadata(path).ok().map(|meta| meta.len())).collect();
        groups.push(json!({"id": "database", "files": db_bytes.len(), "bytes": db_bytes.iter().sum::<u64>()}));
        let free_gb = ls_jobs::paths::disk_free_gb(&root);
        with_store(&st, |store| Ok(Json(json!({
            "free_bytes": free_gb.is_finite().then_some((free_gb * 1e9) as u64),
            "total_bytes": ls_jobs::paths::disk_total_gb(&root).map(|gb| (gb * 1e9) as u64),
            "groups": groups, "policy": policy(store), "database_reusable_bytes": database_reusable_bytes(store)?,
        }))))
    }).await
}

fn database_reusable_bytes(store: &Store) -> rusqlite::Result<u64> {
    let get = |pragma| {
        store
            .conn()
            .pragma_query_value(None, pragma, |row| row.get::<_, i64>(0))
    };
    Ok(get("freelist_count")?
        .max(0)
        .saturating_mul(get("page_size")?.max(0)) as u64)
}

pub(crate) async fn compact(State(st): State<Shared>) -> Response {
    block(st, |st, root| {
        let Ok(_guard) = st.storage.0.try_lock() else {
            return detail_err(StatusCode::CONFLICT, "另一次清理正在运行");
        };
        if !st.workers.is_idle() {
            return detail_err(StatusCode::CONFLICT, "请停止检测任务后再整理数据库");
        }
        with_store(&st, |store| {
            if database_reusable_bytes(store)? == 0 {
                return Ok(Json(json!({"reclaimed_bytes": 0})).into_response());
            }
            let size = || {
                [st.db_path.clone(), format!("{}-wal", st.db_path)]
                    .iter()
                    .filter_map(|path| fs::metadata(path).ok().map(|meta| meta.len()))
                    .sum::<u64>()
            };
            let before = size();
            let free = ls_jobs::paths::disk_free_gb(&root);
            if !free.is_finite() || free * 1e9 < before.saturating_mul(2) as f64 {
                return Ok(detail_err(
                    StatusCode::CONFLICT,
                    "临时空间不足，整理需要约两倍数据库大小的可用空间",
                ));
            }
            store.conn().execute_batch("VACUUM;")?;
            store.checkpoint_wal()?;
            Ok(Json(json!({"reclaimed_bytes": before.saturating_sub(size())})).into_response())
        })
    })
    .await
}

pub(crate) async fn preview(State(st): State<Shared>, Json(choice): Json<Selection>) -> Response {
    if !choice.valid() {
        return detail_err(StatusCode::BAD_REQUEST, "保留天数须为 1 至 3650");
    }
    block(st, move |st, root| with_store(&st, |store| {
        let plan = plan(store, &root, choice)?;
        Ok(Json(json!({
            "preview": plan.preview(choice), "records": plan.records(), "files": plan.files.len(),
            "bytes": plan.files.iter().map(|file| file.bytes).sum::<u64>(),
        })))
    })).await
}

pub(crate) async fn cleanup(State(st): State<Shared>, Json(request): Json<Confirm>) -> Response {
    if !request.selection.valid() || request.preview.len() != 64 {
        return detail_err(StatusCode::BAD_REQUEST, "请先预览清理范围");
    }
    block(st, move |st, root| {
        let Ok(_guard) = st.storage.0.try_lock() else {
            return detail_err(StatusCode::CONFLICT, "另一次清理正在运行");
        };
        match execute(&st, &root, request.selection, Some(&request.preview)) {
            Ok(Some(result)) => Json(result).into_response(),
            Ok(None) => detail_err(StatusCode::CONFLICT, "清理范围已变化，请重新预览"),
            Err(error) => {
                crate::logger::error(&format!("[storage] {error}"));
                server_err()
            }
        }
    })
    .await
}

fn policy(store: &Store) -> Vec<Selection> {
    store
        .setting_get(POLICY)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub(crate) async fn set_policy(
    State(st): State<Shared>,
    Json(policy): Json<Vec<Selection>>,
) -> Response {
    let categories = policy
        .iter()
        .map(|rule| rule.category)
        .collect::<BTreeSet<_>>();
    if policy.len() > Category::ALL.len()
        || categories.len() != policy.len()
        || !policy.iter().all(|rule| rule.valid())
    {
        return detail_err(StatusCode::BAD_REQUEST, "每个类别只允许一项有效的保留策略");
    }
    let Ok(_guard) = st.storage.0.try_lock() else {
        return detail_err(StatusCode::CONFLICT, "清理正在运行，结束后可修改自动规则");
    };
    with_store(&st, |store| {
        store.setting_set(POLICY, &json!(policy).to_string())?;
        Ok(Json(json!({"policy": policy})))
    })
}

pub(crate) fn start(st: Shared) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            let state = st.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let Ok(_guard) = state.storage.0.try_lock() else {
                    return;
                };
                let Ok(root) = Path::new(&state.data_dir).canonicalize() else {
                    return;
                };
                let Ok(store) = Store::open(&state.db_path) else {
                    return;
                };
                for choice in policy(&store).into_iter().filter(|choice| choice.valid()) {
                    match execute(&state, &root, choice, None) {
                        Ok(Some(result))
                            if result.records + result.files + result.skipped_files > 0 =>
                        {
                            crate::logger::info(&format!(
                                "[storage] {:?}: {}",
                                choice.category,
                                json!(result)
                            ))
                        }
                        Err(error) => crate::logger::error(&format!("[storage] {error}")),
                        _ => {}
                    }
                }
            })
            .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::helpers::{req_hdr, test_state_cfg_dir};
    use http_body_util::BodyExt;
    use serde_json::Value;

    async fn body(response: Response) -> Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn storage_preview_protects_live_events_shared_evidence_and_originals() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let db = root.join("storage.db");
        let db = db.to_str().unwrap();
        let store = Store::open(db).unwrap();
        store.ensure_schema().unwrap();
        store.migrate().unwrap();
        for folder in ["snapshots", "dataset_pool", "thumbs", "logs", "backups"] {
            fs::create_dir(root.join(folder)).unwrap();
        }
        for path in [
            "snapshots/shared.jpg",
            "snapshots/deletable.jpg",
            "snapshots/original.jpg",
            "snapshots/check-copy.jpg",
            "thumbs/img_1.jpg",
            "dataset_pool/label.jpg",
        ] {
            fs::write(root.join(path), [5u8; 100]).unwrap();
        }
        let two_days = ls_core::time::local_days_ago_iso(2);
        let event = |path, status| {
            let id = store
                .add_event(&ls_store::NewEvent {
                    camera_id: 1,
                    type_: "no_mask",
                    level: 2,
                    track_id: 1,
                    detail: "",
                    snapshot: Some(path),
                    ts: &two_days,
                })
                .unwrap();
            store
                .conn()
                .execute(
                    "UPDATE events SET status=? WHERE id=?",
                    rusqlite::params![status, id],
                )
                .unwrap();
            id
        };
        let acknowledged = event("snapshots/deletable.jpg", "acked");
        let old_shared = event("snapshots/shared.jpg", "ignored");
        let unhandled = event("snapshots/shared.jpg", "new");
        let folder_sample = store
            .add_folder_image(
                1,
                "snapshots/original.jpg",
                None,
                123,
                100,
                &two_days,
                &two_days,
                None,
                None,
                0,
                0,
                0,
                0,
                "[]",
                "[]",
                false,
                false,
                Some("snapshots/check-copy.jpg"),
                None,
            )
            .unwrap();
        let sample = store
            .add_dataset_sample(
                "event",
                "4",
                Some(1),
                "dataset_pool/label.jpg",
                10,
                10,
                "[]",
            )
            .unwrap();
        store
            .conn()
            .execute(
                "UPDATE dataset_samples SET updated_at=? WHERE id=?",
                rusqlite::params![two_days, sample],
            )
            .unwrap();
        let app = crate::build_router(test_state_cfg_dir(
            db,
            "storage-admin",
            true,
            root.to_str().unwrap(),
        ));
        let runtime = root.join(VULKAN_CACHE_DIR);
        fs::create_dir_all(&runtime).unwrap();
        let stem = format!("ncnn-{}", "a".repeat(64));
        let expired = runtime.join(format!("{stem}-pipelines-0.dpapi"));
        let current = runtime.join(format!("{stem}-pipelines-1.dpapi"));
        let dll = runtime.join(format!("{stem}.dll"));
        let unrelated = runtime.join("ncnn-unrelated-pipelines-0.dpapi");
        for path in [&expired, &current, &dll, &unrelated] {
            fs::write(path, [5u8; 100]).unwrap();
        }
        for path in [&expired, &dll, &unrelated] {
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(SystemTime::now() - Duration::from_secs(2 * 86_400))
                .unwrap();
        }
        let headers = [("x-admin-token", "storage-admin")];
        assert_eq!(
            req_hdr(&app, "GET", "/api/storage", None, &[])
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let status = req_hdr(&app, "GET", "/api/storage", None, &headers).await;
        assert_eq!(status.status(), StatusCode::OK);
        let status = body(status).await;
        let groups = status["groups"].as_array().unwrap();
        assert!(groups.iter().any(|g| g["id"] == "database"));
        assert!(groups
            .iter()
            .any(|g| { g["id"] == "vulkan_cache" && g["files"] == 2 && g["bytes"] == 200 }));

        let preview = async |category: &str| {
            body(
                req_hdr(
                    &app,
                    "POST",
                    "/api/storage/preview",
                    Some(json!({"category": category, "days": 1})),
                    &headers,
                )
                .await,
            )
            .await
        };
        let clean = async |category: &str, token: Value| {
            req_hdr(
                &app,
                "POST",
                "/api/storage/cleanup",
                Some(json!({"selection": {"category": category, "days": 1}, "preview": token})),
                &headers,
            )
            .await
        };
        let planned = preview("vulkan_cache").await;
        assert_eq!(planned["files"], 1);
        assert_eq!(planned["bytes"], 100);
        let cleaned = clean("vulkan_cache", planned["preview"].clone()).await;
        assert_eq!(cleaned.status(), StatusCode::OK);
        assert_eq!(body(cleaned).await["freed_bytes"], 100);
        assert!(!expired.exists());
        assert!(current.is_file() && dll.is_file() && unrelated.is_file());

        let planned = preview("events").await;
        assert_eq!(planned["records"], 2);
        assert_eq!(planned["bytes"], 100);
        fs::write(root.join("snapshots/deletable.jpg"), [6u8; 120]).unwrap();
        let stale = clean("events", planned["preview"].clone()).await;
        assert_eq!(stale.status(), StatusCode::CONFLICT);
        assert!(store.event_by_id(acknowledged).unwrap().is_some());
        let planned = preview("events").await;
        let cleaned = clean("events", planned["preview"].clone()).await;
        assert_eq!(cleaned.status(), StatusCode::OK);
        assert_eq!(body(cleaned).await["freed_bytes"], 120);
        assert!(store.event_by_id(acknowledged).unwrap().is_none());
        assert!(store.event_by_id(old_shared).unwrap().is_none());
        assert!(store.event_by_id(unhandled).unwrap().is_some());
        assert!(root.join("snapshots/shared.jpg").is_file());

        let planned = preview("normal_checks").await;
        assert_eq!(planned["records"], 1);
        assert_eq!(
            clean("normal_checks", planned["preview"].clone())
                .await
                .status(),
            StatusCode::OK
        );
        assert!(root.join("snapshots/original.jpg").is_file());
        assert!(!root.join("snapshots/check-copy.jpg").exists());
        let still_present: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM folder_images WHERE id=? AND snapshot IS NULL",
                [folder_sample],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(still_present, 1);
        let planned = preview("dataset").await;
        assert_eq!(planned["records"], 1);
        assert_eq!(
            clean("dataset", planned["preview"].clone()).await.status(),
            StatusCode::OK
        );
        assert!(!root.join("dataset_pool/label.jpg").exists());
        assert!(store.dataset_sample_by_id(sample).unwrap().is_none());

        store
            .conn()
            .execute_batch(
                "CREATE TABLE compact_test(value BLOB);
             INSERT INTO compact_test VALUES(zeroblob(400000));
             DELETE FROM compact_test;",
            )
            .unwrap();
        assert!(database_reusable_bytes(&store).unwrap() > 0);
        let compacted = req_hdr(&app, "POST", "/api/storage/compact", None, &headers).await;
        assert_eq!(compacted.status(), StatusCode::OK);
        assert!(body(compacted).await["reclaimed_bytes"].as_u64().unwrap() > 0);
        assert_eq!(database_reusable_bytes(&store).unwrap(), 0);
        assert!(store.event_by_id(unhandled).unwrap().is_some());
    }

    #[test]
    fn storage_file_cleanup_preserves_current_log_and_latest_backup() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let store = Store::open(":memory:").unwrap();
        let today = format!("logs/server-{}.log", ls_core::time::local_today_compact());
        for folder in ["logs", "backups", "thumbs"] {
            fs::create_dir(root.join(folder)).unwrap();
        }
        for (path, age) in [
            ("logs/server-20010101.log", 4),
            (today.as_str(), 4),
            ("backups/auto-older.zip", 4),
            ("backups/auto-newer.zip", 3),
            ("backups/manual.zip", 6),
            ("thumbs/ev_7.jpg", 4),
        ] {
            let file = fs::File::create(root.join(path)).unwrap();
            file.set_modified(SystemTime::now() - Duration::from_secs(age * 86_400))
                .unwrap();
        }
        for (category, candidate) in [
            (Category::Logs, "logs/server-20010101.log"),
            (Category::Cache, "thumbs/ev_7.jpg"),
            (Category::Backups, "backups/auto-older.zip"),
        ] {
            let planned = plan(&store, &root, Selection { category, days: 1 }).unwrap();
            assert_eq!(planned.files.len(), 1);
            assert_eq!(planned.files[0].path, root.join(candidate));
        }
    }
}
