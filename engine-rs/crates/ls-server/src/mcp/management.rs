use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use ls_store::{NewMcpToken, Store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::access::BrowserAccess;
use crate::middleware::{is_api_admin, provided_token};
use crate::state::Shared;
use crate::util::{detail_err, server_err, sha256_hex, with_store};

use super::{now, tools, ENDPOINT};

const POLICY_KEY: &str = "mcp.policy";

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct Policy {
    pub enabled: bool,
    pub allow_lan: bool,
    pub allow_image_content: bool,
    pub allow_global_silence: bool,
    pub allowed_write_tools: Vec<String>,
}

impl Policy {
    pub(super) fn load(store: &Store) -> rusqlite::Result<Self> {
        Ok(store
            .setting_get(POLICY_KEY)?
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default())
    }

    fn public(&self) -> Value {
        json!({
            "enabled": self.enabled,
            "allow_lan": self.allow_lan,
            "allow_image_content": self.allow_image_content,
            "allow_global_silence": self.allow_global_silence,
            "allowed_write_tools": self.allowed_write_tools,
            "endpoint": ENDPOINT,
            "tools": tools::definitions().into_iter().map(|tool| json!({
                "name":tool.name,"label":tool.label,"description":tool.description,"write":tool.write,"high":tool.high,
            })).collect::<Vec<_>>(),
        })
    }
}

fn admin_allowed(st: &Shared, browser: &BrowserAccess, headers: &HeaderMap) -> bool {
    !browser.share
        && browser.device.is_none()
        && (browser.client.local
            || (browser.client.secure
                && (browser.session.is_some()
                    || is_api_admin(st, provided_token(headers).as_deref())))
            || (st.testing && browser.client.ip.is_none()))
}

fn denied() -> Response {
    detail_err(
        StatusCode::FORBIDDEN,
        "MCP 设置需要本机管理权限或受信 HTTPS 管理会话",
    )
    .into_response()
}

fn private(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(crate) async fn config(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
    headers: HeaderMap,
) -> Response {
    if !admin_allowed(&st, &browser, &headers) {
        return denied();
    }
    private(with_store(&st, |store| {
        Ok(Json(Policy::load(store)?.public()))
    }))
}

#[derive(Deserialize)]
pub(crate) struct UpdatePolicy {
    enabled: Option<bool>,
    allow_lan: Option<bool>,
    allow_image_content: Option<bool>,
    allow_global_silence: Option<bool>,
    allowed_write_tools: Option<Vec<String>>,
    #[serde(default)]
    confirm_high_risk: bool,
}

pub(crate) async fn update_config(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
    headers: HeaderMap,
    Json(update): Json<UpdatePolicy>,
) -> Response {
    if !admin_allowed(&st, &browser, &headers) {
        return denied();
    }
    static POLICY_WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = POLICY_WRITE.lock().unwrap_or_else(|e| e.into_inner());
    private(with_store(&st, |store| {
        let mut policy = Policy::load(store)?;
        let definitions = tools::definitions();
        let mut changed_high_risk = update.allow_lan == Some(true) && !policy.allow_lan
            || update.allow_global_silence == Some(true) && !policy.allow_global_silence;
        if let Some(mut write_tools) = update.allowed_write_tools {
            for name in &write_tools {
                let Some(tool) = definitions
                    .iter()
                    .find(|tool| tool.write && tool.name == name)
                else {
                    return Ok(detail_err(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "请求包含无法授权的工具",
                    )
                    .into_response());
                };
                changed_high_risk |= tool.high && !policy.allowed_write_tools.contains(name);
            }
            write_tools.sort_unstable();
            write_tools.dedup();
            policy.allowed_write_tools = write_tools;
        }
        if changed_high_risk && !update.confirm_high_risk {
            return Ok(detail_err(
                StatusCode::CONFLICT,
                "启用高权限工具、全局静默或局域网访问需要再次确认",
            )
            .into_response());
        }
        policy.enabled = update.enabled.unwrap_or(policy.enabled);
        policy.allow_lan = update.allow_lan.unwrap_or(policy.allow_lan);
        policy.allow_image_content = update
            .allow_image_content
            .unwrap_or(policy.allow_image_content);
        policy.allow_global_silence = update
            .allow_global_silence
            .unwrap_or(policy.allow_global_silence);
        store.setting_set(POLICY_KEY, &json!(policy).to_string())?;
        Ok(Json(policy.public()).into_response())
    }))
}

pub(crate) async fn tokens(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
    headers: HeaderMap,
) -> Response {
    if !admin_allowed(&st, &browser, &headers) {
        return denied();
    }
    private(with_store(&st, |store| Ok(Json(store.mcp_tokens()?))))
}

#[derive(Deserialize)]
pub(crate) struct CreateToken {
    name: String,
    scope: String,
    camera_ids: Option<Vec<i64>>,
    ttl_days: Option<i64>,
}

pub(crate) async fn issue_token(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
    headers: HeaderMap,
    Json(mut input): Json<CreateToken>,
) -> Response {
    if !admin_allowed(&st, &browser, &headers) {
        return denied();
    }
    let name = input.name.trim();
    if name.is_empty()
        || name.chars().count() > 80
        || name.chars().any(char::is_control)
        || !matches!(input.scope.as_str(), "read_only" | "policy")
        || input
            .ttl_days
            .is_some_and(|days| !(1..=365).contains(&days))
        || input
            .camera_ids
            .as_ref()
            .is_some_and(|ids| ids.len() > 256 || ids.is_empty())
    {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "请核对令牌名称、作用域、机位及有效期",
        )
        .into_response();
    }
    private(with_store(&st, |store| {
        if let Some(ids) = input.camera_ids.as_mut() {
            ids.sort_unstable();
            ids.dedup();
            for &id in ids.iter() {
                if id <= 0 || store.camera(id)?.is_none() {
                    return Ok(detail_err(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "令牌包含不存在的机位",
                    )
                    .into_response());
                }
            }
        }
        let mut secret = [0u8; 32];
        if getrandom::getrandom(&mut secret).is_err() {
            return Ok(server_err());
        }
        use base64::Engine as _;
        let token = format!(
            "lsmcp_{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret)
        );
        let issued = now();
        let id = store.create_mcp_token(&NewMcpToken {
            name,
            token_prefix: &token[..14],
            token_hash: &sha256_hex(token.as_bytes()),
            scope: &input.scope,
            camera_ids: input.camera_ids.as_deref(),
            created_at: issued,
            expires_at: input.ttl_days.map(|ttl| issued + ttl * 86400),
        })?;
        Ok((StatusCode::CREATED, Json(json!({"id":id,"token":token}))).into_response())
    }))
}

pub(crate) async fn revoke_token(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Response {
    if !admin_allowed(&st, &browser, &headers) {
        return denied();
    }
    private(with_store(&st, |store| {
        if store.revoke_mcp_token(id)? {
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
        Ok(detail_err(StatusCode::NOT_FOUND, "令牌不存在或已撤销").into_response())
    }))
}

pub(crate) async fn verify_receipt(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
    headers: HeaderMap,
    Json(receipt): Json<Value>,
) -> Response {
    if !admin_allowed(&st, &browser, &headers) {
        return denied();
    }
    private(with_store(&st, |store| {
        Ok(Json(
            json!({"valid":super::receipts::verify(store, &receipt)?}),
        ))
    }))
}
