// cameras endpoints (CRUD/group/analyzer/start/bulk/usb/copy-config/snapshot).
// 2026-11-12 batch A split. All fns pub(crate) for router registration.
// 2026-11-12 batch A split. Unused items in this superset `use` are cleaned by cargo fix.
use crate::state::Shared;
use crate::util::*;
use axum::extract::{Path as AxPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};

pub(crate) async fn cameras(
    State(st): State<Shared>,
    axum::Extension(browser): axum::Extension<crate::access::BrowserAccess>,
) -> Response {
    let s = match ls_store::Store::open(&st.db_path) {
        Ok(s) => s,
        Err(_) => return server_err(),
    };
    match s.cameras() {
        Ok(rows) => Json(
            rows.iter()
                .filter(|camera| browser.device.as_ref().is_none_or(|device| device.allows_camera(camera.id)))
                .map(|c| {
                    let mut value = json!({"id": c.id, "name": c.name, "type": c.type_, "status": c.status,
                        "group_name": c.group_name, "analyzer": c.analyzer});
                    if browser.device.is_none() && !browser.share {
                        value["source"] = json!(redact_camera_source(&c.source));
                        value["analysis_mode"] = json!(if !c.analysis_source.is_empty() {
                            "substream"
                        } else if c.analysis_shared {
                            "relay"
                        } else {
                            "main"
                        });
                    }
                    value
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => server_err(),
    }
}

fn redact_camera_source(source: &str) -> String {
    let Some((scheme, rest)) = source.split_once("://") else {
        return source.to_string();
    };
    let host = rest.split('/').next().unwrap_or(rest);
    if let Some((_, suffix)) = host.rsplit_once('@') {
        return format!("{scheme}://***@{suffix}{}", &rest[host.len()..]);
    }
    source.to_string()
}

pub(crate) fn valid_substream(source: &str) -> bool {
    source.len() <= 2048
        && !source.chars().any(char::is_control)
        && url::Url::parse(source).is_ok_and(|url| {
            matches!(url.scheme(), "rtsp" | "rtsps")
                && url.host().is_some()
                && url.fragment().is_none()
        })
}

pub(crate) async fn set_analysis_source(
    State(st): State<Shared>,
    AxPath(cid): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    with_store(&st, |store| {
        let Some(camera) = store.camera(cid)? else {
            return Ok(detail_err(StatusCode::NOT_FOUND, "摄像头不存在").into_response());
        };
        if camera.type_ != "rtsp" {
            return Ok(detail_err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "分析码流设置适用于 RTSP 摄像头",
            )
            .into_response());
        }
        let (mode, source, shared) = match body.get("mode").and_then(Value::as_str) {
            Some("main") => ("main", "", false),
            Some("relay") => ("relay", "", true),
            Some("substream") => {
                let source = body
                    .get("source")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&camera.analysis_source);
                if !valid_substream(source) {
                    return Ok(detail_err(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "请填写有效的 RTSP 或 RTSPS 子码流地址",
                    )
                    .into_response());
                }
                ("substream", source, false)
            }
            _ => {
                return Ok(detail_err(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "请选择主码流、共享中继或子码流",
                )
                .into_response())
            }
        };
        if store.set_camera_analysis_source(cid, source, shared)? == 0 {
            return Ok(detail_err(StatusCode::NOT_FOUND, "摄像头不存在").into_response());
        }
        let restarted = crate::worker::restart_camera_if_running(&st, store, cid);
        Ok(Json(json!({"id": cid, "analysis_mode": mode, "restarted": restarted})).into_response())
    })
}

pub(crate) async fn camera_status(State(st): State<Shared>, AxPath(id): AxPath<i64>) -> Response {
    let s = match ls_store::Store::open(&st.db_path) {
        Ok(s) => s,
        Err(_) => return server_err(),
    };
    match s.camera(id) {
        Ok(Some(c)) => {
            let (frame_age, last_frame, reconnects) = capture_health(&st, id);
            let since = s.camera_outage_since(id).ok().flatten();
            let processed_frames = (c.type_ == "file")
                .then(|| {
                    let worker = st.workers.get(id)?;
                    let metrics = worker.performance.lock().ok()?;
                    Some(metrics.frames())
                })
                .flatten();
            Json(json!({"id": c.id, "name": c.name, "type": c.type_,
                "status": c.status, "analyzer": c.analyzer, "fps": null,
                "offline_since": since, "frame_age_s": frame_age, "last_frame_at": last_frame,
                "reconnect_count": reconnects, "processed_frames": processed_frames}))
            .into_response()
        }
        Ok(None) => detail_err(StatusCode::NOT_FOUND, "camera not found").into_response(),
        Err(_) => server_err(),
    }
}

pub(super) fn capture_health(st: &Shared, cid: i64) -> (Option<f64>, Option<String>, u64) {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    let Some(worker) = st.workers.get(cid) else {
        return (None, None, 0);
    };
    let last = worker
        .latest_jpeg
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|s| s.captured_at_ms));
    let captured =
        last.and_then(|ms| UNIX_EPOCH.checked_add(Duration::from_millis(u64::try_from(ms).ok()?)));
    let age = captured
        .and_then(|time| SystemTime::now().duration_since(time).ok())
        .map(|diff| diff.as_secs_f64());
    let ts = captured.map(ls_core::time::format_system_time_local_iso);
    let reconnects = worker
        .performance
        .lock()
        .map(|m| m.reconnects())
        .unwrap_or(0);
    (age, ts, reconnects)
}

