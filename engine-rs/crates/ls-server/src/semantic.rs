use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use ls_infer::embed::{ImageEncoder, TextEncoder, ENCODER, TEXT_FILE, TOKENIZER_FILE, VISION_FILE};
use ls_store::Store;
use serde::Deserialize;
use serde_json::json;

use crate::{access::BrowserAccess, handlers::events::event_json, state::Shared, util::*};

const SETTING: &str = "semantic.enabled";
const MODEL_DIRECTORY: &str = "models/semantic";
const ASSETS: [&str; 3] = [TOKENIZER_FILE, TEXT_FILE, VISION_FILE];

fn verify_asset(root: &Path, file: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let expected = match file {
        TOKENIZER_FILE => "cb9140fae3ac5122c972d37adf83e1248471a38147ad76f8215c8872c6fd8322",
        TEXT_FILE => "3a0603d3a00c05a80a6ded4743c16aaac7b1e62cdcc7e362e7ce418659b96400",
        VISION_FILE => "5f2b401c1a4fc095702a5d45348e17ad46c4f87064085365b43c6e8eaa5c0070",
        _ => return Err("未知离线搜图资产".into()),
    };
    let path = rooted_file(root, Path::new(file)).ok_or("找不到离线搜图资产")?;
    let mut input = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut buf = vec![0; 65_536];
    loop {
        let count = input.read(&mut buf).map_err(|err| err.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buf[..count]);
    }
    if format!("{:x}", digest.finalize()) != expected {
        return Err(format!("{file} 和已验证的 SigLIP 2 文件摘要不符"));
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct SemanticRuntime {
    text: Mutex<Option<TextEncoder>>,
    image: Mutex<Option<ImageEncoder>>,
}

fn model_root(st: &Shared) -> Option<PathBuf> {
    let data = Path::new(&st.data_dir);
    let root = data.join(MODEL_DIRECTORY).canonicalize().ok()?;
    if !root.starts_with(data.canonicalize().ok()?) {
        return None;
    }
    ASSETS
        .iter()
        .all(|file| rooted_file(&root, Path::new(file)).is_some())
        .then_some(root)
}

fn enabled(store: &Store) -> bool {
    store.setting_get(SETTING).ok().flatten().as_deref() == Some("1")
}

pub(crate) async fn status(State(st): State<Shared>) -> Response {
    with_store(&st, |store| {
        Ok(Json(json!({
            "enabled": enabled(store),
            "available": model_root(&st).is_some(),
            "directory": MODEL_DIRECTORY, "model": ENCODER,
        }))
        .into_response())
    })
}

#[derive(Deserialize)]
pub(crate) struct Config {
    enabled: bool,
}

pub(crate) async fn configure(State(st): State<Shared>, Json(config): Json<Config>) -> Response {
    if config.enabled && model_root(&st).is_none() {
        return detail_err(
            StatusCode::CONFLICT,
            "请先准备本地图像、文字编码权重与分词器",
        );
    }
    with_store(&st, |store| {
        store.setting_set(SETTING, if config.enabled { "1" } else { "0" })?;
        Ok(Json(json!({"enabled": config.enabled})).into_response())
    })
}

#[derive(Deserialize)]
pub(crate) struct ExploreQuery {
    q: Option<String>,
    similar_to: Option<i64>,
    limit: Option<i64>,
}

pub(crate) async fn explore(
    State(st): State<Shared>,
    Extension(browser): Extension<BrowserAccess>,
    Query(query): Query<ExploreQuery>,
) -> Response {
    if !matches!(
        (&query.q, query.similar_to),
        (Some(_), None) | (None, Some(_))
    ) || query
        .q
        .as_ref()
        .is_some_and(|text| text.trim().is_empty() || text.chars().count() > 256)
    {
        return detail_err(
            StatusCode::BAD_REQUEST,
            "请输入 1 至 256 个字符或选择一个事件",
        );
    }
    tokio::task::spawn_blocking(move || {
        with_store(&st, |store| {
            let Some(root) = model_root(&st).filter(|_| enabled(store)) else {
                return Ok(detail_err(StatusCode::CONFLICT, "离线搜图尚未启用"));
            };
            let scope = browser.device.as_ref().and_then(|d| d.cameras.as_deref());
            let embedding = if let Some(id) = query.similar_to {
                let Some(event) = store
                    .event_by_id(id)?
                    .filter(|e| scope.is_none_or(|ids| ids.contains(&e.camera_id)))
                else {
                    return Ok(detail_err(StatusCode::NOT_FOUND, "未找到该事件"));
                };
                let Some(vector) = store.event_embedding(&event, ENCODER)? else {
                    return Ok(detail_err(StatusCode::CONFLICT, "该事件截图暂未编入索引"));
                };
                vector
            } else {
                let Ok(mut slot) = st.semantic.text.try_lock() else {
                    return Ok(detail_err(
                        StatusCode::TOO_MANY_REQUESTS,
                        "离线搜图繁忙，请稍后重试",
                    ));
                };
                if slot.is_none() {
                    match verify_asset(&root, TOKENIZER_FILE)
                        .and_then(|()| verify_asset(&root, TEXT_FILE))
                        .and_then(|()| TextEncoder::load(&root))
                    {
                        Ok(model) => *slot = Some(model),
                        Err(error) => {
                            crate::logger::error(&format!("[semantic] {error}"));
                            return Ok(detail_err(
                                StatusCode::SERVICE_UNAVAILABLE,
                                "无法加载离线分词或文字模型",
                            ));
                        }
                    }
                }
                let query_text = query.q.as_deref().unwrap_or_default();
                match slot.as_mut().unwrap().encode(query_text) {
                    Ok(vector) => vector,
                    Err(_) => {
                        return Ok(detail_err(StatusCode::UNPROCESSABLE_ENTITY, "文字编码失败"))
                    }
                }
            };
            let ranked = store.rank_event_embeddings(
                ENCODER,
                &embedding,
                scope,
                query.similar_to,
                query.limit.unwrap_or(30),
            )?;
            let mut results = Vec::with_capacity(ranked.len());
            for (id, distance) in ranked {
                if let Some(event) = store
                    .event_by_id(id)?
                    .filter(|e| scope.is_none_or(|ids| ids.contains(&e.camera_id)))
                {
                    results
                        .push(json!({"event": event_json(&event), "similarity": 1.0 - distance}));
                }
            }
            Ok(Json(json!({"results": results})).into_response())
        })
    })
    .await
    .unwrap_or_else(|_| server_err())
}

pub(crate) fn start(st: Shared) {
    tokio::spawn(async move {
        let mut cursor = 0;
        loop {
            let worker = st.clone();
            let (next, wait) =
                match tokio::task::spawn_blocking(move || index_one(&worker, cursor)).await {
                    Ok(Ok(next)) => next,
                    Ok(Err(error)) => {
                        crate::logger::error(&format!("[semantic] {error}"));
                        (cursor, 60)
                    }
                    Err(_) => (cursor, 60),
                };
            cursor = next;
            tokio::time::sleep(Duration::from_secs(wait)).await;
        }
    });
}

fn index_one(
    st: &Shared,
    cursor: i64,
) -> Result<(i64, u64), Box<dyn std::error::Error + Send + Sync>> {
    let store = Store::open(&st.db_path)?;
    let Some(root) = model_root(st).filter(|_| enabled(&store)) else {
        *st.semantic
            .image
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        if let Ok(mut text) = st.semantic.text.try_lock() {
            *text = None;
        }
        return Ok((0, 5));
    };
    let Some(event) = store.next_unindexed_event(ENCODER, cursor)? else {
        store.prune_event_vectors()?;
        return Ok((0, 15));
    };
    let Some((bytes, image)) = event
        .snapshot
        .as_deref()
        .and_then(|path| rooted_file(Path::new(&st.data_dir), Path::new(path)))
        .and_then(|path| ls_jobs::media::read_snapshot(&path))
        .and_then(|bytes| ls_jobs::media::decode_image(&bytes).map(|image| (bytes, image)))
    else {
        return Ok((event.id, 1));
    };
    let mut slot = st
        .semantic
        .image
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if slot.is_none() {
        verify_asset(&root, VISION_FILE)?;
        *slot = Some(ImageEncoder::load(&root)?);
    }
    let image = image
        .resize_exact(224, 224, image::imageops::FilterType::Triangle)
        .to_rgb8();
    let mut bgr = image.into_raw();
    for pixel in bgr.as_chunks_mut::<3>().0 {
        pixel.swap(0, 2);
    }
    let vector = slot.as_mut().unwrap().encode(&bgr, 224, 224)?;
    if enabled(&store) {
        store.index_event_embedding(&event, ENCODER, &sha256_hex(&bytes), &vector)?;
    }
    Ok((event.id, 3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::helpers::{body_json, req_hdr, test_state_cfg_dir};

    #[tokio::test]
    #[ignore = "requires the verified optional models and packaged ONNX Runtime"]
    async fn semantic_search_indexes_project_snapshots_and_rechecks_device_scope() {
        let assets = PathBuf::from(std::env::var_os("LS_SIGLIP_SMOKE_DIR").unwrap());
        let temp = tempfile::tempdir_in(&assets).unwrap();
        let data = temp.path();
        let asset_dir = data.join(MODEL_DIRECTORY);
        std::fs::create_dir_all(&asset_dir).unwrap();
        for name in ASSETS {
            std::fs::hard_link(assets.join(name), asset_dir.join(name)).unwrap();
        }
        let acceptance =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Resources/acceptance/imgs");
        let db = data.join("semantic.db");
        let db = db.to_str().unwrap();
        let store = Store::open(db).unwrap();
        store.ensure_schema().unwrap();
        store.migrate().unwrap();
        let mut ids = Vec::new();
        for (camera, file) in [
            (1, "person-frame.jpg"),
            (2, "empty-frame.jpg"),
            (1, "lab-day-cctv.jpg"),
        ] {
            std::fs::copy(acceptance.join(file), data.join(file)).unwrap();
            store.conn().execute("INSERT OR IGNORE INTO cameras(id, name,source,type) VALUES(?1, 'fixture','0','usb')", [camera]).unwrap();
            let id = store
                .add_event(&ls_store::NewEvent {
                    camera_id: camera,
                    type_: "no_mask",
                    level: 2,
                    track_id: 1,
                    detail: "fixture",
                    snapshot: Some(file),
                    ts: "2026-11-25T12:00:00",
                })
                .unwrap();
            ids.push(id);
        }
        let st = test_state_cfg_dir(db, "semantic-admin", true, data.to_str().unwrap());
        let app = crate::build_router(st.clone()).layer(Extension(axum::extract::ConnectInfo(
            "127.0.0.1:30000".parse::<std::net::SocketAddr>().unwrap(),
        )));
        let admin = [("x-admin-token", "semantic-admin")];
        let device_token = "1".repeat(64);
        let cookie = format!("ls_device={device_token}");
        let device = [("cookie", cookie.as_str())];
        let policy = |camera| {
            json!({
                "role": "read_only", "all_cameras": false, "camera_ids": [camera],
                "groups": [], "expires_at": crate::access::unix_now() + 60,
            })
            .to_string()
        };
        let device_id = store
            .add_browser_device(
                "scoped",
                &sha256_hex(device_token.as_bytes()),
                &policy(1),
                None,
            )
            .unwrap();
        let enable = req_hdr(
            &app,
            "PUT",
            "/api/events/explore/settings",
            Some(json!({"enabled":true})),
            &admin,
        )
        .await;
        assert_eq!(enable.status(), StatusCode::OK);
        let mut cursor = 0;
        for id in &ids {
            (cursor, _) = index_one(&st, cursor).unwrap();
            assert_eq!(cursor, *id);
        }
        let r = req_hdr(
            &app,
            "GET",
            "/api/events/explore?q=white%20lab%20coat",
            None,
            &admin,
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(body_json(r).await["results"].as_array().unwrap().len(), 3);
        let path = format!("/api/events/explore?similar_to={}", ids[0]);
        let r = req_hdr(&app, "GET", &path, None, &device).await;
        assert_eq!(r.status(), StatusCode::OK);
        let result = body_json(r).await;
        let hits = result["results"].as_array().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["event"]["id"], ids[2]);
        store
            .update_browser_device(device_id, "scoped", &policy(2))
            .unwrap();
        assert_eq!(
            req_hdr(&app, "GET", &path, None, &device).await.status(),
            StatusCode::NOT_FOUND
        );
    }
}
