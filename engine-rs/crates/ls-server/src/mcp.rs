use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::to_bytes;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use ls_store::{McpToken, Store};
use serde_json::{json, Value};

use crate::state::Shared;
use crate::util::{detail_err, server_err, sha256_hex, uuid_like};

mod management;
mod receipts;
mod tools;

use management::Policy;
pub(crate) use management::{
    config, issue_token, revoke_token, tokens, update_config, verify_receipt,
};

pub(crate) const ENDPOINT: &str = "/mcp";
const PROTOCOL: &str = "2025-06-18";
const COMPAT_PROTOCOL: &str = "2025-03-26";
const BODY_LIMIT: usize = 64 * 1024;
const NOT_ALLOWED: i64 = -32003;

#[derive(Clone, Copy)]
struct ToolScope<'a> {
    pub cameras: Option<&'a [i64]>,
    pub allow_image_content: bool,
    pub allow_global_silence: bool,
    pub actor: &'a str,
}

struct Principal {
    token: McpToken,
    token_hash: String,
    policy: Policy,
}

struct RateWindow {
    start: Instant,
    total: u16,
    writes: u16,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn one_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let values = headers.get_all(name);
    let mut values = values.iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}

fn origin_allowed(request: &Request, authority: &axum::http::uri::Authority, secure: bool) -> bool {
    if !request.headers().contains_key(header::ORIGIN) {
        return true;
    }
    let scheme = if secure { "https" } else { "http" };
    let port = authority
        .port_u16()
        .unwrap_or(if secure { 443 } else { 80 });
    one_header(request.headers(), "origin")
        .and_then(|text| url::Url::parse(text).ok())
        .is_some_and(|origin| {
            origin.scheme() == scheme
                && origin.username().is_empty()
                && origin.password().is_none()
                && origin
                    .host_str()
                    .is_some_and(|host| host.eq_ignore_ascii_case(authority.host()))
                && origin.port_or_known_default() == Some(port)
                && origin.path() == "/"
                && origin.query().is_none()
                && origin.fragment().is_none()
        })
}

fn private(mut response: Response) -> Response {
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}

fn authorize(st: &Shared, request: &Request) -> Result<Principal, StatusCode> {
    let store = Store::open(&st.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let policy = Policy::load(&store).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !policy.enabled {
        return Err(StatusCode::NOT_FOUND);
    }
    let authority = one_header(request.headers(), "host")
        .and_then(|text| text.parse::<axum::http::uri::Authority>().ok())
        .filter(|authority| !authority.as_str().contains('@'))
        .ok_or(StatusCode::FORBIDDEN)?;
    let host = authority.host().trim_matches(['[', ']']);
    let client = st.access.request(request, host).client;
    let loopback = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|peer| peer.0.ip().to_canonical().is_loopback());
    let local = loopback
        && crate::is_loopback_host(host)
        && !request.headers().contains_key("x-forwarded-for")
        && !request.headers().contains_key("forwarded");
    if !(local
        || policy.allow_lan
            && client.secure
            && st
                .trusted_hosts
                .iter()
                .any(|trusted| trusted.eq_ignore_ascii_case(host)))
        || !origin_allowed(request, &authority, client.secure)
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let token = one_header(request.headers(), "authorization")
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("Bearer"))
        .map(|(_, token)| token.trim())
        .filter(|token| {
            token.len() == 49
                && token.starts_with("lsmcp_")
                && token[6..]
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_')
        })
        .filter(|_| !request.headers().contains_key(header::COOKIE))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let token_hash = sha256_hex(token.as_bytes());
    let token = store
        .valid_mcp_token(&token_hash, now())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    Ok(Principal {
        token,
        token_hash,
        policy,
    })
}

fn auth_error(code: StatusCode) -> Response {
    let message = match code {
        StatusCode::NOT_FOUND => "MCP 未启用",
        StatusCode::FORBIDDEN => "当前地址或来源未获授权",
        StatusCode::UNAUTHORIZED => "需要有效的独立 MCP Bearer 令牌",
        _ => "MCP 暂时不可用",
    };
    let mut response = detail_err(code, message);
    if code == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"LabSentinel MCP\""),
        );
    }
    private(response)
}

