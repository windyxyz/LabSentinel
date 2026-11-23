use crate::state::Shared;
use crate::util::{detail_err, with_store};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use ls_store::MaskKind;
use serde::Deserialize;
use serde_json::{json, Value};

pub(crate) async fn spatial_settings(State(st): State<Shared>, Path(cid): Path<i64>) -> Response {
    with_store(&st, |store| {
        if store.camera(cid)?.is_none() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "监控源不存在").into_response());
        }
        Ok(
            Json(json!({"masks": store.camera_masks(cid)?, "spots": store.fp_spots(cid)?}))
                .into_response(),
        )
    })
}

#[derive(Deserialize)]
pub(crate) struct NewMask {
    name: String,
    kind: MaskKind,
    polygon: Value,
}

pub(crate) async fn create_camera_mask(
    State(st): State<Shared>,
    Path(cid): Path<i64>,
    Json(body): Json<NewMask>,
) -> Response {
    let name = body.name.trim();
    if name.is_empty()
        || name.chars().count() > 80
        || !super::zones_rules::valid_zone_polygon(&body.polygon)
    {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "请输入遮罩名称和 3 至 256 个整数坐标顶点",
        )
        .into_response();
    }
    let Ok(polygon) = serde_json::from_value::<Vec<ls_core::Point>>(body.polygon) else {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "遮罩坐标无效").into_response();
    };
    if polygon
        .iter()
        .any(|&(x, y)| !(0..=1280).contains(&x) || !(0..=720).contains(&y))
    {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "遮罩的顶点须在画面内")
            .into_response();
    }
    with_store(&st, |store| {
        let tx = rusqlite::Transaction::new_unchecked(
            store.conn(),
            rusqlite::TransactionBehavior::Immediate,
        )?;
        if store.camera(cid)?.is_none() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "监控源不存在").into_response());
        }
        let id = store.add_camera_mask(cid, name, body.kind, &polygon)?;
        tx.commit()?;
        let revision = st.workers.notify_policy_changed();
        Ok(Json(json!({"id": id, "policy_revision": revision})).into_response())
    })
}

pub(crate) async fn remove_camera_mask(
    State(st): State<Shared>,
    Path((cid, id)): Path<(i64, i64)>,
) -> Response {
    with_store(&st, |store| {
        if store.delete_camera_mask(cid, id)? == 0 {
            return Ok(detail_err(StatusCode::NOT_FOUND, "遮罩不存在").into_response());
        }
        let revision = st.workers.notify_policy_changed();
        Ok(Json(json!({"ok": true, "policy_revision": revision})).into_response())
    })
}

pub(crate) async fn remember_event_spot(
    State(st): State<Shared>,
    Path(eid): Path<i64>,
) -> Response {
    with_store(&st, |store| {
        let Some((id, cid)) = store.remember_false_positive(eid)? else {
            return Ok(detail_err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "请先标记误报，且事件需要有可定位的单个检测框",
            )
            .into_response());
        };
        let revision = st.workers.notify_policy_changed();
        Ok(Json(json!({"id": id, "camera_id": cid, "policy_revision": revision})).into_response())
    })
}

pub(crate) async fn remove_false_positive_spot(
    State(st): State<Shared>,
    Path((cid, id)): Path<(i64, i64)>,
) -> Response {
    with_store(&st, |store| {
        if store.delete_fp_spot(cid, id)? == 0 {
            return Ok(detail_err(StatusCode::NOT_FOUND, "误报点不存在").into_response());
        }
        let revision = st.workers.notify_policy_changed();
        Ok(Json(json!({"ok": true, "policy_revision": revision})).into_response())
    })
}

#[derive(Deserialize)]
pub(crate) struct AuditQuery {
    camera_id: Option<i64>,
    before_id: Option<i64>,
    limit: Option<i64>,
}

pub(crate) async fn suppressions_list(
    State(st): State<Shared>,
    Query(query): Query<AuditQuery>,
) -> Response {
    with_store(&st, |store| {
        Ok(Json(store.suppressions(
            query.camera_id,
            query.before_id,
            query.limit.unwrap_or(50),
        )?))
    })
}
