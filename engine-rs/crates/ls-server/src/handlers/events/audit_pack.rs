use std::collections::HashSet;
use std::fs::File;
use std::io::{Error, ErrorKind, Read, Seek, Write};
use std::path::Path;

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use futures_util::StreamExt;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use tokio_util::io::ReaderStream;
use zip::write::SimpleFileOptions;

use super::{event_json, EventsQuery};
use crate::access::BrowserAccess;
use crate::state::Shared;
use crate::util::{csv_escape, detail_err, rooted_file, server_err};

const MAX_EVENTS: i64 = 2_000;
const MAX_BYTES: u64 = 256 * 1024 * 1024;
const SIZE_ERROR: &str = "audit pack size limit";
static EXPORT_SLOTS: Semaphore = Semaphore::const_new(2);

pub(crate) async fn export(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
    Query(query): Query<EventsQuery>,
) -> Response {
    let Ok(slot) = EXPORT_SLOTS.try_acquire() else {
        return detail_err(StatusCode::TOO_MANY_REQUESTS, "审计包正在导出，请稍后再试");
    };
    let result = tokio::task::spawn_blocking(move || (build(&st, &browser, &query), slot)).await;
    let (file, slot) = match result {
        Ok((Ok(file), slot)) => (file, slot),
        Ok((Err(error), _)) if error.to_string() == SIZE_ERROR => {
            return detail_err(
                StatusCode::PAYLOAD_TOO_LARGE,
                "审计包超过 2000 条或 256 MiB，请缩小筛选范围",
            );
        }
        _ => return server_err(),
    };
    let stream = ReaderStream::new(tokio::fs::File::from_std(file)).map(move |chunk| {
        let _ = &slot;
        chunk
    });
    (
        [
            (header::CONTENT_TYPE, "application/zip"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"labsentinel-audit.zip\"",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}

type BuildResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn build(st: &Shared, browser: &BrowserAccess, query: &EventsQuery) -> BuildResult<File> {
    let store = ls_store::Store::open(&st.db_path)?;
    let today = ls_core::time::local_today_iso();
    let rows = store.events_query(&ls_store::EventsFilter {
        limit: MAX_EVENTS + 1,
        offset: 0,
        ..query.filter(browser, &today)
    })?;
    if rows.len() > MAX_EVENTS as usize {
        return Err(limit());
    }
    let mut archive = Archive {
        zip: zip::ZipWriter::new(tempfile::tempfile()?),
        remaining: MAX_BYTES,
        manifest: String::new(),
    };
    let root = Path::new(&st.data_dir);
    let mut entries = Vec::with_capacity(rows.len());
    let mut archived_settings = HashSet::new();
    let mut csv = String::from("\u{feff}id,time,camera,type,level,status,detail\r\n");
    let mut html = String::from("<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><title>LabSentinel 审计包</title><h1>事件审计</h1><p><a href=\"events.csv\">CSV</a> · <a href=\"events.json\">JSON</a> · <a href=\"manifest.sha256\">SHA-256</a></p><table><thead><tr><th>事件</th><th>时间</th><th>机位</th><th>类型</th><th>内容</th><th>证据</th></tr></thead><tbody>");
    for event in rows {
        // 只归档本应用留存的媒介。文件夹的用户原始文件留在原处。
        let mut media = json!({});
        let mut links = String::new();
        for (kind, stored, label) in [
            ("snapshot", event.snapshot.as_deref(), "截图"),
            ("video", event.video.as_deref(), "录像"),
        ] {
            let Some(stored) = stored else {
                media[kind] = json!({"state":"none"});
                continue;
            };
            let Some(path) = rooted_file(root, Path::new(stored)) else {
                media[kind] = json!({"state":"unavailable"});
                continue;
            };
            let extension = match (
                kind,
                path.extension()
                    .and_then(|e| e.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
            ) {
                ("video", _) => "mp4",
                (_, Some("webp")) => "webp",
                (_, Some("png")) => "png",
                (_, Some("jpg" | "jpeg")) => "jpg",
                _ => "bin",
            };
            let name = format!("media/{}/{kind}.{extension}", event.id);
            let file = match File::open(path) {
                Ok(file) => file,
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    media[kind] = json!({"state":"unavailable"});
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            let hash = archive.add(&name, file)?;
            let expected = (kind == "snapshot")
                .then_some(event.snapshot_sha256.as_deref())
                .flatten();
            let integrity = compare_hash(expected, &hash);
            media[kind] =
                json!({"state":"included","path":name,"sha256":hash,"integrity":integrity});
            if let Some(expected) = expected {
                media[kind]["recorded_sha256"] = json!(expected);
            }
            use std::fmt::Write as _;
            let _ = write!(links, "<a href=\"{name}\">{label}</a> ");
            if integrity == "mismatch" {
                links.push_str("（与存证校验值不符） ");
            }
        }
        let mut entry = event_json(&event);
        entry["media"] = media;
        entry["recording"] = json!(store.event_recording(event.id)?);
        entry["provenance_hash"] = json!(null);
        if let Some((recorded, settings)) = store.event_provenance(event.id)? {
            let sha = crate::util::sha256_hex(settings.as_bytes());
            let name = format!("provenance/{sha}.json");
            if archived_settings.insert(sha.clone()) {
                archive.add(&name, settings.as_bytes())?;
            }
            let integrity = compare_hash(Some(&recorded), &sha);
            entry["provenance_hash"] = json!(recorded);
            entry["provenance"] = json!({
                "path": name, "sha256": sha, "recorded_sha256": recorded, "integrity": integrity,
            });
            use std::fmt::Write as _;
            let _ = write!(links, "<a href=\"{name}\">规则与模型记录</a> ");
            if integrity == "mismatch" {
                links.push_str("（与存证校验值不符） ");
            }
        }
        entries.push(entry);
        use std::fmt::Write as _;
        let _ = writeln!(
            csv,
            "{},{},{},{},{},{},{}\r",
            event.id,
            cell(&event.ts),
            event.camera_id,
            cell(&event.type_),
            event.level,
            cell(&event.status),
            cell(&event.detail)
        );
        let _ = write!(
            html,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{links}</td></tr>",
            event.id,
            escape(&event.ts),
            event.camera_id,
            escape(&ls_dispatch::type_text(&event.type_)),
            escape(&event.detail)
        );
    }
    html.push_str("</tbody></table></html>");
    archive.add("index.html", html.as_bytes())?;
    archive.add("events.csv", csv.as_bytes())?;
    archive.add(
        "events.json",
        serde_json::to_vec(&json!({"events":entries,"exported_at":crate::util::iso_now()}))?
            .as_slice(),
    )?;
    let manifest = std::mem::take(&mut archive.manifest);
    archive.add("manifest.sha256", manifest.as_bytes())?;
    let mut file = archive.zip.finish()?;
    file.rewind()?;
    Ok(file)
}

struct Archive {
    zip: zip::ZipWriter<File>,
    remaining: u64,
    manifest: String,
}

impl Archive {
    fn add(&mut self, name: &str, mut reader: impl Read) -> BuildResult<String> {
        self.zip.start_file(
            name,
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let length = reader.read(&mut buffer)?;
            if length == 0 {
                break;
            }
            self.remaining = self
                .remaining
                .checked_sub(length as u64)
                .ok_or_else(limit)?;
            hasher.update(&buffer[..length]);
            self.zip.write_all(&buffer[..length])?;
        }
        let hash = format!("{:x}", hasher.finalize());
        use std::fmt::Write as _;
        let _ = writeln!(self.manifest, "{hash}  {name}");
        Ok(hash)
    }
}

fn limit() -> Box<dyn std::error::Error + Send + Sync> {
    Error::other(SIZE_ERROR).into()
}

fn compare_hash(recorded: Option<&str>, archived: &str) -> &'static str {
    match recorded {
        None => "unrecorded",
        Some(expected) if expected.eq_ignore_ascii_case(archived) => "verified",
        Some(_) => "mismatch",
    }
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn cell(value: &str) -> String {
    if value.starts_with(['\t', '\r', '\n']) || value.trim_start().starts_with(['=', '+', '-', '@'])
    {
        csv_escape(&format!("'{value}"))
    } else {
        csv_escape(value)
    }
}
