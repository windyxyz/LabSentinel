use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use argon2::password_hash::SaltString;
use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Semaphore;

use crate::envfile::random_token;
use crate::state::Shared;
use crate::util::sha256_hex;

const COOKIE: &str = "ls_session";
const POLICY_KEY: &str = "access.password";
const RECOVERY_KEY: &str = "access.recovery";
const SESSION_SECONDS: u64 = 12 * 60 * 60;
const RECOVERY_SECONDS: u64 = 10 * 60;
const RATE_WINDOW: Duration = Duration::from_secs(60);
const MIN_PASSWORD_CHARS: usize = 12;
const MAX_PASSWORD_BYTES: usize = 512;

type AccessError = (StatusCode, &'static str);
const STORAGE_ERROR: AccessError = (StatusCode::INTERNAL_SERVER_ERROR, "无法读取或保存访问设置");
const INVALID_PASSWORD: AccessError = (StatusCode::UNAUTHORIZED, "密码不正确");
const INVALID_RECOVERY: AccessError = (StatusCode::UNAUTHORIZED, "恢复码无效或已过期");
const TLS_REQUIRED: AccessError = (StatusCode::FORBIDDEN, "请通过 HTTPS 访问局域网密码入口");
const RATE_LIMITED: AccessError = (StatusCode::TOO_MANY_REQUESTS, "尝试过多，请稍后再试");

#[derive(Clone, Copy)]
pub(crate) struct Client {
    pub local: bool,
    pub secure: bool,
    pub ip: Option<IpAddr>,
}

impl Client {
    pub(crate) fn can_use_password(self) -> bool {
        self.local || self.secure
    }
}

#[derive(Clone)]
pub(crate) struct BrowserAccess {
    pub client: Client,
    /// Digest of the browser credential, after a successful server-side lookup.
    pub session: Option<String>,
    pub device: Option<crate::handlers::pairing::PairedDevice>,
    pub device_expired: bool,
    pub share: bool,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct Policy {
    local: bool,
    lan: bool,
    password_hash: Option<String>,
    #[serde(default)]
    trust_proxy: Option<bool>,
}

struct Session {
    local: bool,
    started: u64,
    expires: u64,
}

#[derive(Default)]
struct AccessState {
    policy: Policy,
    revision: u64,
    sessions: HashMap<String, Session>,
    attempts: HashMap<Option<IpAddr>, (Instant, u8)>,
}

pub(crate) struct AccessControl {
    trust_loopback_proxy: bool,
    state: Mutex<AccessState>,
    password_slots: Arc<Semaphore>,
}

impl Default for AccessControl {
    fn default() -> Self {
        Self::new(false)
    }
}

impl AccessControl {
    pub fn new(trust_loopback_proxy: bool) -> Self {
        Self {
            trust_loopback_proxy,
            state: Mutex::new(AccessState::default()),
            password_slots: Arc::new(Semaphore::new(2)),
        }
    }

    fn state(&self) -> MutexGuard<'_, AccessState> {
        self.state.lock().unwrap_or_else(|err| err.into_inner())
    }

    pub fn load(&self, db_path: &str) {
        let stored = ls_store::Store::open(db_path).and_then(|store| store.setting_get(POLICY_KEY));
        let policy = match stored {
            Ok(None) => Some(Policy::default()),
            Ok(Some(encoded)) => serde_json::from_str(&encoded).ok(),
            Err(_) => None,
        };
        self.state().policy = policy.unwrap_or_else(|| {
            eprintln!("[access] 无法载入密码设置。请使用本机恢复码重设。");
            Policy {
                local: true,
                lan: true,
                ..Policy::default()
            }
        });
    }

    pub fn request(&self, req: &Request, host: &str) -> BrowserAccess {
        let peer = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip().to_canonical());
        let loopback = peer.is_some_and(|ip| ip.is_loopback());
        let headers = req.headers();
        let local_host = crate::is_loopback_host(host);
        let trust_proxy = self
            .state()
            .policy
            .trust_proxy
            .unwrap_or(self.trust_loopback_proxy);
        let forwarded = (loopback && !local_host && trust_proxy)
            .then(|| single_header(headers, "x-forwarded-for"))
            .flatten()
            .and_then(|value| value.parse::<IpAddr>().ok());
        let client = Client {
            local: loopback && local_host,
            secure: forwarded.is_some()
                && single_header(headers, "x-forwarded-proto") == Some("https"),
            ip: forwarded.or(peer),
        };
        let session = client
            .can_use_password()
            .then(|| cookie_digest(headers, COOKIE))
            .flatten()
            .filter(|hash| self.session_valid(hash, client));
        BrowserAccess {
            client,
            session,
            device: None,
            device_expired: false,
            share: false,
        }
    }

    pub fn required(&self, client: Client) -> bool {
        let state = self.state();
        if client.local {
            state.policy.local
        } else {
            state.policy.lan
        }
    }

    pub fn lan_protected(&self) -> bool {
        self.state().policy.lan
    }

    pub fn session_valid(&self, digest: &str, client: Client) -> bool {
        self.state()
            .sessions
            .get(digest)
            .is_some_and(|session| session.expires > unix_now() && session.local == client.local)
    }

    async fn password_work<T: Send + 'static>(
        &self,
        client: Client,
        work: impl FnOnce() -> Result<T, AccessError> + Send + 'static,
    ) -> Result<T, AccessError> {
        if !client.can_use_password() {
            return Err(TLS_REQUIRED);
        }
        {
            let now = Instant::now();
            let mut state = self.state();
            state
                .attempts
                .retain(|_, (start, _)| now.saturating_duration_since(*start) < RATE_WINDOW);
            if state.attempts.len() >= 256 && !state.attempts.contains_key(&client.ip) {
                return Err(RATE_LIMITED);
            }
            let (_, tries) = state.attempts.entry(client.ip).or_insert((now, 0));
            if *tries >= 12 {
                return Err(RATE_LIMITED);
            }
            *tries += 1;
        }
        let permit = self
            .password_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| RATE_LIMITED)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        })
        .await
        .map_err(|_| STORAGE_ERROR)?
    }

    fn policy(&self) -> (Policy, u64) {
        let state = self.state();
        (state.policy.clone(), state.revision)
    }

    fn issue(&self, client: Client, revision: u64) -> Result<String, AccessError> {
        let mut state = self.state();
        check_revision(&state, revision)?;
        Ok(insert_session(&mut state, client))
    }

    fn apply(
        &self,
        db_path: &str,
        client: Client,
        policy: Policy,
        revision: u64,
        recovery: Option<&str>,
    ) -> Result<Option<String>, AccessError> {
        let mut state = self.state();
        check_revision(&state, revision)?;
        let store = ls_store::Store::open(db_path).map_err(|_| STORAGE_ERROR)?;
        let tx = store
            .conn()
            .unchecked_transaction()
            .map_err(|_| STORAGE_ERROR)?;
        if let Some(code) = recovery {
            let stored = store.setting_get(RECOVERY_KEY).map_err(|_| STORAGE_ERROR)?;
            let valid = stored
                .and_then(|raw| serde_json::from_str::<RecoveryCode>(&raw).ok())
                .is_some_and(|entry| {
                    entry.expires > unix_now()
                        && crate::middleware::ct_eq(&entry.digest, &sha256_hex(code.as_bytes()))
                });
            if !valid {
                return Err(INVALID_RECOVERY);
            }
        }
        // Acquire the database write lock before changing the policy. A recovery code can be used once.
        tx.execute("DELETE FROM app_settings WHERE key=?", [RECOVERY_KEY])
            .map_err(|_| STORAGE_ERROR)?;
        let encoded = serde_json::to_string(&policy).map_err(|_| STORAGE_ERROR)?;
        store
            .setting_set(POLICY_KEY, &encoded)
            .map_err(|_| STORAGE_ERROR)?;
        tx.commit().map_err(|_| STORAGE_ERROR)?;
        state.policy = policy;
        state.revision += 1;
        state.sessions.clear();
        Ok(state
            .policy
            .password_hash
            .is_some()
            .then(|| insert_session(&mut state, client)))
    }
}