#[derive(serde::Deserialize)]
struct DahuaSource {
    host: String,
    port: u16,
    channel: u16,
    username: String,
    password: String,
    substream: bool,
}

impl DahuaSource {
    fn urls(&self) -> Option<(String, String)> {
        let host = url::Host::parse(self.host.trim()).ok()?;
        if self.port == 0
            || self.channel == 0
            || self.username.trim().is_empty()
            || self.password.is_empty()
            || self
                .username
                .chars()
                .chain(self.password.chars())
                .any(char::is_control)
        {
            return None;
        }
        fn escape(value: &str) -> String {
            use std::fmt::Write;
            let mut encoded = String::new();
            for byte in value.bytes() {
                if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                    encoded.push(char::from(byte));
                } else {
                    let _ = write!(encoded, "%{byte:02X}");
                }
            }
            encoded
        }
        let base = format!(
            "rtsp://{}:{}@{host}:{}/cam/realmonitor?channel={}&subtype=",
            escape(&self.username),
            escape(&self.password),
            self.port,
            self.channel
        );
        let main = format!("{base}0");
        valid_substream(&main).then(|| {
            let sub = if self.substream {
                format!("{base}1")
            } else {
                String::new()
            };
            (main, sub)
        })
    }
}

#[cfg(test)]
mod dahua_tests {
    use super::DahuaSource;

    #[test]
    fn recorder_channel_and_credentials_are_encoded() {
        let mut config = DahuaSource {
            host: "192.168.1.10".into(),
            port: 8554,
            channel: 7,
            username: "lab@a".into(),
            password: "p:#/%".into(),
            substream: true,
        };
        let (main, sub) = config.urls().unwrap();
        assert_eq!(
            main,
            "rtsp://lab%40a:p%3A%23%2F%25@192.168.1.10:8554/cam/realmonitor?channel=7&subtype=0"
        );
        assert_eq!(sub.strip_suffix('1'), main.strip_suffix('0'));
        config.host = "192.168.1.10/other".into();
        assert!(config.urls().is_none());
    }
}

