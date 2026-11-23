use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{multipart::Field, Multipart, Path as AxPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::face::{self, FaceApiProfile, FaceMode, FaceSettings, LOCAL_PROFILE, MAX_PHOTO};
use crate::local_face::{self, Photo};
use crate::state::Shared;
use crate::util::{detail_err, server_err, with_store};

type HttpFailure = (StatusCode, String);

fn invalid(detail: impl Into<String>) -> HttpFailure {
    (StatusCode::UNPROCESSABLE_ENTITY, detail.into())
}

fn unavailable(detail: impl Into<String>) -> HttpFailure {
    (StatusCode::SERVICE_UNAVAILABLE, detail.into())
}

fn upstream(detail: impl Into<String>) -> HttpFailure {
    (StatusCode::BAD_GATEWAY, detail.into())
}

fn db_error(_: rusqlite::Error) -> HttpFailure {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "人脸数据存储暂不可用".into(),
    )
}

async fn run_blocking(
    st: Shared,
    action: impl FnOnce(&Shared, &ls_store::Store) -> Result<Value, HttpFailure> + Send + 'static,
) -> Response {
    tokio::task::spawn_blocking(move || {
        let response = ls_store::Store::open(&st.db_path)
            .map_err(db_error)
            .and_then(|store| action(&st, &store));
        match response {
            Ok(data) => Json(data).into_response(),
            Err((status, detail)) => detail_err(status, &detail).into_response(),
        }
    })
    .await
    .unwrap_or_else(|_| server_err())
}

fn settings(st: &Shared, store: &ls_store::Store) -> FaceSettings {
    FaceSettings::load(store, Path::new(&st.data_dir))
}

fn status(st: &Shared, cfg: &FaceSettings) -> Value {
    let current = cfg.api(&cfg.active.api);
    let installed = local_face::assets_present(&st.data_dir);
    let configured = match cfg.active.mode {
        FaceMode::Off => false,
        FaceMode::Local => installed && cfg.local_enabled,
        FaceMode::Api => {
            current.is_some_and(|profile| profile.enabled && !profile.secret.is_empty())
        }
    };
    json!({"enabled": cfg.active_profile().is_some(), "configured": configured,
           "mode": cfg.active.mode,
           "threshold": if cfg.active.mode == FaceMode::Api { current.map_or(0.75, |p| p.threshold) } else { cfg.local_threshold }})
}

fn config_json(
    st: &Shared,
    store: &ls_store::Store,
    cfg: &FaceSettings,
) -> rusqlite::Result<Value> {
    let devices = st
        .faces
        .devices()
        .map(|providers| providers.map(|gpu| if gpu { "DirectML" } else { "CPU" }));
    let cameras: Vec<_> = store
        .cameras()?
        .iter()
        .filter(|camera| camera.type_ != "folder")
        .map(|c| {
            let allowed = store
                .setting_get(&format!("face.camera.{}", c.id))
                .ok()
                .flatten();
            json!({"id": c.id, "name": c.name, "permission": allowed.as_deref().unwrap_or("off"),
                   "alert_on_match": local_face::match_alert_enabled(store, c.id)})
        })
        .collect();
    Ok(
        json!({"status": status(st,cfg), "active": cfg.active, "previous": cfg.previous,
        "retention_days": cfg.retention_days, "apis": cfg.apis.iter().map(FaceApiProfile::public).collect::<Vec<_>>(),
        "local": {"enabled":cfg.local_enabled, "threshold":cfg.local_threshold,
                  "installed":local_face::assets_present(&st.data_dir), "devices": devices},
        "cameras": cameras, "pending_remote_cleanup": store.face_remote_removals()?.len()}),
    )
}

pub(crate) async fn face_status(State(st): State<Shared>) -> Response {
    with_store(&st, |store| Ok(Json(status(&st, &settings(&st, store)))))
}

pub(crate) async fn face_config(State(st): State<Shared>) -> Response {
    with_store(&st, |store| {
        Ok(Json(config_json(&st, store, &settings(&st, store))?))
    })
}

#[derive(Deserialize)]
pub(crate) struct ConfigPatch {
    mode: Option<FaceMode>,
    api: Option<String>,
    #[serde(default)]
    restore_previous: bool,
    local_enabled: Option<bool>,
    local_threshold: Option<f32>,
    retention_days: Option<u32>,
    profile: Option<ProfilePatch>,
}

#[derive(Deserialize)]
struct ProfilePatch {
    id: Option<String>,
    name: String,
    url: String,
    api_key: Option<String>,
    threshold: f32,
    timeout_s: u64,
    enabled: bool,
}

