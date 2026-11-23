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
use axum::Extension;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::broadcast;

pub(crate) async fn events(
    State(st): State<Shared>,
    axum::Extension(browser): axum::Extension<crate::access::BrowserAccess>,
    Query(q): Query<EventsQuery>,
) -> Response {
    let s = match ls_store::Store::open(&st.db_path) {
        Ok(s) => s,
        Err(_) => return server_err(),
    };
    match s.events_query(&q.filter(&browser, &ls_core::time::local_today_iso())) {
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
    level: Option<i64>,
    #[serde(rename = "type")]
    type_: Option<String>,
    category: Option<String>,
    start_ts: Option<String>,
    end_ts: Option<String>,
    today: Option<u8>,
    q: Option<String>,
    ai_verdict: Option<String>,
}

impl EventsQuery {
    pub(super) fn filter<'a>(
        &'a self,
        browser: &'a crate::access::BrowserAccess,
        today: &'a str,
    ) -> ls_store::EventsFilter<'a> {
        let day = (self.today == Some(1)).then_some(today);
        ls_store::EventsFilter {
            camera_id: self.camera_id,
            status: self.status.as_deref(),
            level: self.level,
            type_: self.type_.as_deref(),
            category: self.category.as_deref(),
            start_ts: day.or(self.start_ts.as_deref()),
            end_ts: day.or(self.end_ts.as_deref()),
            search: self.q.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            ai_verdict: self.ai_verdict.as_deref(),
            allowed_cameras: browser.device.as_ref().and_then(|d| d.cameras.as_deref()),
            limit: self.limit.unwrap_or(50).clamp(0, 200),
            offset: self.offset.unwrap_or(0).max(0),
        }
    }
}

pub(crate) fn event_json(e: &ls_store::Event) -> Value {
    let vlm_verdict = e
        .vlm_verdict
        .as_deref()
        .and_then(|v| serde_json::from_str::<Value>(v).ok());
    let background_review = e
        .ai_review
        .as_deref()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .filter(|review| recognized_verdict(review).is_some());
    let review = background_review.as_ref().or(vlm_verdict.as_ref());
    let ai_verdict = review.and_then(recognized_verdict);
    let (review_ts, review_model) = if background_review.is_some() {
        (e.ai_review_ts.as_deref(), e.ai_review_model.as_deref())
    } else {
        (e.vlm_ts.as_deref(), None)
    };
    json!({"id": e.id, "ts": e.ts, "camera_id": e.camera_id, "type": e.type_,
           "category": if matches!(e.type_.as_str(), "photo_check" | "video_check") { "check" } else { "alert" },
           "level": e.level, "track_id": e.track_id, "detail": e.detail,
           "has_snapshot": e.snapshot.is_some(), "has_video": e.video.is_some(),
           "has_overlay": e.detections_json.is_some(), "snapshot_sha256": e.snapshot_sha256,
           "status": e.status, "handled_by": e.handled_by, "handled_ts": e.handled_ts,
           "ai_verdict": ai_verdict, "ai_review": review,
           "ai_review_ts": review_ts, "ai_review_model": review_model,
           "vlm_suppressed": e.vlm_suppressed != 0,
           "vlm_ts": e.vlm_ts,
           "vlm_verdict": vlm_verdict})
}

fn recognized_verdict(review: &Value) -> Option<&str> {
    review
        .get("verdict")?
        .as_str()
        .filter(|verdict| matches!(*verdict, "likely_true" | "likely_false" | "uncertain"))
}

pub(crate) async fn events_count(
    State(st): State<Shared>,
    Extension(browser): Extension<crate::access::BrowserAccess>,
    Query(q): Query<EventsQuery>,
) -> Response {
    with_store(&st, |store| {
        let count =
            store.events_query_count(&q.filter(&browser, &ls_core::time::local_today_iso()))?;
        Ok(Json(json!({"count": count})))
    })
}

pub(crate) async fn event_notes(State(st): State<Shared>, AxPath(id): AxPath<i64>) -> Response {
    with_store(&st, |store| {
        if store.event_by_id(id)?.is_none() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "事件不存在").into_response());
        }
        Ok(Json(store.event_notes(id)?).into_response())
    })
}

#[derive(serde::Deserialize)]
pub(crate) struct NoteInput {
    text: String,
}

pub(crate) async fn add_event_note(
    State(st): State<Shared>,
    Extension(crate::middleware::RequestActor(actor)): Extension<crate::middleware::RequestActor>,
    AxPath(id): AxPath<i64>,
    Json(input): Json<NoteInput>,
) -> Response {
    let note = input.text.trim();
    if note.is_empty()
        || note.chars().count() > 500
        || note
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "交接备注需填写 1–500 字")
            .into_response();
    }
    with_store(&st, |store| {
        match store.add_event_note(id, &actor, note)? {
            Some(note) => Ok((StatusCode::CREATED, Json(note)).into_response()),
            None => Ok(detail_err(StatusCode::NOT_FOUND, "事件不存在").into_response()),
        }
    })
}