pub(crate) async fn add_camera(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    // T-103 等价：source 白名单（白名单外拒绝；file 源不强制存在，worker 报 error）
    let type_ = body.get("type").and_then(|v| v.as_str()).unwrap_or("file");
    let mut source = body
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut analysis_source = String::new();
    if let Some(dahua) = body.get("dahua") {
        let urls = serde_json::from_value::<DahuaSource>(dahua.clone())
            .ok()
            .and_then(|config| config.urls());
        let Some((main, sub)) = urls.filter(|_| type_ == "rtsp") else {
            return detail_err(
                StatusCode::BAD_REQUEST,
                "请检查大华设备地址、端口、通道和设备账号",
            )
            .into_response();
        };
        source = main;
        analysis_source = sub;
    }
    // USB（2026-11-21 dshow 化）：入库前归一化为 ffmpeg 可用形态——
    // ① 裸数字（历史索引/手填）→ 按枚举表映射为 `video=<设备名>`（枚举失败
    //    如实 400，不收不可取流的 source）；
    // ② 枚举下拉/手填设备名由前端补 `video=` 前缀；`/dev/video*`（linux v4l2）直通。
    if type_ == "usb" {
        let is_digits = !source.is_empty() && source.chars().all(|c| c.is_ascii_digit());
        if is_digits {
            let idx: usize = source.parse().unwrap_or(usize::MAX);
            match ls_ingest::list_usb_devices() {
                Ok(ds) => match ds.get(idx) {
                    Some(d) => source = d.source.clone(),
                    None => {
                        return detail_err(
                            StatusCode::BAD_REQUEST,
                            &format!("USB 设备索引 {idx} 不存在（本机枚举到 {} 台）", ds.len()),
                        )
                        .into_response()
                    }
                },
                Err(e) => {
                    return detail_err(
                        StatusCode::BAD_REQUEST,
                        &format!("无法枚举 USB 设备：{e}；请改填设备名（video=<名称>）"),
                    )
                    .into_response()
                }
            }
        }
    }
    let valid = match type_ {
        "rtsp" => source.starts_with("rtsp://") || source.starts_with("rtsps://"),
        "file" => [".mp4", ".mkv", ".avi", ".mov", ".ts", ".flv"]
            .iter()
            .any(|e| source.to_ascii_lowercase().ends_with(e)),
        "usb" => {
            (source.starts_with("video=") && source.len() > 6)
                || source.starts_with("/dev/video")
                || (!source.is_empty() && source.chars().all(|c| c.is_ascii_digit()))
        }
        "folder" => !source.is_empty(),
        _ => false,
    };
    if !valid || source.contains('#') || source.contains('\n') {
        return detail_err(StatusCode::BAD_REQUEST, "source 非法").into_response();
    }
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("cam");
    let group_name = body
        .get("group_name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    let autostart = body
        .get("autostart")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    match ls_store::Store::open(&st.db_path) {
        Ok(s) => {
            match s.add_camera_with_sources(name, &source, type_, group_name, &analysis_source) {
                Ok(id) => {
                    if type_ == "rtsp" {
                        st.go2rtc.refresh();
                    }
                    let mut status = "stopped";
                    if autostart {
                        if let Ok(Some(cfg)) = crate::worker::WorkerConfig::from_db(&s, id) {
                            if crate::worker::start_worker(&st, cfg).is_ok() {
                                status = "starting";
                            }
                        }
                    }
                    Json(json!({"id": id, "name": name, "source": if body.get("dahua").is_some() { "" } else { &source },
                            "type": type_, "group_name": group_name, "status": status}))
                .into_response()
                }
                Err(_) => server_err(),
            }
        }
        Err(_) => server_err(),
    }
}

pub(crate) async fn set_camera_group(
    State(st): State<Shared>,
    AxPath(cid): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    let group = body
        .get("group_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    with_store(&st, |s| {
        if s.camera(cid)?.is_none() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response());
        }
        s.set_camera_group(cid, &group)?;
        Ok(Json(json!({"id": cid, "group_name": group})).into_response())
    })
}

pub(crate) async fn set_camera_analyzer(
    State(st): State<Shared>,
    AxPath(cid): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    let impl_ = body
        .get("analyzer")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // RA5（模型模块化）：rust 形态新增 "vlm"——外部联网模型后端；
    // UX2（2026-11-20）：扩展 "cascade"（本地+API 协作）与 "default"（跟随
    // 全局策略——detect.mode，PUT /api/detect/config 后软重启即时生效）。
    // 历史 "python" 保留兼容（按 rust 语义存储）。
    let normalized = if impl_ == "python" {
        "rust".to_string()
    } else {
        impl_.clone()
    };
    if !["rust", "vlm", "cascade", "default"].contains(&normalized.as_str()) {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "analyzer 必须是 rust|vlm|cascade|default（python 兼容映射 rust）",
        )
        .into_response();
    }
    with_store(&st, |s| {
        let cam = match s.camera(cid)? {
            Some(c) => c,
            None => {
                return Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response())
            }
        };
        if cam.analyzer == normalized {
            return Ok(
                Json(json!({"id": cid, "analyzer": normalized, "restarted": false}))
                    .into_response(),
            );
        }
        s.set_camera_analyzer(cid, &normalized)?;
        // UX2：运行中的相机软重启即时生效（不再要求手工重启该路）
        let restarted = crate::worker::restart_camera_if_running(&st, s, cid);
        Ok(
            Json(json!({"id": cid, "analyzer": normalized, "restarted": restarted}))
                .into_response(),
        )
    })
}