fn check_revision(state: &AccessState, revision: u64) -> Result<(), AccessError> {
    if state.revision == revision {
        Ok(())
    } else {
        Err((StatusCode::CONFLICT, "访问设置已更新，请刷新后重试"))
    }
}

fn single_header<'a>(headers: &'a HeaderMap, name: &'static str) -> Option<&'a str> {
    let all = headers.get_all(name);
    let mut values = all.iter();
    let first = values.next()?.to_str().ok()?;
    (values.next().is_none() && !first.contains(',')).then_some(first)
}

pub(crate) fn cookie_digest(headers: &HeaderMap, cookie: &str) -> Option<String> {
    let value = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|raw| raw.to_str().ok())
        .flat_map(|raw| raw.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(name, value)| (name == cookie).then_some(value))?;
    (value.len() == 64 && value.bytes().all(|ch| ch.is_ascii_hexdigit()))
        .then(|| sha256_hex(value.as_bytes()))
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn insert_session(state: &mut AccessState, client: Client) -> String {
    let now = unix_now();
    state.sessions.retain(|_, session| session.expires > now);
    if state.sessions.len() >= 128 {
        if let Some(oldest) = state
            .sessions
            .iter()
            .min_by_key(|(_, s)| s.started)
            .map(|(id, _)| id.clone())
        {
            state.sessions.remove(&oldest);
        }
    }
    let token = random_token();
    state.sessions.insert(
        sha256_hex(token.as_bytes()),
        Session {
            local: client.local,
            started: now,
            expires: now + SESSION_SECONDS,
        },
    );
    token
}