pub(crate) async fn set_status(
    State(st): State<Shared>,
    Extension(crate::middleware::RequestActor(actor)): Extension<crate::middleware::RequestActor>,
    AxPath(id): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    let status = body
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    if !["new", "acked", "false_positive", "ignored"].contains(&status.as_str()) {
        // T-505 契约对齐：Python 侧 EventStatusBody Literal → 422，两实现同契约
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "非法状态").into_response();
    }
    tokio::task::spawn_blocking(move || with_store(&st, |s| {
        let now = iso_now();
        let handled = status != "new";
        if s.set_event_status(id, &status, handled.then_some(actor.as_str()), handled.then_some(now.as_str()))? == 0 {
            return Ok(detail_err(StatusCode::NOT_FOUND, "事件不存在").into_response());
        }
        let mut result = json!({"ok": true, "handled_by": handled.then_some(actor), "handled_ts": handled.then_some(now)});
        if status == "false_positive" {
            match super::dataset::stage_event_sample(&st, s, id, false) {
                Ok(sample) => result["sample_id"] = json!(sample),
                Err((_, error)) => result["sample_error"] = json!(error),
            }
        }
        Ok(Json(result).into_response())
    })).await.unwrap_or_else(|_| server_err())
}

pub(crate) async fn bulk_event_status(
    State(st): State<Shared>,
    Extension(crate::middleware::RequestActor(actor)): Extension<crate::middleware::RequestActor>,
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
    tokio::task::spawn_blocking(move || {
        with_store(&st, |s| {
            let n = s.bulk_status(&ids, &status, &actor)?;
            let skipped = ids.len().saturating_sub(n);
            let mut sample_saved = 0;
            let mut sample_missing = 0;
            if status == "false_positive" {
                let unique: std::collections::BTreeSet<_> = ids.iter().copied().collect();
                for id in unique {
                    if super::dataset::stage_event_sample(&st, s, id, false).is_ok() {
                        sample_saved += 1;
                    } else {
                        sample_missing += 1;
                    }
                }
            }
            Ok(Json(json!({
                "ok": true,
                "updated": n,
                "skipped": skipped,
                "missing": [],
                "failed": [],
                "samples_saved": sample_saved,
                "samples_unavailable": sample_missing,
            }))
            .into_response())
        })
    })
    .await
    .unwrap_or_else(|_| server_err())
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
                let Some(p) = rooted_file(
                    std::path::Path::new(&st.data_dir),
                    std::path::Path::new(&rel),
                ) else {
                    return detail_err(StatusCode::NOT_FOUND, "media file not found")
                        .into_response();
                };
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

pub(super) fn event_image_path(
    st: &Shared,
    store: &ls_store::Store,
    event: &ls_store::Event,
) -> Option<std::path::PathBuf> {
    use std::path::Path;
    if let Some(file) = event
        .snapshot
        .as_deref()
        .and_then(|path| rooted_file(Path::new(&st.data_dir), Path::new(path)))
    {
        return Some(file);
    }
    let path = store.check_image_path(event.id).ok().flatten()?;
    let folder = store
        .camera(event.camera_id)
        .ok()
        .flatten()
        .filter(|c| c.type_ == "folder")?;
    rooted_file(Path::new(&folder.source), Path::new(&path))
}

pub(crate) async fn event_snapshot(State(st): State<Shared>, AxPath(eid): AxPath<i64>) -> Response {
    with_store(&st, |store| {
        let file = store
            .event_by_id(eid)?
            .and_then(|event| event_image_path(&st, store, &event));
        let Some(bytes) = file.and_then(|path| std::fs::read(path).ok()) else {
            return Ok(
                detail_err(StatusCode::NOT_FOUND, "该事件的原图或截图不可用").into_response(),
            );
        };
        let content_type = match image::guess_format(&bytes) {
            Ok(image::ImageFormat::Png) => "image/png",
            Ok(image::ImageFormat::WebP) => "image/webp",
            Ok(image::ImageFormat::Bmp) => "image/bmp",
            Ok(image::ImageFormat::Tiff) => "image/tiff",
            _ => "image/jpeg",
        };
        Ok(([(header::CONTENT_TYPE, content_type)], bytes).into_response())
    })
}

pub(crate) async fn event_video(State(st): State<Shared>, AxPath(eid): AxPath<i64>) -> Response {
    serve_event_media(&st, eid, |e| &e.video, "video/mp4")
}

pub(crate) async fn event_overlay(State(st): State<Shared>, AxPath(eid): AxPath<i64>) -> Response {
    with_store(&st, |store| {
        let metadata = store
            .event_by_id(eid)?
            .and_then(|event| event.detections_json)
            .and_then(|text| serde_json::from_str::<Value>(&text).ok());
        Ok(Json(metadata).into_response())
    })
}

/// LS-050：事件缩略图（宽 320px JPEG）——按需生成并缓存到 `data/thumbs/ev_<id>.jpg`，
/// 列表页使用较小的截图；缓存缺失时在阻塞线程内解码和缩放。
pub(crate) async fn event_thumb(State(st): State<Shared>, AxPath(eid): AxPath<i64>) -> Response {
    let shared = st.clone();
    match tokio::task::spawn_blocking(move || load_event_thumb(&shared, eid)).await {
        Ok(Some(bytes)) => ([(header::CONTENT_TYPE, "image/jpeg")], bytes).into_response(),
        _ => event_snapshot(State(st), AxPath(eid)).await,
    }
}

fn load_event_thumb(st: &Shared, eid: i64) -> Option<Vec<u8>> {
    let src = ls_store::Store::open(&st.db_path).ok().and_then(|store| {
        store
            .event_by_id(eid)
            .ok()
            .flatten()
            .and_then(|event| event_image_path(st, &store, &event))
    })?;
    let dir = std::path::Path::new(&st.data_dir).join("thumbs");
    let cache = dir.join(format!("ev_{eid}.jpg"));
    if let Ok(bytes) = std::fs::read(&cache) {
        return Some(bytes);
    }
    let raw = ls_jobs::media::read_snapshot(&src)?;
    let thumbnail = ls_jobs::media::decode_image(&raw)?.thumbnail(320, 180);
    let jpeg = ls_jobs::media::encode_jpeg(&thumbnail, 82)?;
    if std::fs::create_dir_all(&dir).is_ok() {
        // 原子写：先写 .tmp 再改名，避免并发读到半截文件
        let tmp = dir.join(format!("ev_{eid}_{}.jpg.tmp", uuid_like()));
        if std::fs::write(&tmp, &jpeg)
            .and_then(|()| std::fs::rename(&tmp, &cache))
            .is_err()
        {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    Some(jpeg)
}

/// P0-1 修复（安全审计）：WS 握手校验——此前零鉴权（CSWSH：恶意网页可
/// drive-by 读取事件流并绕过读保护）。语义对齐 Python `_ws_origin_ok` +
/// `_ws_auth_check`：
/// - 存在 Origin 时必须与 Host 同源或在 allowed_origins 白名单（无 Origin =
///   非浏览器客户端，放行）；
/// - read_protected 时校验令牌（子协议 bearer.<token> 优先 → Authorization
///   Bearer），admin/read/设备令牌均可（与 HTTP 读口径一致）。
pub(super) async fn socket_revoked(
    st: &Shared,
    browser: &crate::access::BrowserAccess,
    token: Option<&str>,
) {
    if browser.device.is_none() && crate::middleware::is_api_reader(st, token) {
        return std::future::pending().await;
    }
    loop {
        let valid = if let Some(device) = &browser.device {
            let st = st.clone();
            let expected = device.clone();
            tokio::task::spawn_blocking(move || {
                super::pairing::paired_device(&st, &expected.digest).as_ref() == Some(&expected)
            })
            .await
            .unwrap_or_default()
        } else if let Some(token) = token {
            let st = st.clone();
            let token = token.to_owned();
            tokio::task::spawn_blocking(move || {
                crate::middleware::check_device_token(&st, &token).is_some()
            })
            .await
            .unwrap_or(false)
        } else if let Some(digest) = &browser.session {
            st.access.session_valid(digest, browser.client)
        } else {
            !st.access.required(browser.client)
        };
        if !valid {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

pub(crate) async fn ws_events(
    State(st): State<Shared>,
    axum::Extension(browser): axum::Extension<crate::access::BrowserAccess>,
    Query(query): Query<ReplayQuery>,
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let replay = match query.since.as_deref() {
        None => None,
        Some("latest") => Some(None),
        Some(value) => match value.parse::<i64>() {
            Ok(id) if id >= 0 => Some(Some(id)),
            _ => {
                return detail_err(StatusCode::BAD_REQUEST, "since 必须是非负事件编号或 latest")
                    .into_response()
            }
        },
    };
    let token = crate::middleware::ws_token(&headers);
    let rx = st.events_tx.subscribe();
    ws.protocols(["labsentinel"])
        .on_upgrade(move |socket| async move {
            let stream = async {
                if let Some(since) = replay {
                    let cameras = browser
                        .device
                        .as_ref()
                        .and_then(|device| device.cameras.clone());
                    handle_replay(socket, rx, st.db_path.clone(), cameras, since).await;
                } else {
                    handle_ws(socket, rx, browser.device.as_ref()).await;
                }
            };
            tokio::select! {
                _ = stream => {},
                _ = socket_revoked(&st, &browser, token.as_deref()) => {},
            }
        })
}

#[derive(serde::Deserialize)]
pub(crate) struct ReplayQuery {
    since: Option<String>,
}

async fn handle_replay(
    mut socket: WebSocket,
    mut rx: broadcast::Receiver<String>,
    db_path: String,
    cameras: Option<Vec<i64>>,
    since: Option<i64>,
) {
    use axum::extract::ws::Message;

    const PAGE: i64 = 64;
    let db = db_path.clone();
    let scope = cameras.clone();
    let latest = tokio::task::spawn_blocking(move || {
        ls_store::Store::open(&db)?.latest_event_id(scope.as_deref())
    })
    .await;
    let Ok(Ok(latest)) = latest else { return };
    let mut after = since.map_or(latest, |cursor| cursor.min(latest));
    if socket
        .send(Message::Text(json!({ "stream_cursor": after }).to_string()))
        .await
        .is_err()
    {
        return;
    }
    let mut poll = tokio::time::interval(std::time::Duration::from_secs(2));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut more = true;
    loop {
        // The broadcast is a wake-up. Reading committed IDs in order makes reconnection,
        // concurrent producers, and lagged receivers use the same cursor.
        if more {
            let db = db_path.clone();
            let scope = cameras.clone();
            let batch = tokio::task::spawn_blocking(move || {
                ls_store::Store::open(&db)?.events_after_id(after, scope.as_deref(), PAGE)
            })
            .await;
            let Ok(Ok(batch)) = batch else { return };
            more = batch.len() == PAGE as usize;
            for event in batch {
                if socket
                    .send(Message::Text(event_json(&event).to_string()))
                    .await
                    .is_err()
                {
                    return;
                }
                after = event.id;
            }
        }
        tokio::select! {
            message = socket.recv() => if matches!(message, None | Some(Err(_)) | Some(Ok(Message::Close(_)))) { return },
            message = rx.recv() => {
                if matches!(message, Err(broadcast::error::RecvError::Closed)) { return; }
                more = true;
            },
            _ = poll.tick() => more = true,
            _ = tokio::task::yield_now(), if more => {},
        }
    }
}

/// LS-051：`/ws/detections/:cid` 实时检测元数据广播（受 trusted_hosts + 读令牌守卫，
/// 与 `/ws/events` 同鉴权口径）。客户端慢消费时 `handle_ws` 的 Lagged 自动丢弃，
/// 不阻塞推理线程。
pub(crate) async fn ws_detections(
    State(st): State<Shared>,
    AxPath(cid): AxPath<i64>,
    axum::Extension(browser): axum::Extension<crate::access::BrowserAccess>,
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let channel = st.detections.subscribe(cid);
    let rx = channel.subscribe();
    let token = crate::middleware::ws_token(&headers);
    ws.protocols(["labsentinel"])
        .on_upgrade(move |socket| async move {
            tokio::select! {
                _ = handle_detections_ws(socket, rx) => {},
                _ = socket_revoked(&st, &browser, token.as_deref()) => {},
            }
            drop(channel);
        })
}

async fn handle_detections_ws(
    socket: axum::extract::ws::WebSocket,
    mut rx: tokio::sync::broadcast::Receiver<String>,
) {
    use axum::extract::ws::Message;
    let (mut sender, mut receiver) = socket.split();
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(raw) => {
                        if sender.send(Message::Text(raw)).await.is_err() { break; }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
            incoming = receiver.next() => {
                if incoming.is_none() { break; } // 客户端关闭
            }
        }
    }
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
    pub(crate) fn new() -> Self {
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

    pub(crate) fn add_file(&mut self, name: &str, data: &[u8]) {
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

    pub(crate) fn finish(mut self) -> Vec<u8> {
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
        if let Some(file) = event_image_path(&st, s, &ev) {
            let name = file.file_name().unwrap_or_default().to_string_lossy();
            if let Ok(data) = std::fs::read(&file) {
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
            "snapshot_sha256": ev.snapshot_sha256,
            "detections": ev.detections_json.as_deref().and_then(|text| serde_json::from_str::<Value>(text).ok()),
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

async fn handle_ws(
    mut socket: WebSocket,
    mut rx: broadcast::Receiver<String>,
    device: Option<&super::pairing::PairedDevice>,
) {
    use axum::extract::ws::Message;
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(text) => {
                        if let Some(device) = device {
                            let visible = serde_json::from_str::<Value>(&text).ok()
                                .and_then(|event| event.get("camera_id")?.as_i64())
                                .is_some_and(|id| device.allows_camera(id));
                            if !visible { continue; }
                        }
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