pub(crate) async fn start_camera(State(st): State<Shared>, AxPath(cid): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.camera(cid)?.is_none() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response());
        }
        // RA1.2（ADR-0011）：进程内 worker 真实现（帧循环在专用线程；启动失败
        // 退避重连，超限写 error）。响应契约 = Python `{"ok": true}`。
        match crate::worker::WorkerConfig::from_db(s, cid) {
            Err(e) => Ok(detail_err(StatusCode::INTERNAL_SERVER_ERROR, &e).into_response()),
            Ok(None) => Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response()),
            Ok(Some(cfg)) => match crate::worker::start_worker(&st, cfg) {
                Ok(()) => Ok(Json(json!({"ok": true})).into_response()),
                Err(e) if e == "已在运行" => Ok(Json(json!({"ok": true})).into_response()),
                Err(e) => Ok(detail_err(StatusCode::INTERNAL_SERVER_ERROR, &e).into_response()),
            },
        }
    })
}

pub(crate) async fn camera_webrtc(
    State(st): State<Shared>,
    AxPath(cid): AxPath<i64>,
    axum::Extension(browser): axum::Extension<crate::access::BrowserAccess>,
    headers: axum::http::HeaderMap,
) -> Response {
    with_store(&st, |s| match s.camera(cid)? {
        None => Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response()),
        Some(c) if c.type_ == "folder" => Ok(detail_err(
            StatusCode::BAD_REQUEST,
            "图片文件夹源没有实时流；请在「图片识别」页查看关键图片",
        )
        .into_response()),
        Some(c) => {
            // 为可撤销的浏览器会话和访客保留服务端媒体连接。
            let token = crate::middleware::provided_token(&headers);
            let direct =
                browser.device.is_none() && crate::middleware::is_api_admin(&st, token.as_deref());
            let (mode, suffix) = match (
                c.type_ == "rtsp" && st.go2rtc.address_for_camera(cid).is_some(),
                direct,
            ) {
                (true, false) => ("mse", "?transport=mse"),
                (true, true) => ("webrtc", ""),
                (false, _) => ("snapshot", ""),
            };
            Ok(Json(json!({
                "camera_id": cid,
                "mode": mode,
                "ws_url": format!("/ws/webrtc/{cid}{suffix}"),
            }))
            .into_response())
        }
    })
}