fn hash_password(password: &str) -> Result<String, AccessError> {
    if password.chars().count() < MIN_PASSWORD_CHARS || password.len() > MAX_PASSWORD_BYTES {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "访问密码至少 12 个字符，最多 512 字节",
        ));
    }
    let mut salt_bytes = [0; 16];
    getrandom::getrandom(&mut salt_bytes).map_err(|_| STORAGE_ERROR)?;
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|_| STORAGE_ERROR)?;
    // Argon2id interactive profile. Password hashing runs outside the async executor.
    let params = Params::new(19 * 1024, 2, 1, Some(32)).map_err(|_| STORAGE_ERROR)?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| STORAGE_ERROR)
}

fn verify_password(password: &str, encoded: &str) -> Result<(), AccessError> {
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(INVALID_PASSWORD);
    }
    let hash = PasswordHash::new(encoded).map_err(|_| INVALID_PASSWORD)?;
    if hash.algorithm.as_str() != "argon2id" {
        return Err(INVALID_PASSWORD);
    }
    Argon2::default()
        .verify_password(password.as_bytes(), &hash)
        .map_err(|_| INVALID_PASSWORD)
}

#[derive(Serialize, Deserialize)]
struct RecoveryCode {
    digest: String,
    expires: u64,
}

pub(crate) fn recovery_code(db_path: &str) -> Result<String, &'static str> {
    let store = ls_store::Store::open(db_path).map_err(|_| STORAGE_ERROR.1)?;
    let code = random_token();
    let entry = RecoveryCode {
        digest: sha256_hex(code.as_bytes()),
        expires: unix_now() + RECOVERY_SECONDS,
    };
    let encoded = serde_json::to_string(&entry).map_err(|_| STORAGE_ERROR.1)?;
    store
        .setting_set(RECOVERY_KEY, &encoded)
        .map_err(|_| STORAGE_ERROR.1)?;
    Ok(code)
}

fn session_response(client: Client, result: Result<Option<String>, AccessError>) -> Response {
    let token = match result {
        Ok(value) => value,
        Err((status, detail)) => return (status, Json(json!({"detail": detail}))).into_response(),
    };
    let max_age = token.as_ref().map_or(0, |_| SESSION_SECONDS);
    set_cookie(
        Json(json!({"ok": true})).into_response(),
        COOKIE,
        token.as_deref().unwrap_or_default(),
        max_age,
        client.secure,
    )
}