fn admit(hash: &str, write: bool) -> bool {
    static WINDOWS: OnceLock<Mutex<HashMap<String, RateWindow>>> = OnceLock::new();
    let mut entries = WINDOWS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let now = Instant::now();
    let interval = Duration::from_secs(60);
    if entries.len() >= 1024 {
        entries.retain(|_, count| now.saturating_duration_since(count.start) < interval);
    }
    if entries.len() >= 2048 && !entries.contains_key(hash) {
        return false;
    }
    let entry = entries.entry(hash.to_string()).or_insert(RateWindow {
        start: now,
        total: 0,
        writes: 0,
    });
    if now.saturating_duration_since(entry.start) >= interval {
        *entry = RateWindow {
            start: now,
            total: 0,
            writes: 0,
        };
    }
    if entry.total >= 120 || write && entry.writes >= 20 {
        return false;
    }
    entry.total += 1;
    entry.writes += u16::from(write);
    true
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Response {
    Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})).into_response()
}

fn rpc_result(id: &Value, result: Value) -> Response {
    Json(json!({"jsonrpc":"2.0","id":id,"result":result})).into_response()
}

fn permitted(principal: &Principal, tool: &tools::Definition) -> bool {
    !tool.write
        || principal.token.scope == "policy"
            && principal
                .policy
                .allowed_write_tools
                .iter()
                .any(|name| name == tool.name)
}

pub(crate) async fn no_stream(State(st): State<Shared>, request: Request) -> Response {
    match authorize(&st, &request) {
        Ok(_) => {
            private((StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")]).into_response())
        }
        Err(error) => auth_error(error),
    }
}

pub(crate) async fn handle(State(st): State<Shared>, request: Request) -> Response {
    if let Err(error) = authorize(&st, &request) {
        return auth_error(error);
    }
    if request.headers().contains_key("mcp-protocol-version")
        && !one_header(request.headers(), "mcp-protocol-version")
            .is_some_and(|version| matches!(version, PROTOCOL | COMPAT_PROTOCOL))
    {
        return private(detail_err(StatusCode::BAD_REQUEST, "不支持的 MCP 协议版本"));
    }
    if let Some(accept) = one_header(request.headers(), "accept") {
        let supported = accept.split(',').any(|entry| {
            let mut parts = entry.split(';').map(str::trim);
            let mime = parts.next().unwrap_or_default();
            matches!(mime, "application/json" | "application/*" | "*/*")
                && !parts.any(|param| {
                    param.split_once('=').is_some_and(|(key, value)| {
                        key.eq_ignore_ascii_case("q") && value.trim().parse::<f32>() == Ok(0.0)
                    })
                })
        });
        if !supported {
            return private(detail_err(
                StatusCode::NOT_ACCEPTABLE,
                "MCP 客户端须支持 application/json",
            ));
        }
    }
    if !one_header(request.headers(), "content-type").is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
    }) {
        return private(detail_err(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "MCP 请求需要 application/json",
        ));
    }
    let (parts, body) = request.into_parts();
    let bytes =
        match tokio::time::timeout(Duration::from_secs(10), to_bytes(body, BODY_LIMIT)).await {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(_)) => {
                return private(detail_err(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "MCP 请求体过长或读取失败",
                ))
            }
            Err(_) => {
                return private(detail_err(
                    StatusCode::REQUEST_TIMEOUT,
                    "MCP 请求体读取超时",
                ))
            }
        };
    let Ok(message) = serde_json::from_slice::<Value>(&bytes) else {
        return private(rpc_error(&Value::Null, -32700, "JSON 解析失败"));
    };
    let request = Request::from_parts(parts, axum::body::Body::empty());
    let principal = match authorize(&st, &request) {
        Ok(principal) => principal,
        Err(error) => return auth_error(error),
    };
    private(rpc(&st, &principal, &message).await)
}