/// GET /ws/webrtc/:cid——WebRTC 信令代理到本机 go2rtc（LS-042 决策 A）。
/// go2rtc 不可用时返回 503（前端应先看 /api/cameras/:cid/webrtc 的 mode 字段自动降级快照轮播）。
pub(crate) async fn ws_webrtc(
    State(st): State<Shared>,
    AxPath(cid): AxPath<i64>,
    Query(query): Query<std::collections::HashMap<String, String>>,
    axum::Extension(browser): axum::Extension<crate::access::BrowserAccess>,
    headers: axum::http::HeaderMap,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> Response {
    let token = crate::middleware::ws_token(&headers);
    let mse = query.get("transport").is_some_and(|mode| mode == "mse");
    if !mse && (browser.device.is_some() || !crate::middleware::is_api_admin(&st, token.as_deref()))
    {
        return detail_err(StatusCode::FORBIDDEN, "此会话需使用中继播放").into_response();
    }
    let Some(addr) = st.go2rtc.address_for_camera(cid) else {
        return detail_err(
            StatusCode::SERVICE_UNAVAILABLE,
            "go2rtc 未运行，已降级为快照轮播",
        )
        .into_response();
    };
    let exists = ls_store::Store::open(&st.db_path)
        .ok()
        .and_then(|s| s.camera(cid).ok().flatten())
        .is_some_and(|c| c.type_ == "rtsp");
    if !exists {
        return detail_err(StatusCode::NOT_FOUND, "camera not found").into_response();
    }
    let upstream_url = format!("ws://{addr}/api/ws?src=cam_{cid}");
    let wants_proto = headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').any(|p| p.trim() == "labsentinel"))
        .unwrap_or(false);
    let upgrade = if wants_proto {
        ws.protocols(["labsentinel"])
    } else {
        ws
    };
    upgrade
        .max_message_size(64 * 1024)
        .on_upgrade(move |client_ws| async move {
            use futures_util::{SinkExt, StreamExt};
            use tokio_tungstenite::tungstenite::Message as TMsg;
            let Ok(Ok((up_ws, _))) = tokio::time::timeout(
                std::time::Duration::from_secs(4),
                tokio_tungstenite::connect_async(&upstream_url),
            )
            .await
            else {
                return;
            };
            let (mut c_tx, mut c_rx) = client_ws.split();
            let (mut u_tx, mut u_rx) = up_ws.split();
            let c2u = async {
                let mut started = false;
                while let Some(Ok(msg)) = c_rx.next().await {
                    let out = match msg {
                        axum::extract::ws::Message::Text(t) => {
                            let Ok(value) = serde_json::from_str::<Value>(&t) else {
                                continue;
                            };
                            let Some(kind) = value.get("type").and_then(Value::as_str) else {
                                continue;
                            };
                            let Some(payload) = value.get("value").and_then(Value::as_str) else {
                                continue;
                            };
                            match (mse, started, kind) {
                                (true, false, "mse") | (false, false, "webrtc/offer") => {
                                    started = true
                                }
                                (false, true, "webrtc/candidate") => {}
                                _ => continue,
                            }
                            TMsg::Text(json!({"type":kind, "value":payload}).to_string())
                        }
                        axum::extract::ws::Message::Close(_) => break,
                        _ => continue,
                    };
                    if u_tx.send(out).await.is_err() {
                        break;
                    }
                }
            };
            let u2c = async {
                while let Some(Ok(msg)) = u_rx.next().await {
                    let out = match msg {
                        TMsg::Text(t) => axum::extract::ws::Message::Text(t.to_string()),
                        TMsg::Binary(b) => axum::extract::ws::Message::Binary(b.to_vec()),
                        TMsg::Ping(p) => axum::extract::ws::Message::Ping(p.to_vec()),
                        TMsg::Pong(p) => axum::extract::ws::Message::Pong(p.to_vec()),
                        TMsg::Close(_) => break,
                        _ => continue,
                    };
                    if c_tx.send(out).await.is_err() {
                        break;
                    }
                }
            };
            tokio::select! {
                _ = c2u => {},
                _ = u2c => {},
                _ = super::events::socket_revoked(&st, &browser, token.as_deref()) => {},
            }
        })
}

pub(crate) async fn camera_groups(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let cams = s.cameras()?;
        let mut by_group: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
        for c in &cams {
            let e = by_group.entry(c.group_name.clone()).or_insert((0, 0));
            e.0 += 1;
            if c.status == "running" {
                e.1 += 1;
            }
        }
        let groups: Vec<Value> = by_group
            .into_iter()
            .map(|(name, (count, running))| {
                json!({"name": name, "count": count, "running": running})
            })
            .collect();
        Ok(Json(json!({"groups": groups})))
    })
}

pub(crate) async fn cameras_bulk(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let ids: Vec<i64> = body
        .get("ids")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
        .unwrap_or_default();
    let action = body
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if ids.is_empty()
        || ids.len() > 100
        || ids.iter().collect::<std::collections::HashSet<_>>().len() != ids.len()
        || (action != "start" && action != "stop")
    {
        return detail_err(
            StatusCode::BAD_REQUEST,
            "ids 必须为去重非空列表且 ≤100 个；action=start|stop",
        )
        .into_response();
    }
    with_store(&st, |s| {
        let mut missing = Vec::new();
        let mut failed = Vec::new();
        let mut ok: Vec<i64> = Vec::new();
        for cid in &ids {
            if s.camera(*cid)?.is_none() {
                missing.push(*cid);
                continue;
            }
            // RA1.2：start/stop 走进程内 worker（Python bulk 契约：ok/missing/failed）
            let res = if action == "start" {
                match crate::worker::WorkerConfig::from_db(s, *cid) {
                    Err(e) => Err(e),
                    Ok(None) => Err("camera disappeared".into()),
                    Ok(Some(cfg)) => crate::worker::start_worker(&st, cfg).map(|_| ()),
                }
            } else {
                crate::worker::stop_worker(&st, *cid);
                Ok(())
            };
            match res {
                Ok(()) => ok.push(*cid),
                Err(e) => failed.push(json!({"id": cid, "error": e})),
            }
        }
        Ok(Json(json!({"ok": ok, "missing": missing, "failed": failed})).into_response())
    })
}

