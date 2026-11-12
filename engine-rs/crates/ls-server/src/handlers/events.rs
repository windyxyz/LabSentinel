// events endpoints + media/evidence (ZipWriter, evidence.zip) + WS entry.
// 2026-11-12 batch A split. All fns pub(crate) for router registration.
// 2026-11-12 batch A split. Unused items in this superset `use` are cleaned by cargo fix.
use crate::state::Shared;
use crate::util::*;
use axum::extract::{
    ws::{WebSocket, WebSocketUpgrade},
    Path as AxPath, Query, State,
};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};
use tokio::sync::broadcast;

pub(crate) async fn events(State(st): State<Shared>, Query(q): Query<EventsQuery>) -> Response {
    let s = match ls_store::Store::open(&st.db_path) {
        Ok(s) => s,
        Err(_) => return server_err(),
    };
    match s.events(
        q.limit.unwrap_or(50).min(200),
        q.offset.unwrap_or(0),
        q.camera_id,
        q.status.as_deref(),
    ) {
        Ok(rows) => Json(rows.iter().map(event_json).collect::<Vec<_>>()).into_response(),
        Err(_) => server_err(),
    }
}

#[derive(serde::Deserialize)]

pub(crate) struct EventsQuery {
    limit: Option<i64>,
    offset: Option<i64>,
    camera_id: Option<i64>,
    status: Option<String>,
}

pub(crate) fn event_json(e: &ls_store::Event) -> Value {
    json!({"id": e.id, "ts": e.ts, "camera_id": e.camera_id, "type": e.type_,
           "level": e.level, "track_id": e.track_id, "detail": e.detail,
           "has_snapshot": e.snapshot.is_some(), "has_video": e.video.is_some(),
           "status": e.status})
}

pub(crate) async fn events_count(State(st): State<Shared>) -> Json<Value> {
    let n = ls_store::Store::open(&st.db_path)
        .ok()
        .and_then(|s| s.count_events().ok())
        .unwrap_or(0);
    Json(json!({"count": n}))
}

pub(crate) async fn set_status(
    State(st): State<Shared>,
    AxPath(id): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    let status = body.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if !["new", "acked", "false_positive", "ignored"].contains(&status) {
        // T-505 契约对齐：Python 侧 EventStatusBody Literal → 422，两实现同契约
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "非法状态").into_response();
    }
    let s = match ls_store::Store::open(&st.db_path) {
        Ok(s) => s,
        Err(_) => return server_err(),
    };
    match s.set_event_status(id, status, None, None) {
        Ok(0) => detail_err(StatusCode::NOT_FOUND, "event not found").into_response(),
        Ok(_) => Json(json!({"ok": true})).into_response(),
        Err(_) => server_err(),
    }
}

pub(crate) async fn bulk_event_status(
    State(st): State<Shared>,
    Json(body): Json<Value>,
) -> Response {
    let status = body
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let ids: Vec<i64> = body
        .get("ids")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
        .unwrap_or_default();
    if !["new", "acked", "false_positive", "ignored"].contains(&status.as_str())
        || body
            .get("ids")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().any(|x| x.is_boolean()))
            .unwrap_or(false)
    {
        // 与 Python EventStatusBody Literal / bool 拒绝同契约 → 422
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "非法状态或 ids 含布尔")
            .into_response();
    }
    with_store(&st, |s| {
        let n = s.bulk_status(&ids, &status, "admin")?;
        Ok(Json(json!({"ok": true, "updated": n})).into_response())
    })
}

pub(crate) async fn events_timeline(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let days: i64 = q.get("days").and_then(|v| v.parse().ok()).unwrap_or(7);
    let cid: Option<i64> = q.get("camera_id").and_then(|v| v.parse().ok());
    let level: Option<i64> = q.get("level").and_then(|v| v.parse().ok());
    with_store(&st, |s| {
        let rows = s.events_timeline(days, cid, level)?;
        let days_list: Vec<Value> = rows
            .into_iter()
            .map(|(d, hours)| {
                let total: i64 = hours.iter().map(|h| h.1).sum();
                json!({"date": d, "total": total,
                       "hours": hours.into_iter().map(|(h, n, l3)|
                           json!({"hour": h, "total": n, "l3": l3})).collect::<Vec<_>>()})
            })
            .collect();
        Ok(Json(
            json!({"days": days_list.len(), "days_list": days_list}),
        ))
    })
}