fn save_profile(
    st: &Shared,
    cfg: &mut FaceSettings,
    patch: ProfilePatch,
) -> Result<String, HttpFailure> {
    let old = patch.id.as_deref().and_then(|id| cfg.api(id));
    if patch.id.is_some() && old.is_none() {
        return Err(invalid("API 模型已不存在"));
    }
    let raw = patch.url.trim();
    let same_address = old.is_some_and(|old| old.base_url == raw);
    let address = face::normalize_url(raw, same_address).map_err(invalid)?;
    let name = patch.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(invalid("请输入 1 至 80 字的模型名称"));
    }
    if !(0.0..=1.0).contains(&patch.threshold) || !(1..=15).contains(&patch.timeout_s) {
        return Err(invalid("阈值应在 0–1 之间，超时应在 1–15 秒之间"));
    }
    let raw_key = patch.api_key.as_deref().unwrap_or_default().trim();
    if raw_key.len() > 4096 || raw_key.chars().any(char::is_control) {
        return Err(invalid("API 密钥格式无效"));
    }
    let data_dir = Path::new(&st.data_dir);
    let unchanged = old.filter(|old| {
        old.base_url == address
            && (raw_key.is_empty() || crate::fernet::unprotect(&old.secret, data_dir) == raw_key)
    });
    let id = unchanged.map_or_else(crate::envfile::random_token, |old| old.id.clone());
    let secret = match unchanged {
        Some(old) => old.secret.clone(),
        None if raw_key.is_empty() => return Err(invalid("新模型或新服务地址需要 API 密钥")),
        None => crate::fernet::protect(raw_key, data_dir)
            .ok_or_else(|| unavailable("无法加密 API 密钥"))?,
    };
    let profile = FaceApiProfile {
        id: id.clone(),
        name: name.into(),
        base_url: address,
        secret,
        enabled: patch.enabled,
        threshold: patch.threshold,
        timeout_s: patch.timeout_s,
    };
    if let Some(slot) = cfg.apis.iter_mut().find(|entry| entry.id == id) {
        *slot = profile;
    } else {
        cfg.apis.push(profile);
    }
    Ok(id)
}

pub(crate) async fn face_update_config(
    State(st): State<Shared>,
    Json(patch): Json<ConfigPatch>,
) -> Response {
    run_blocking(st, move |st, store| {
        let _guard = st
            .faces
            .config_lock
            .lock()
            .map_err(|_| unavailable("设置暂不可用"))?;
        let mut cfg = settings(st, store);
        let original = cfg.active.clone();
        if let Some(v) = patch.local_enabled {
            cfg.local_enabled = v;
        }
        if let Some(v) = patch.local_threshold {
            if !(0.0..=1.0).contains(&v) {
                return Err(invalid("本地识别阈值应在 0–1 之间"));
            }
            cfg.local_threshold = v;
        }
        if let Some(v) = patch.retention_days {
            if !(1..=365).contains(&v) {
                return Err(invalid("样本保留期限应在 1–365 天之间"));
            }
            cfg.retention_days = v;
        }
        let saved_profile = patch
            .profile
            .map(|input| save_profile(st, &mut cfg, input))
            .transpose()?;
        if patch.restore_previous {
            cfg.active = cfg
                .previous
                .clone()
                .ok_or_else(|| invalid("目前没有可切回的模型"))?;
        } else {
            let requested_api = patch.api.or_else(|| {
                if cfg.active.api.is_empty() || patch.mode == Some(FaceMode::Api) {
                    saved_profile.clone()
                } else {
                    None
                }
            });
            if let Some(id) = requested_api {
                if cfg.api(&id).is_none() {
                    return Err(invalid("请选择已有 API 模型"));
                }
                cfg.active.api = id;
            }
            if let Some(mode) = patch.mode {
                cfg.active.mode = mode;
            }
        }
        if cfg.active.mode != FaceMode::Off && cfg.active_profile().is_none() {
            if patch.mode.is_some() || patch.restore_previous {
                return Err(invalid("请先启用所选模型"));
            }
            cfg.active.mode = FaceMode::Off;
        }
        if original != cfg.active && original.mode != FaceMode::Off {
            cfg.previous = Some(original);
        }
        store
            .setting_set(
                "face.settings",
                &serde_json::to_string(&cfg).map_err(|_| unavailable("无法保存人脸配置"))?,
            )
            .map_err(db_error)?;
        if cfg.active.mode != FaceMode::Local {
            st.faces.reset_model();
        }
        let mut response = config_json(st, store, &cfg).map_err(db_error)?;
        if let Some(id) = saved_profile {
            response["saved_api"] = json!(id);
        }
        Ok(response)
    })
    .await
}