pub(crate) fn set_cookie(
    mut response: Response,
    name: &str,
    value: &str,
    max_age: u64,
    https: bool,
) -> Response {
    let secure = if https { "; Secure" } else { "" };
    let cookie =
        format!("{name}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure}");
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("valid session cookie"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(crate) async fn status(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
) -> Response {
    let (policy, _) = st.access.policy();
    let required = if browser.client.local {
        policy.local
    } else {
        policy.lan
    };
    let mut response = Json(json!({
        "required": required, "signed_in": browser.session.is_some(),
        "local": browser.client.local, "password_supported": browser.client.can_use_password(),
        "password_set": policy.password_hash.is_some(),
        "device": browser.device.as_ref().map(|device| device.summary()),
        "device_expired": browser.device_expired,
    }))
    .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(crate) async fn settings(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
) -> Json<Value> {
    let state = st.access.state();
    let now = unix_now();
    let mut sessions = state
        .sessions
        .iter()
        .filter(|(_, s)| s.expires > now)
        .map(|(digest, s)| {
            (
                s.started,
                json!({
                    "started": s.started, "expires": s.expires, "local": s.local,
                    "current": browser.session.as_ref() == Some(digest),
                }),
            )
        })
        .collect::<Vec<_>>();
    sessions.sort_unstable_by_key(|(started, _)| std::cmp::Reverse(*started));
    Json(json!({
        "local": state.policy.local, "lan": state.policy.lan, "from_local": browser.client.local,
        "password_set": state.policy.password_hash.is_some(), "api_token_set": !st.admin_token.is_empty(),
        "trust_proxy": state.policy.trust_proxy.unwrap_or(st.access.trust_loopback_proxy),
        "sessions": sessions.into_iter().map(|(_, session)| session).collect::<Vec<_>>(),
    }))
}

#[derive(Deserialize)]
pub(crate) struct Login {
    password: String,
}

pub(crate) async fn login(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
    Json(body): Json<Login>,
) -> Response {
    let (policy, revision) = st.access.policy();
    let verified = st
        .access
        .password_work(browser.client, move || {
            let hash = policy.password_hash.as_deref().ok_or(INVALID_PASSWORD)?;
            verify_password(&body.password, hash)
        })
        .await;
    let result = verified
        .and_then(|()| st.access.issue(browser.client, revision))
        .map(Some);
    session_response(browser.client, result)
}

pub(crate) async fn logout(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
) -> Response {
    if let Some(digest) = browser.session {
        st.access.state().sessions.remove(&digest);
    }
    session_response(browser.client, Ok(None))
}

pub(crate) async fn revoke_others(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
) -> Json<Value> {
    st.access
        .state()
        .sessions
        .retain(|digest, _| browser.session.as_ref() == Some(digest));
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
pub(crate) struct ChangePassword {
    local: bool,
    lan: bool,
    password: Option<String>,
    current_password: Option<String>,
    trust_proxy: Option<bool>,
}

pub(crate) async fn update(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
    Json(body): Json<ChangePassword>,
) -> Response {
    let (old, revision) = st.access.policy();
    let client = browser.client;
    if old.password_hash.is_none() && st.admin_token.is_empty() && !client.local {
        return session_response(
            client,
            Err((StatusCode::FORBIDDEN, "请在服务所在电脑设置访问密码")),
        );
    }
    let shared = Arc::clone(&st);
    let result = st
        .access
        .password_work(client, move || {
            if let Some(encoded) = &old.password_hash {
                verify_password(
                    body.current_password.as_deref().unwrap_or_default(),
                    encoded,
                )?;
            } else if old.local || old.lan {
                return Err((StatusCode::CONFLICT, "请先使用本机恢复码重设密码"));
            }
            let password_hash = if body.local || body.lan {
                let encoded = match body.password {
                    Some(password) => hash_password(&password)?,
                    None => old
                        .password_hash
                        .ok_or((StatusCode::UNPROCESSABLE_ENTITY, "请输入访问密码"))?,
                };
                Some(encoded)
            } else {
                None
            };
            let policy = Policy {
                local: body.local,
                lan: body.lan,
                password_hash,
                trust_proxy: body.trust_proxy.or(old.trust_proxy),
            };
            shared
                .access
                .apply(&shared.db_path, client, policy, revision, None)
        })
        .await;
    session_response(client, result)
}

#[derive(Deserialize)]
pub(crate) struct Recover {
    code: String,
    password: String,
}

pub(crate) async fn recover(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
    Json(body): Json<Recover>,
) -> Response {
    let client = browser.client;
    if !client.local {
        return session_response(
            client,
            Err((StatusCode::FORBIDDEN, "请在服务所在电脑打开本机地址")),
        );
    }
    if body.code.len() != 64 || !body.code.bytes().all(|ch| ch.is_ascii_hexdigit()) {
        return session_response(client, Err(INVALID_RECOVERY));
    }
    let (mut policy, revision) = st.access.policy();
    let shared = Arc::clone(&st);
    let result = st
        .access
        .password_work(client, move || {
            policy.password_hash = Some(hash_password(&body.password)?);
            if !policy.lan {
                policy.local = true;
            }
            shared
                .access
                .apply(&shared.db_path, client, policy, revision, Some(&body.code))
        })
        .await;
    session_response(client, result)
}

pub(crate) fn routes() -> axum::Router<Shared> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/status", get(status))
        .route("/settings", get(settings).put(update))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/revoke-others", post(revoke_others))
        .route("/recover", post(recover))
        .layer(DefaultBodyLimit::max(4096))
}
