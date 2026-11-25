use crate::access::{set_cookie, unix_now, BrowserAccess};
use crate::state::Shared;
use crate::util::*;
use axum::body::Bytes;
use axum::extract::{Path as AxPath, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::Extension;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(crate) const DEVICE_COOKIE: &str = "ls_device";
const CODE_TTL: u64 = 300;
const COOKIE_TTL: u64 = 365 * 86_400;

#[derive(Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Role {
    #[default]
    ReadOnly,
    Triage,
}

#[derive(Clone, Deserialize)]
struct Grant {
    #[serde(default)]
    role: Role,
    all_cameras: bool,
    #[serde(default)]
    camera_ids: Vec<i64>,
    #[serde(default)]
    groups: Vec<String>,
    expires_days: u64,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
struct DevicePolicy {
    role: Role,
    all_cameras: bool,
    camera_ids: Vec<i64>,
    groups: Vec<String>,
    expires_at: u64,
}

impl Grant {
    fn policy(self) -> Result<DevicePolicy, &'static str> {
        if !(1..=365).contains(&self.expires_days) {
            return Err("设备有效期须为 1 至 365 天");
        }
        if self.camera_ids.len() > 1024
            || self.groups.len() > 256
            || self.camera_ids.iter().any(|id| *id <= 0)
        {
            return Err("相机范围无效");
        }
        let mut camera_ids = if self.all_cameras {
            Vec::new()
        } else {
            self.camera_ids
        };
        let mut groups = if self.all_cameras {
            Vec::new()
        } else {
            self.groups
        };
        groups.retain(|group| !group.trim().is_empty());
        if !self.all_cameras && camera_ids.is_empty() && groups.is_empty() {
            return Err("请选择至少一台摄像头或一个分组");
        }
        camera_ids.sort_unstable();
        camera_ids.dedup();
        groups.sort_unstable();
        groups.dedup();
        Ok(DevicePolicy {
            role: self.role,
            all_cameras: self.all_cameras,
            camera_ids,
            groups,
            expires_at: unix_now() + self.expires_days * 86_400,
        })
    }
}

#[derive(Clone)]
pub(crate) struct PendingPairing {
    expires: u64,
    grant: Option<Grant>,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct PairedDevice {
    pub id: i64,
    pub name: String,
    pub digest: String,
    pub cameras: Option<Vec<i64>>,
    policy: DevicePolicy,
}

impl PairedDevice {
    pub fn allows_camera(&self, id: i64) -> bool {
        self.cameras
            .as_ref()
            .is_none_or(|ids| ids.binary_search(&id).is_ok())
    }

    pub fn can_triage(&self) -> bool {
        self.policy.role == Role::Triage
    }