pub(crate) async fn face_install_local(State(st): State<Shared>) -> Response {
    run_blocking(st, |st, store| {
        st.faces.install(&st.data_dir).map_err(upstream)?;
        config_json(st, store, &settings(st, store)).map_err(db_error)
    })
    .await
}

pub(crate) async fn face_test_api(
    State(st): State<Shared>,
    AxPath(id): AxPath<String>,
) -> Response {
    run_blocking(st, move |st, store| {
        let cfg = settings(st, store);
        let profile = cfg.api(&id).ok_or_else(|| invalid("请选择已有 API 模型"))?;
        let client = profile
            .client(Path::new(&st.data_dir))
            .map_err(unavailable)?;
        let start = Instant::now();
        let people = client.list_persons().map_err(upstream)?;
        Ok(json!({"ok":true, "elapsed_ms":start.elapsed().as_millis(),"subjects":people.len()}))
    })
    .await
}

pub(crate) async fn face_set_camera(
    State(st): State<Shared>,
    AxPath(id): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    with_store(&st, |store| {
        let Some(camera) = store.camera(id)? else {
            return Ok(detail_err(StatusCode::NOT_FOUND, "摄像头不存在").into_response());
        };
        let permission = body.get("permission").and_then(Value::as_str).unwrap_or("");
        if camera.type_ == "folder" || !matches!(permission, "off" | "local" | "local_or_api") {
            return Ok(detail_err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "请为视频源选择有效的人脸识别范围",
            )
            .into_response());
        }
        let alert = permission != "off"
            && body
                .get("alert_on_match")
                .and_then(Value::as_bool)
                .unwrap_or_else(|| local_face::match_alert_enabled(store, id));
        store.settings_set(&[
            (&format!("face.camera.{id}"), permission),
            (
                &format!("face.camera_alert.{id}"),
                if alert { "1" } else { "0" },
            ),
        ])?;
        Ok(Json(json!({"ok":true,"permission":permission,"alert_on_match":alert})).into_response())
    })
}

#[derive(Default)]
struct PhotoInput {
    subject: String,
    consent: bool,
    photos: Vec<Vec<u8>>,
}