pub(crate) async fn usb_devices() -> Response {
    // 2026-11-21（用户实测反馈）：ffmpeg dshow 枚举真实现（发行包自带 ffmpeg，
    // 零新依赖）——本机摄像头自动列出，不再要求手填索引。
    // source = `video=<设备名>`（ffmpeg dshow 原生语法），worker build_cmd 识别
    // 该前缀自动加 `-f dshow` 取流；枚举失败保留原错误文案（前端回退手填）。
    match ls_ingest::list_usb_devices() {
        Ok(devices) => Json(json!({
            "devices": devices.iter().map(|d| json!({
                "index": d.index, "name": d.name, "source": d.source,
            })).collect::<Vec<_>>(),
            "error": serde_json::Value::Null,
        }))
        .into_response(),
        Err(e) => Json(json!({
            "devices": [],
            "error": e,
        }))
        .into_response(),
    }
}

pub(crate) async fn copy_camera_config(
    State(st): State<Shared>,
    AxPath(cid): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    let targets: Vec<i64> = body
        .get("target_ids")
        .and_then(|v| v.as_array())
        .map(|a| {
            let mut ids: Vec<i64> = a.iter().filter_map(|x| x.as_i64()).collect();
            ids.sort();
            ids.dedup();
            ids
        })
        .unwrap_or_default();
    let copy_zones = body
        .get("copy_zones")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let copy_rules = body
        .get("copy_rules")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let suffix = body
        .get("name_suffix")
        .and_then(|v| v.as_str())
        .unwrap_or("-copy")
        .to_string();
    if targets.is_empty() {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "target_ids 必填").into_response();
    }
    if targets.contains(&cid) {
        return detail_err(StatusCode::BAD_REQUEST, "target_ids 不能包含源摄像头自身")
            .into_response();
    }
    with_store(&st, |s| {
        if s.camera(cid)?.is_none() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response());
        }
        for t in &targets {
            if s.camera(*t)?.is_none() {
                return Ok((
                    StatusCode::NOT_FOUND,
                    Json(json!({"detail": format!("目标摄像头不存在: #{t}")})),
                )
                    .into_response());
            }
        }
        // Python 语义：先全量校验再写库；规则 zone_id 按区域名映射到目标
        let src_zones = s.zones_all()?;
        let src_zones: Vec<_> = src_zones.into_iter().filter(|z| z.1 == cid).collect();
        let src_rules = s.rules_all()?;
        let src_rules: Vec<_> = src_rules.into_iter().filter(|r| r.1 == cid).collect();
        let mut zones_created = 0i64;
        let mut zones_skipped = 0i64;
        let mut rules_created = 0i64;
        let mut rules_skipped = 0i64;
        let mut warnings: Vec<String> = Vec::new();
        for tid in &targets {
            let tgt_zones = s.zones_all()?;
            let mut zone_map: std::collections::HashMap<String, i64> = tgt_zones
                .iter()
                .filter(|z| z.1 == *tid)
                .map(|z| (z.2.clone(), z.0))
                .collect();
            if copy_zones {
                for z in &src_zones {
                    let new_name = format!("{}{}", z.2, suffix);
                    if let Some(existing) = zone_map.get(&new_name) {
                        zone_map.insert(z.2.clone(), *existing);
                        zones_skipped += 1;
                        continue;
                    }
                    let zid = s.add_zone(*tid, &new_name, &z.3)?;
                    zone_map.insert(z.2.clone(), zid);
                    zone_map.insert(new_name, zid);
                    zones_created += 1;
                }
            } else {
                zones_skipped += src_zones.len() as i64;
            }
            if copy_rules {
                for r in &src_rules {
                    let zid = r.2.and_then(|zid| {
                        let src_name = src_zones.iter().find(|z| z.0 == zid).map(|z| z.2.clone());
                        src_name.and_then(|n| zone_map.get(&n).copied())
                    });
                    if r.2.is_some() && zid.is_none() {
                        rules_skipped += 1;
                        warnings.push(format!(
                            "目标 #{tid}：规则 {} 的源区域在目标上不存在（未复制区域或同名缺失）——已跳过",
                            r.3
                        ));
                        continue;
                    }
                    // 去重：同 behavior+zone+时段
                    let dup = s.rules_all()?.iter().any(|x| {
                        x.1 == *tid && x.2 == zid && x.3 == r.3 && x.6 == r.6 && x.7 == r.7
                    });
                    if dup {
                        rules_skipped += 1;
                        continue;
                    }
                    s.add_rule(
                        *tid,
                        zid,
                        &r.3,
                        r.4,
                        r.5,
                        r.6.as_deref(),
                        r.7.as_deref(),
                        r.8,
                        r.9 != 0,
                    )?;
                    rules_created += 1;
                }
            } else {
                rules_skipped += src_rules.len() as i64;
            }
        }
        Ok(Json(json!({"ok": true, "zones_created": zones_created,
                       "zones_skipped": zones_skipped, "rules_created": rules_created,
                       "rules_skipped": rules_skipped, "warnings": warnings}))
        .into_response())
    })
}