    pub fn summary(&self) -> Value {
        let scope = json!([self.policy, self.cameras]);
        json!({"id": self.id, "name": self.name, "can_triage": self.can_triage(), "expires_at": self.policy.expires_at,
            "scope_version": sha256_hex(scope.to_string().as_bytes())})
    }
}

pub(crate) fn paired_device(st: &Shared, digest: &str) -> Option<PairedDevice> {
    let store = ls_store::Store::open(&st.db_path).ok()?;
    let device = store.device_by_hash(digest).ok()??;
    if device.revoked || device.platform != "web" {
        return None;
    }
    let policy: DevicePolicy = serde_json::from_str(device.policy_json.as_deref()?).ok()?;
    if policy.expires_at <= unix_now() {
        return None;
    }
    let cameras = if policy.all_cameras {
        None
    } else {
        let mut ids = policy.camera_ids.clone();
        if !policy.groups.is_empty() {
            ids.extend(
                store
                    .cameras()
                    .ok()?
                    .into_iter()
                    .filter(|camera| policy.groups.binary_search(&camera.group_name).is_ok())
                    .map(|camera| camera.id),
            );
        }
        ids.sort_unstable();
        ids.dedup();
        Some(ids)
    };
    crate::middleware::touch_device(st, &store, device.id);
    Some(PairedDevice {
        id: device.id,
        name: device.name,
        digest: digest.to_owned(),
        cameras,
        policy,
    })
}

pub(crate) fn permit_device(
    st: &Shared,
    device: &PairedDevice,
    method: &Method,
    path: &str,
) -> bool {
    if method == Method::GET || method == Method::HEAD {
        if matches!(
            path,
            "/api/cameras"
                | "/api/events"
                | "/api/events/count"
                | "/api/events/explore"
                | "/api/events/explore/settings"
                | "/api/stats/daily"
                | "/api/stats/compliance"
                | "/ws/events"
                | "/api/access/status"
                | "/api/share-links/verify"
                | "/api/v1/meta"
                | "/api/health/live"
                | "/api/health/ready"
        ) {
            return true;
        }
        for prefix in ["/ws/webrtc/", "/ws/detections/"] {
            if let Some(id) = path
                .strip_prefix(prefix)
                .and_then(|value| value.parse().ok())
            {
                return device.allows_camera(id);
            }
        }
        if let Some((id, field)) = split_path_id(path, "/api/cameras/") {
            return matches!(field, "snapshot" | "webrtc" | "status") && device.allows_camera(id);
        }
    }
    if method == Method::POST && matches!(path, "/api/pairing/claim" | "/api/pairing/logout") {
        return true;
    }
    let Some((id, field)) = split_path_id(path, "/api/events/") else {
        return false;
    };
    let permitted_operation =
        (method == Method::POST && matches!(field, "status" | "notes") && device.can_triage())
            || ((method == Method::GET || method == Method::HEAD)
                && (matches!(
                    field,
                    "thumb" | "snapshot" | "video" | "overlay" | "explain"
                ) || (field == "notes" && device.can_triage())));
    permitted_operation
        && ls_store::Store::open(&st.db_path)
            .ok()
            .and_then(|store| store.event_by_id(id).ok().flatten())
            .is_some_and(|event| device.allows_camera(event.camera_id))
}

fn split_path_id<'a>(path: &'a str, prefix: &str) -> Option<(i64, &'a str)> {
    let (id, item) = path.strip_prefix(prefix)?.split_once('/')?;
    Some((id.parse().ok()?, item))
}

pub(crate) async fn create_pairing(State(st): State<Shared>, body: Bytes) -> Response {
    let grant = if body.is_empty() {
        None
    } else {
        let Ok(grant) = serde_json::from_slice::<Grant>(&body) else {
            return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "配对权限格式无效")
                .into_response();
        };
        if let Err(reason) = grant.clone().policy() {
            return detail_err(StatusCode::UNPROCESSABLE_ENTITY, reason).into_response();
        }
        Some(grant)
    };
    let Ok(mut state) = st.pair_state.lock() else {
        return server_err();
    };
    let now = unix_now();
    state.pending.retain(|_, pending| pending.expires > now);
    if state.pending.len() >= 32 {
        return detail_err(
            StatusCode::TOO_MANY_REQUESTS,
            "待认领的配对码已满，请撤销旧码或等其到期",
        )
        .into_response();
    }
    let mut code;
    loop {
        let mut bytes = [0u8; 4];
        if getrandom::getrandom(&mut bytes).is_err() {
            return server_err();
        }
        code = format!("{:06}", u32::from_le_bytes(bytes) % 1_000_000);
        if !state.pending.contains_key(&code) {
            break;
        }
    }
    state.pending.insert(
        code.clone(),
        PendingPairing {
            expires: now + CODE_TTL,
            grant,
        },
    );
    Json(json!({"code": code, "expires_in_s": CODE_TTL,
        "claim": "POST /api/pairing/claim {code, device_name, platform}"}))
    .into_response()
}

#[derive(Deserialize)]
pub(crate) struct Claim {
    code: String,
    device_name: Option<String>,
    platform: Option<String>,
    #[serde(default)]
    browser: bool,
}

pub(crate) async fn claim_pairing(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
    Json(body): Json<Claim>,
) -> Response {
    tokio::task::spawn_blocking(move || claim(st, browser, body))
        .await
        .unwrap_or_else(|_| server_err())
}