async fn read_field(mut field: Field<'_>, limit: usize) -> Result<Vec<u8>, HttpFailure> {
    let mut bytes = Vec::new();
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|_| invalid("上传内容读取失败"))?
    {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err((StatusCode::PAYLOAD_TOO_LARGE, "上传内容超过大小限制".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn photos(mut form: Multipart) -> Result<PhotoInput, HttpFailure> {
    let mut input = PhotoInput::default();
    while let Some(field) = form
        .next_field()
        .await
        .map_err(|_| invalid("上传内容读取失败"))?
    {
        match field.name() {
            Some("photo") if input.photos.len() < 5 => {
                input.photos.push(read_field(field, MAX_PHOTO).await?)
            }
            Some("photo") => return Err(invalid("一次最多上传 5 张照片")),
            Some("subject") => {
                input.subject = String::from_utf8(read_field(field, 512).await?)
                    .map_err(|_| invalid("人员名称必须是有效文本"))?
                    .trim()
                    .to_string();
            }
            Some("consent") => {
                input.consent = matches!(
                    read_field(field, 16).await?.as_slice(),
                    b"true" | b"1" | b"on"
                )
            }
            _ => {}
        }
    }
    if input.photos.is_empty() {
        return Err(invalid("请选择人脸照片"));
    }
    Ok(input)
}

fn enabled(cfg: &FaceSettings) -> Result<&str, HttpFailure> {
    cfg.active_profile().ok_or_else(|| {
        (
            StatusCode::FORBIDDEN,
            "人脸识别未开启，请在设置中选择并启用模型".into(),
        )
    })
}

pub(crate) async fn face_register(State(st): State<Shared>, form: Multipart) -> Response {
    let input = match photos(form).await {
        Ok(input) => input,
        Err((status, detail)) => return detail_err(status, &detail).into_response(),
    };
    if !input.consent
        || input.subject.is_empty()
        || input.subject.chars().count() > 80
        || input.subject.chars().any(char::is_control)
    {
        return detail_err(
            StatusCode::BAD_REQUEST,
            "请输入有效的人员姓名，并确认已获得本人的录入同意",
        )
        .into_response();
    }
    run_blocking(st, move |st, store| {
        let _permit = Arc::clone(&st.faces.slots)
            .try_acquire_owned()
            .map_err(|_| unavailable("人脸模型忙，请稍后重试"))?;
        let _library = st
            .faces
            .library_lock
            .lock()
            .map_err(|_| unavailable("人脸库暂不可用"))?;
        let cfg = settings(st, store);
        let profile = enabled(&cfg)?;
        let prior = store
            .face_person(profile, &input.subject)
            .map_err(db_error)?;
        if prior.as_ref().is_some_and(|person| person.removing) {
            return Err(invalid("该人员正在撤回登记，请先完成删除再录入"));
        }
        let mut person = ls_store::FacePerson {
            id: 0,
            profile: profile.into(),
            subject: input.subject,
            remote_subject: None,
            consent_ts: ls_core::time::local_now_iso(),
            expires_at: local_face::unix_now() + i64::from(cfg.retention_days) * 86_400,
            removing: false,
            samples: 0,
        };
        let photos = input
            .photos
            .iter()
            .map(|data| Photo::decode(data, None).map_err(invalid))
            .collect::<Result<Vec<_>, _>>()?;
        let dir = Path::new(&st.data_dir);
        let still_current = || settings(st, store).active_profile() == Some(profile);
        let id = if profile == LOCAL_PROFILE {
            let mut templates = Vec::new();
            for photo in &photos {
                let embedding = st
                    .faces
                    .feature(&st.data_dir, store, photo)
                    .map_err(invalid)?;
                let data = serde_json::to_string(&embedding)
                    .map_err(|_| unavailable("无法生成有效人脸模板"))?;
                templates.push(
                    crate::fernet::protect(&data, dir)
                        .ok_or_else(|| unavailable("无法加密人脸模板"))?,
                );
            }
            if !still_current() {
                return Err(invalid("录入期间切换了模型，请重新录入"));
            }
            store
                .save_face_person(&person, &templates)
                .map_err(db_error)?
        } else {
            let client = cfg
                .api(profile)
                .ok_or_else(|| invalid("当前 API 不存在"))?
                .client(dir)
                .map_err(unavailable)?;
            let remote = format!("LabSentinel-{}", crate::envfile::random_token());
            let prior_remote = prior.and_then(|old| old.remote_subject);
            store
                .queue_face_remote_removal(profile, &remote)
                .map_err(db_error)?;
            person.remote_subject = Some(remote.clone());
            let mut examples = Vec::new();
            let result = (|| {
                for photo in &photos {
                    if !still_current() {
                        return Err(invalid("录入期间切换了模型，请重新录入"));
                    }
                    examples.push(
                        client
                            .add_person(&remote, &photo.jpeg().map_err(invalid)?)
                            .map_err(upstream)?,
                    );
                }
                if !still_current() {
                    return Err(invalid("录入期间切换了模型，请重新录入"));
                }
                store.save_face_person(&person, &examples).map_err(db_error)
            })();
            let obsolete = if result.is_ok() {
                prior_remote.as_deref()
            } else {
                Some(remote.as_str())
            };
            if let Some(obsolete) = obsolete {
                if client.delete_person(obsolete).is_ok() {
                    let _ = store.clear_face_remote_removal(profile, obsolete);
                }
            }
            result?
        };
        Ok(
            json!({"ok":true,"id":id,"subject":person.subject,"consent_ts":person.consent_ts,
            "expires_at":person.expires_at,"samples_added":photos.len()}),
        )
    })
    .await
}

pub(crate) async fn face_recognize(State(st): State<Shared>, form: Multipart) -> Response {
    let input = match photos(form).await {
        Ok(input) if input.photos.len() == 1 => input,
        Ok(_) => {
            return detail_err(StatusCode::BAD_REQUEST, "试识别一次请选择一张照片").into_response()
        }
        Err((status, detail)) => return detail_err(status, &detail).into_response(),
    };
    if !input.consent {
        return detail_err(StatusCode::BAD_REQUEST, "请确认照片用于试识别的授权").into_response();
    }
    run_blocking(st, move |st, store| {
        let _permit = Arc::clone(&st.faces.slots).try_acquire_owned().map_err(|_| unavailable("人脸模型忙，请稍后重试"))?;
        let cfg = settings(st, store);
        enabled(&cfg)?;
        let photo = Photo::decode(&input.photos[0], None).map_err(invalid)?;
        let start = Instant::now();
        let result = local_face::recognize(st, store, &cfg, &photo).map_err(|error| {
            if cfg.active.mode == FaceMode::Api { upstream(error) } else { invalid(error) }
        })?;
        if !cfg.same_recognition(&settings(st, store)) {
            return Err(invalid("试识别期间切换了模型，请重试"));
        }
        let matched = result.map(|m| json!({"subject":m.subject,"similarity":m.similarity}));
        Ok(json!({"matched":matched,"mode":cfg.active.mode,"elapsed_ms":start.elapsed().as_millis()}))
    }).await
}

pub(crate) async fn face_library(State(st): State<Shared>) -> Response {
    run_blocking(st, |_, store| {
        Ok(json!(store.face_people().map_err(db_error)?))
    })
    .await
}

pub(crate) async fn face_persons_list(State(st): State<Shared>) -> Response {
    run_blocking(st, |st, store| {
        let cfg = settings(st, store);
        let profile = enabled(&cfg)?;
        let people = store.face_people().map_err(db_error)?;
        let mut rows: Vec<_> = people.iter().filter(|person| person.profile == profile)
            .map(|person| json!(person)).collect();
        if profile.starts_with("legacy-") {
            let client = cfg.api(profile).ok_or_else(|| unavailable("未配置 API 模型"))?
                .client(Path::new(&st.data_dir)).map_err(unavailable)?;
            for subject in client.list_persons().map_err(upstream)? {
                if !people.iter().any(|person| person.remote_subject.as_deref() == Some(&subject)) {
                    rows.push(json!({"subject":subject,"consent_ts":store.face_consent_ts(&subject).map_err(db_error)?}));
                }
            }
        }
        Ok(json!(rows))
    }).await
}

pub(crate) fn remove_person(
    st: &Shared,
    store: &ls_store::Store,
    cfg: &FaceSettings,
    person: &ls_store::FacePerson,
) -> Result<(), HttpFailure> {
    store.mark_face_removing(person.id).map_err(db_error)?;
    if let Some(subject) = &person.remote_subject {
        let client = cfg
            .api(&person.profile)
            .ok_or_else(|| unavailable("远程模型不可达，删除仍待同步"))?
            .client(Path::new(&st.data_dir))
            .map_err(upstream)?;
        client.delete_person(subject).map_err(upstream)?;
    }
    store.delete_face_person(person.id).map_err(db_error)
}

pub(crate) async fn face_remove_from_library(
    State(st): State<Shared>,
    AxPath(id): AxPath<i64>,
) -> Response {
    run_blocking(st, move |st, store| {
        let _guard = st
            .faces
            .library_lock
            .lock()
            .map_err(|_| unavailable("人脸库暂不可用"))?;
        let person = store
            .face_person_by_id(id)
            .map_err(db_error)?
            .ok_or_else(|| (StatusCode::NOT_FOUND, "人员记录不存在".into()))?;
        remove_person(st, store, &settings(st, store), &person)?;
        Ok(json!({"ok":true}))
    })
    .await
}

pub(crate) async fn face_delete(
    State(st): State<Shared>,
    AxPath(subject): AxPath<String>,
) -> Response {
    run_blocking(st, move |st, store| {
        let _guard = st
            .faces
            .library_lock
            .lock()
            .map_err(|_| unavailable("人脸库暂不可用"))?;
        let cfg = settings(st, store);
        let profile = enabled(&cfg)?;
        if let Some(person) = store.face_person(profile, &subject).map_err(db_error)? {
            remove_person(st, store, &cfg, &person)?;
        } else if profile.starts_with("legacy-") {
            let client = cfg
                .api(profile)
                .ok_or_else(|| unavailable("未配置 API 模型"))?
                .client(Path::new(&st.data_dir))
                .map_err(unavailable)?;
            client.delete_person(&subject).map_err(upstream)?;
            store.delete_face_consent(&subject).map_err(db_error)?;
        } else {
            return Err((StatusCode::NOT_FOUND, "人员记录不存在".into()));
        }
        Ok(json!({"ok":true}))
    })
    .await
}

pub(crate) async fn face_clear_all(State(st): State<Shared>) -> Response {
    run_blocking(st, |st, store| {
        let _guard = st
            .faces
            .library_lock
            .lock()
            .map_err(|_| unavailable("人脸库暂不可用"))?;
        let cfg = settings(st, store);
        let people = store.face_people().map_err(db_error)?;
        let mut failures = 0;
        for person in &people {
            if remove_person(st, store, &cfg, person).is_err() {
                failures += 1;
            }
        }
        Ok(json!({"ok":failures==0,"pending":failures,"deleted":people.len()-failures}))
    })
    .await
}

pub(crate) async fn face_event_match(
    State(st): State<Shared>,
    AxPath(id): AxPath<i64>,
) -> Response {
    with_store(&st, |store| {
        let matched = store
            .event_face_match(id, local_face::unix_now())?
            .map(|(subject, similarity)| json!({"subject":subject,"similarity":similarity}));
        Ok(Json(json!({"matched":matched})))
    })
}