#[derive(serde::Deserialize)]
pub(crate) struct SnapshotQuery {
    preview: Option<u8>,
}

pub(crate) async fn camera_snapshot(
    State(st): State<Shared>,
    AxPath(cid): AxPath<i64>,
    Query(query): Query<SnapshotQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    let camera = ls_store::Store::open(&st.db_path)
        .ok()
        .and_then(|s| s.camera(cid).ok().flatten());
    let Some(camera) = camera else {
        return detail_err(StatusCode::NOT_FOUND, "camera not found").into_response();
    };
    if query.preview == Some(1) && camera.type_ == "file" {
        let source = std::path::PathBuf::from(camera.source);
        return match tokio::task::spawn_blocking(move || {
            if !source.is_file() {
                return Err((StatusCode::NOT_FOUND, "找不到视频文件，请检查接入地址"));
            }
            let jpeg = ls_ingest::decode_image(&source, 1280, 720)
                .and_then(|(bgr, w, h)| ls_analyzer::evidence::encode_jpeg_bgr(&bgr, w, h));
            jpeg.map_err(|_| {
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "无法读取视频首帧，请检查视频文件是否能正常播放",
                )
            })
        })
        .await
        {
            Ok(Ok(jpeg)) => (
                [
                    ("content-type", "image/jpeg"),
                    ("cache-control", "no-store"),
                ],
                jpeg,
            )
                .into_response(),
            Ok(Err((status, message))) => detail_err(status, message).into_response(),
            Err(_) => server_err(),
        };
    }
    // RA1.2：worker 运行中 → 最新抽帧 JPEG；未运行 → 503（与 start_camera 同口径）
    match st.workers.get(cid) {
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"detail": "摄像头未运行（无实时帧）；先启动摄像头",
                        "code": "no_worker"})),
        )
            .into_response(),
        Some(h) => match h.latest_jpeg.lock().ok().and_then(|s| s.clone()) {
            None => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"detail": "暂无可用帧（刚启动或抽帧未命中）", "code": "no_frame"})),
            )
                .into_response(),
            Some(frame) => {
                let tag = format!("\"{}-{}\"", frame.captured_at_ms, frame.sequence);
                let unchanged = headers
                    .get("if-none-match")
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| {
                        value
                            .split(',')
                            .map(str::trim)
                            .any(|entry| entry == "*" || entry.trim_start_matches("W/") == tag)
                    });
                let common = [
                    ("content-type", "image/jpeg".to_string()),
                    ("cache-control", "no-store".to_string()),
                    ("x-ls-frame-time-ms", frame.captured_at_ms.to_string()),
                    ("etag", tag),
                ];
                if unchanged {
                    (StatusCode::NOT_MODIFIED, common).into_response()
                } else {
                    (common, frame.jpeg).into_response()
                }
            }
        },
    }
}

/// DELETE /cameras/{cid}（RA1.2 补路由：清单里一直有、router 从未注册——请求恒 404）。
/// Python 契约：manager.stop_camera → 级联清理（区域/规则）→ `{"ok": true}`；
/// 文件夹派生图/缩略图清理属 Python 文件系统语义，Rust 侧仅 DB 级联（ls-store 同口径）。
pub(crate) async fn delete_camera(State(st): State<Shared>, AxPath(cid): AxPath<i64>) -> Response {
    crate::worker::stop_worker(&st, cid);
    with_store(&st, |s| {
        s.delete_camera(cid)?;
        st.go2rtc.refresh();
        Ok(Json(json!({"ok": true})).into_response())
    })
}

// ==== R2-309 批次 3：配置导入/存证打包/插件占位/face 分支/告警自检/PPE 下载 ====