fn claim(st: Shared, browser: BrowserAccess, body: Claim) -> Response {
    let code = body.code.trim();
    if code.len() != 6 || !code.bytes().all(|ch| ch.is_ascii_digit()) {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "请输入 6 位配对码").into_response();
    }
    let Ok(mut state) = st.pair_state.lock() else {
        return server_err();
    };
    state
        .pending
        .retain(|_, pending| pending.expires > unix_now());
    let Some(pending) = state.pending.get(code) else {
        return detail_err(StatusCode::BAD_REQUEST, "配对码无效或已过期").into_response();
    };
    if body.browser && pending.grant.is_none() {
        return detail_err(
            StatusCode::CONFLICT,
            "请使用「设备与配对」为浏览器重新生成配对码",
        )
        .into_response();
    }
    let name = device_name(body.device_name.as_deref().unwrap_or(""));
    let store = match ls_store::Store::open(&st.db_path) {
        Ok(store) => store,
        Err(_) => return server_err(),
    };
    let response = if let Some(grant) = pending.grant.clone() {
        if !browser.client.can_use_password() {
            return detail_err(StatusCode::FORBIDDEN, "请使用 HTTPS 访问地址配对局域网设备")
                .into_response();
        }
        let Ok(policy) = grant.policy() else {
            return server_err();
        };
        let Ok(encoded) = serde_json::to_string(&policy) else {
            return server_err();
        };
        let token = crate::envfile::random_token();
        let digest = sha256_hex(token.as_bytes());
        let previous = browser.device.as_ref().map(|device| device.id);
        if store
            .add_browser_device(&name, &digest, &encoded, previous)
            .is_err()
        {
            return server_err();
        }
        set_cookie(
            Json(json!({"ok": true, "expires_at": policy.expires_at})).into_response(),
            DEVICE_COOKIE,
            &token,
            COOKIE_TTL,
            browser.client.secure,
        )
    } else {
        let platform = body
            .platform
            .as_deref()
            .unwrap_or("")
            .chars()
            .take(32)
            .collect::<String>();
        let platform = if platform == "share" {
            "legacy"
        } else {
            &platform
        };
        let token = random_token();
        let digest = sha256_hex(token.as_bytes());
        if store.add_device(&name, platform, &digest).is_err() {
            return server_err();
        }
        Json(json!({"token": token, "note": "令牌仅显示一次，请保存到安全存储"})).into_response()
    };
    state.pending.remove(code);
    response
}

fn device_name(name: &str) -> String {
    let clean: String = name
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(64)
        .collect();
    if clean.is_empty() {
        "device".into()
    } else {
        clean
    }
}

#[derive(Deserialize)]
pub(crate) struct Code {
    code: String,
}

pub(crate) async fn revoke_pairing(State(st): State<Shared>, Json(body): Json<Code>) -> Response {
    let Ok(mut state) = st.pair_state.lock() else {
        return server_err();
    };
    state.pending.remove(body.code.trim());
    Json(json!({"ok": true})).into_response()
}

pub(crate) async fn list_devices(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let rows = s.devices()?;
        Ok(Json(
            rows.iter()
                .map(|device| {
                    let policy = device
                        .policy_json
                        .as_deref()
                        .and_then(|raw| serde_json::from_str::<DevicePolicy>(raw).ok());
                    let expired = policy.as_ref().is_some_and(|p| p.expires_at <= unix_now());
                    json!({"id": device.id, "name": device.name, "platform": device.platform,
                "created_ts": device.created_ts, "last_seen_ts": device.last_seen_ts,
                "revoked": device.revoked, "expired": expired, "policy": policy})
                })
                .collect::<Vec<_>>(),
        ))
    })
}

#[derive(Deserialize)]
pub(crate) struct DeviceEdit {
    name: String,
    #[serde(flatten)]
    grant: Grant,
}

pub(crate) async fn edit_device(
    State(st): State<Shared>,
    AxPath(id): AxPath<i64>,
    Json(body): Json<DeviceEdit>,
) -> Response {
    let policy = match body.grant.policy() {
        Ok(policy) => policy,
        Err(reason) => return detail_err(StatusCode::UNPROCESSABLE_ENTITY, reason).into_response(),
    };
    let Ok(encoded) = serde_json::to_string(&policy) else {
        return server_err();
    };
    with_store(&st, |store| {
        if store.update_browser_device(id, &device_name(&body.name), &encoded)? {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok(detail_err(StatusCode::NOT_FOUND, "设备已撤销或需要重新配对").into_response())
        }
    })
}

pub(crate) async fn revoke_device(State(st): State<Shared>, AxPath(id): AxPath<i64>) -> Response {
    with_store(&st, |store| {
        if store.revoke_device(id)? {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok(detail_err(StatusCode::NOT_FOUND, "设备不存在").into_response())
        }
    })
}

pub(crate) async fn device_logout(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
) -> Response {
    with_store(&st, |store| {
        if let Some(device) = browser.device {
            store.revoke_device(device.id)?;
        }
        Ok(set_cookie(
            Json(json!({"ok": true})).into_response(),
            DEVICE_COOKIE,
            "",
            0,
            browser.client.secure,
        ))
    })
}