pub(crate) fn serve_event_media(
    st: &Shared,
    eid: i64,
    col: fn(&ls_store::Event) -> &Option<String>,
    content_type: &str,
) -> Response {
    match ls_store::Store::open(&st.db_path) {
        Ok(s) => match s.event_by_id(eid) {
            Ok(Some(ev)) => {
                let rel = col(&ev).clone().unwrap_or_default();
                let p = std::path::Path::new(&st.data_dir).join(&rel);
                match std::fs::read(&p) {
                    Ok(bytes) => ([(header::CONTENT_TYPE, content_type)], bytes).into_response(),
                    Err(_) => {
                        detail_err(StatusCode::NOT_FOUND, "media file not found").into_response()
                    }
                }
            }
            _ => detail_err(StatusCode::NOT_FOUND, "event not found").into_response(),
        },
        Err(_) => server_err(),
    }
}

pub(crate) async fn event_snapshot(State(st): State<Shared>, AxPath(eid): AxPath<i64>) -> Response {
    serve_event_media(&st, eid, |e| &e.snapshot, "image/jpeg")
}

pub(crate) async fn event_video(State(st): State<Shared>, AxPath(eid): AxPath<i64>) -> Response {
    serve_event_media(&st, eid, |e| &e.video, "video/mp4")
}

pub(crate) async fn ws_events(State(st): State<Shared>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| handle_ws(socket, st.events_tx.subscribe()))
}

// ==== R2-309 批次 4：网页端告警渠道配置（任务140 复刻 + Fernet 互操作）+ 局域网发现 ====

/// 标准 ZIP（stored 无压缩）写出器：本地文件头 + central directory + EOCD。
/// CRC32（IEEE 802.3，表驱动）手写实现，零额外依赖——存证打包只有 jpeg/mp4，
/// stored 已足够（压缩收益小，可靠优先）。
pub(crate) struct ZipWriter {
    out: Vec<u8>,
    central: Vec<u8>,
}

impl ZipWriter {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            central: Vec::new(),
        }
    }

    fn crc32(data: &[u8]) -> u32 {
        static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
        let table = TABLE.get_or_init(|| {
            let mut t = [0u32; 256];
            for (i, e) in t.iter_mut().enumerate() {
                let mut c = i as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 {
                        0xEDB8_8320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
                *e = c;
            }
            t
        });
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c = table[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
        }
        !c
    }

    fn add_file(&mut self, name: &str, data: &[u8]) {
        let crc = Self::crc32(data);
        let offset = self.out.len() as u32;
        // 本地文件头：sig, ver(20), flags(0), method(stored), time/date(0), crc,
        // csize=size, usize=size, namelen, extralen(0)
        self.out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        self.out.extend_from_slice(&20u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&crc.to_le_bytes());
        self.out
            .extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.out
            .extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.out
            .extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(name.as_bytes());
        self.out.extend_from_slice(data);
        // central directory 条目
        self.central
            .extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        self.central.extend_from_slice(&20u16.to_le_bytes());
        self.central.extend_from_slice(&20u16.to_le_bytes());
        self.central.extend_from_slice(&0u16.to_le_bytes());
        self.central.extend_from_slice(&0u16.to_le_bytes());
        self.central.extend_from_slice(&0u16.to_le_bytes());
        self.central.extend_from_slice(&0u16.to_le_bytes());
        self.central.extend_from_slice(&crc.to_le_bytes());
        self.central
            .extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.central
            .extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.central
            .extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.central.extend_from_slice(&0u16.to_le_bytes()); // extra
        self.central.extend_from_slice(&0u16.to_le_bytes()); // comment
        self.central.extend_from_slice(&0u16.to_le_bytes()); // disk
        self.central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        self.central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        self.central.extend_from_slice(&offset.to_le_bytes());
        self.central.extend_from_slice(name.as_bytes());
    }

    fn finish(mut self) -> Vec<u8> {
        let cd_offset = self.out.len() as u32;
        let cd_size = self.central.len() as u32;
        let count = {
            // 条目数 = central 里 0x02014b50 出现次数
            self.central
                .windows(4)
                .filter(|w| *w == 0x0201_4b50u32.to_le_bytes())
                .count() as u16
        };
        self.out.extend_from_slice(&self.central);
        self.out.extend_from_slice(&0x0605_4b50u32.to_le_bytes()); // EOCD
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&count.to_le_bytes());
        self.out.extend_from_slice(&count.to_le_bytes());
        self.out.extend_from_slice(&cd_size.to_le_bytes());
        self.out.extend_from_slice(&cd_offset.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out
    }
}