async fn rpc(st: &Shared, principal: &Principal, message: &Value) -> Response {
    let id = &message["id"];
    let method = message["method"].as_str().unwrap_or("");
    if message["jsonrpc"] != "2.0"
        || method.is_empty()
        || !(id.is_string() || id.is_i64() || id.is_u64() || message.get("id").is_none())
    {
        return rpc_error(&Value::Null, -32600, "无效的 JSON-RPC 请求");
    }
    let definitions = tools::definitions();
    let tool_name = message["params"]["name"].as_str().unwrap_or("");
    let tool = (method == "tools/call")
        .then(|| definitions.iter().find(|tool| tool.name == tool_name))
        .flatten();
    if !admit(&principal.token_hash, tool.is_some_and(|tool| tool.write)) {
        return (StatusCode::TOO_MANY_REQUESTS, Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32029,"message":"MCP 请求过于频繁"}}))).into_response();
    }
    if message.get("id").is_none() {
        return if matches!(
            method,
            "notifications/initialized" | "notifications/cancelled"
        ) {
            StatusCode::ACCEPTED.into_response()
        } else {
            detail_err(StatusCode::BAD_REQUEST, "该 MCP 操作需要请求编号")
        };
    }
    match method {
        "initialize" => rpc_result(
            id,
            json!({
                "protocolVersion":if message["params"]["protocolVersion"]==COMPAT_PROTOCOL {COMPAT_PROTOCOL} else {PROTOCOL},
                "capabilities":{"tools":{"listChanged":false}},
                "serverInfo":{"name":"LabSentinel","version":st.version},
            }),
        ),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(
            id,
            json!({"tools":definitions.into_iter().filter(|tool| permitted(principal,tool)).map(|tool|json!({
            "name":tool.name,"description":tool.description,"inputSchema":tool.input_schema,
            "annotations":{"title":tool.label,"readOnlyHint":!tool.write,"destructiveHint":tool.high,"openWorldHint":false},
        })).collect::<Vec<_>>() }),
        ),
        "tools/call" => invoke(st, principal, id, tool, message).await,
        _ => rpc_error(id, -32601, "未知的 JSON-RPC 方法"),
    }
}

async fn invoke(
    st: &Shared,
    principal: &Principal,
    id: &Value,
    definition: Option<&tools::Definition>,
    message: &Value,
) -> Response {
    let allowed = definition.is_some_and(|tool| permitted(principal, tool));
    let name = definition.map_or("unknown_tool", |tool| tool.name);
    let actor = format!("mcp:{}", principal.token.id);
    let request_id = uuid_like();
    let args = message["params"]
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let Ok(audit_id) = Store::open(&st.db_path).and_then(|store| {
        let decision = if allowed { "admitted" } else { "denied" };
        store.mcp_audit_start(&request_id, &actor, name, &json!({"decision":decision}))
    }) else {
        return server_err();
    };
    let scope = ToolScope {
        cameras: principal.token.camera_ids.as_deref(),
        allow_image_content: principal.policy.allow_image_content,
        allow_global_silence: principal.policy.allow_global_silence,
        actor: &actor,
    };
    let mut result = if !allowed {
        Err(tools::ToolError {
            code: NOT_ALLOWED,
            message: "该工具未获授权".into(),
        })
    } else if !args.is_object() {
        Err(tools::ToolError {
            code: -32602,
            message: "工具参数必须是 JSON 对象".into(),
        })
    } else {
        tools::call(st, scope, name, &args).await
    };
    let Ok(store) = Store::open(&st.db_path) else {
        return server_err();
    };
    if let Ok(outcome) = &mut result {
        if !(scope.allow_image_content
            && name == "get_event_evidence"
            && args["include_thumbnail"] == true)
        {
            if let Some(value) = outcome.value.as_object_mut() {
                value.remove("thumbnail");
            }
        }
        if receipts::attach(&store, audit_id, name, scope.cameras, outcome).is_err() {
            result = Err(tools::ToolError {
                code: -32603,
                message: "操作已返回结果，但凭证暂不可用".into(),
            });
        }
    }
    match result {
        Ok(outcome) => {
            if store
                .mcp_audit_result(
                    audit_id,
                    200,
                    outcome.event_id,
                    &json!({"decision":"allowed"}),
                )
                .is_err()
            {
                return rpc_error(id, -32603, "无法写入审计结果");
            }
            let mut visible = outcome.value.clone();
            let thumbnail = visible
                .as_object_mut()
                .and_then(|object| object.remove("thumbnail"));
            let mut content = vec![json!({"type":"text","text":visible.to_string()})];
            if scope.allow_image_content {
                if let Some(image) = thumbnail {
                    if let (Some(data), Some(mime)) =
                        (image["data"].as_str(), image["mimeType"].as_str())
                    {
                        content.push(json!({"type":"image","data":data,"mimeType":mime}));
                    }
                }
            }
            rpc_result(
                id,
                json!({"content":content,"structuredContent":outcome.value,"isError":false}),
            )
        }
        Err(error) => {
            let denied = error.code == NOT_ALLOWED;
            let decision = if denied { "denied" } else { "failed" };
            if store
                .mcp_audit_result(
                    audit_id,
                    if denied { 403 } else { 400 },
                    None,
                    &json!({"decision":decision,"error_code":error.code}),
                )
                .is_err()
            {
                return rpc_error(id, -32603, "无法写入审计结果");
            }
            rpc_error(id, error.code, &error.message)
        }
    }
}