pub(crate) fn read_media_file(st: &Shared, rel: &str) -> Option<(String, Vec<u8>)> {
    let p = std::path::Path::new(&st.data_dir).join(rel);
    let data = std::fs::read(&p).ok()?;
    let name = std::path::Path::new(rel)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    Some((name, data))
}

pub(crate) async fn event_evidence_zip(
    State(st): State<Shared>,
    AxPath(eid): AxPath<i64>,
) -> Response {
    with_store(&st, |s| {
        let Some(ev) = s.event_by_id(eid)? else {
            return Ok(detail_err(StatusCode::NOT_FOUND, "event not found").into_response());
        };
        let mut zip = ZipWriter::new();
        let mut parts = 0usize;
        if let Some(snap) = &ev.snapshot {
            if let Some((name, data)) = read_media_file(&st, snap) {
                zip.add_file(&format!("event-{eid}/{name}"), &data);
                parts += 1;
            }
        }
        if let Some(video) = &ev.video {
            if let Some((name, data)) = read_media_file(&st, video) {
                zip.add_file(&format!("event-{eid}/{name}"), &data);
                parts += 1;
            }
        }
        let meta = json!({
            "id": ev.id, "ts": ev.ts, "camera_id": ev.camera_id, "type": ev.type_,
            "level": ev.level, "track_id": ev.track_id, "detail": ev.detail,
            "status": ev.status,
        });
        zip.add_file(
            &format!("event-{eid}/evidence.json"),
            meta.to_string().as_bytes(),
        );
        if parts == 0 {
            return Ok(
                detail_err(StatusCode::NOT_FOUND, "事件无存证文件（快照/视频不存在）")
                    .into_response(),
            );
        }
        Ok(([(header::CONTENT_TYPE, "application/zip")], zip.finish()).into_response())
    })
}

pub(crate) async fn export_evidence_zip(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let parse_i64 = |k: &str| q.get(k).and_then(|v| v.parse().ok());
    let camera_id = parse_i64("camera_id");
    let days: i64 = q
        .get("days")
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
        .clamp(1, 365);
    let start = iso_days_ago(days - 1) + "T00:00:00";
    with_store(&st, |s| {
        let rows = s.events_query(&ls_store::EventsFilter {
            camera_id,
            start_ts: Some(&start),
            limit: 2000,
            ..Default::default()
        })?;
        let with_media: Vec<_> = rows
            .into_iter()
            .filter(|e| e.snapshot.is_some() || e.video.is_some())
            .collect();
        if with_media.is_empty() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "范围内无带存证的事件").into_response());
        }
        let mut zip = ZipWriter::new();
        let mut index = Vec::new();
        for e in &with_media {
            let mut files = 0usize;
            if let Some(snap) = &e.snapshot {
                if let Some((name, data)) = read_media_file(&st, snap) {
                    zip.add_file(&format!("event-{}/{}", e.id, name), &data);
                    files += 1;
                }
            }
            if let Some(video) = &e.video {
                if let Some((name, data)) = read_media_file(&st, video) {
                    zip.add_file(&format!("event-{}/{}", e.id, name), &data);
                    files += 1;
                }
            }
            index.push(json!({"id": e.id, "ts": e.ts, "camera_id": e.camera_id,
                "type": e.type_, "level": e.level, "detail": e.detail, "files": files}));
        }
        zip.add_file(
            "export-index.json",
            json!({"exported": index.len(), "events": index})
                .to_string()
                .as_bytes(),
        );
        Ok(([(header::CONTENT_TYPE, "application/zip")], zip.finish()).into_response())
    })
}

pub(crate) async fn handle_ws(mut socket: WebSocket, mut rx: broadcast::Receiver<String>) {
    use axum::extract::ws::Message;
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(text) => {
                        if socket.send(Message::Text(text)).await.is_err() { break; }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue, // 慢客户端丢弃
                    Err(_) => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
}
