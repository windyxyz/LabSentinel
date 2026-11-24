use std::collections::HashSet;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use ls_core::lab::{PpeItem, PpePolicy};
use ls_store::Store;
use rusqlite::{Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    state::Shared,
    util::{detail_err, sha256_hex, with_store},
};

#[derive(Clone, Copy, Serialize)]
struct Pack {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    required: &'static [PpeItem],
    after_hours: bool,
}

const PACKS: [Pack; 4] = [
    Pack {
        id: "wet_chem",
        name: "化学实验区",
        description: "实验服及护目镜组合",
        required: &[PpeItem::Coat, PpeItem::Goggles],
        after_hours: false,
    },
    Pack {
        id: "biosafety",
        name: "生物实验区",
        description: "实验服与口罩组合",
        required: &[PpeItem::Coat, PpeItem::Mask],
        after_hours: false,
    },
    Pack {
        id: "optical_laser",
        name: "激光设备区",
        description: "确认护目镜可见。型号和适用波长由现场核实。",
        required: &[PpeItem::Goggles],
        after_hours: false,
    },
    Pack {
        id: "after_hours_unattended",
        name: "非工作时段值守",
        description: "每天 22:00–06:00，单人连续留守超过 300 秒时提示",
        required: &[],
        after_hours: true,
    },
];
const REVISION: u32 = 1;

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    camera_id: i64,
    #[serde(default)]
    zone_id: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackRequest {
    pack_id: String,
    targets: Vec<Target>,
    #[serde(default)]
    replace_existing_policy: bool,
    #[serde(default)]
    preview_hash: Option<String>,
}

#[derive(Serialize)]
struct Change {
    target: Target,
    camera: String,
    zone: Option<String>,
    action: &'static str,
    existing_ppe: Option<PpePolicy>,
    existing_rules: Vec<ls_store::RuleFullRow>,
}

fn preview(store: &Store, request: &PackRequest, pack: Pack) -> Result<Vec<Change>, String> {
    if request.targets.is_empty() || request.targets.len() > 32 {
        return Err("请选择 1 至 32 个目标机位".into());
    }
    let mut unique = HashSet::new();
    let rules = store.rules_all().map_err(|e| e.to_string())?;
    request
        .targets
        .iter()
        .map(|&target| {
            if !unique.insert((target.camera_id, target.zone_id)) {
                return Err("存在重复机位和区域".into());
            }
            let camera = store
                .camera(target.camera_id)
                .map_err(|e| e.to_string())?
                .ok_or("目标摄像头不存在")?;
            if !matches!(camera.type_.as_str(), "usb" | "rtsp" | "file") {
                return Err("该模板需要连续视频画面".into());
            }
            let zone = if let Some(id) = target.zone_id {
                let zone = store
                    .zone(id)
                    .map_err(|e| e.to_string())?
                    .ok_or("目标区域不存在")?;
                if zone.1 != target.camera_id {
                    return Err("目标区域与所选摄像头不匹配".into());
                }
                Some(zone.2)
            } else {
                None
            };
            let existing_ppe = store
                .lab_rules(target.camera_id)
                .map_err(|e| e.to_string())?
                .ppe_policies
                .into_iter()
                .find(|policy| policy.zone_id == target.zone_id);
            let existing_rules: Vec<_> = rules
                .iter()
                .filter(|r| {
                    r.1 == target.camera_id
                        && r.2 == target.zone_id
                        && r.3 == "solo_after_hours"
                        && pack.after_hours
                })
                .cloned()
                .collect();
            let action = if pack.after_hours {
                if existing_rules.is_empty() {
                    "add"
                } else if existing_rules.iter().any(|r| {
                    r.6.as_deref() == Some("22:00")
                        && r.7.as_deref() == Some("06:00")
                        && r.8 == 300
                        && r.9 == 1
                        && matches!(r.10.as_deref(), None | Some("" | "1,2,3,4,5,6,7"))
                        && !r.11
                }) {
                    "unchanged"
                } else {
                    "conflict"
                }
            } else {
                match &existing_ppe {
                    None => "add",
                    Some(ppe)
                        if ppe.alternatives.len() == 1
                            && ppe.alternatives[0].len() == pack.required.len()
                            && pack
                                .required
                                .iter()
                                .all(|item| ppe.alternatives[0].contains(item)) =>
                    {
                        "unchanged"
                    }
                    Some(_) if request.replace_existing_policy => "replace",
                    Some(_) => "conflict",
                }
            };
            Ok(Change {
                target,
                camera: camera.name,
                zone,
                action,
                existing_ppe,
                existing_rules,
            })
        })
        .collect()
}

fn plan(pack: Pack, changes: &[Change]) -> (Value, String) {
    let plan = json!({"pack": pack, "version": REVISION, "changes": changes});
    let hash = sha256_hex(plan.to_string().as_bytes());
    (plan, hash)
}

pub(crate) async fn scene_packs() -> Response {
    Json(json!({"packs": PACKS, "version": REVISION})).into_response()
}

pub(crate) async fn scene_preview(
    State(st): State<Shared>,
    Json(request): Json<PackRequest>,
) -> Response {
    run(&st, request, false)
}

pub(crate) async fn scene_apply(
    State(st): State<Shared>,
    Json(request): Json<PackRequest>,
) -> Response {
    run(&st, request, true)
}

fn run(st: &Shared, request: PackRequest, apply: bool) -> Response {
    let Some(pack) = PACKS
        .iter()
        .find(|pack| pack.id == request.pack_id)
        .copied()
    else {
        return detail_err(StatusCode::NOT_FOUND, "场景模板不存在");
    };
    with_store(st, |store| {
        let transaction = apply
            .then(|| Transaction::new_unchecked(store.conn(), TransactionBehavior::Immediate))
            .transpose()?;
        let changes = match preview(store, &request, pack) {
            Ok(changes) => changes,
            Err(error) => return Ok(detail_err(StatusCode::UNPROCESSABLE_ENTITY, &error)),
        };
        let (mut response, hash) = plan(pack, &changes);
        response["preview_hash"] = json!(hash);
        if let Some(transaction) = transaction {
            if request.preview_hash.as_deref() != Some(&hash) {
                return Ok(detail_err(StatusCode::CONFLICT, "规则已变化，请重新预览"));
            }
            if changes.iter().any(|c| c.action == "conflict") {
                return Ok(detail_err(
                    StatusCode::CONFLICT,
                    "目标包含已有规则冲突，请先处理冲突",
                ));
            }
            let mut applied = 0;
            for change in changes
                .iter()
                .filter(|change| matches!(change.action, "add" | "replace"))
            {
                let Target { camera_id, zone_id } = change.target;
                if pack.after_hours {
                    store.add_rule_with_schedule(
                        camera_id,
                        zone_id,
                        "solo_after_hours",
                        2,
                        300,
                        Some("22:00"),
                        Some("06:00"),
                        300,
                        true,
                        None,
                        false,
                    )?;
                } else {
                    let mut lab = store.lab_rules(camera_id)?;
                    lab.ppe_policies.retain(|policy| policy.zone_id != zone_id);
                    lab.ppe_policies.push(PpePolicy {
                        zone_id,
                        alternatives: vec![pack.required.to_vec()],
                    });
                    store.write_lab_rules(camera_id, &lab)?;
                }
                transaction.execute(
                    "INSERT INTO scene_pack_applications(camera_id,zone_id,pack_id,pack_version,preview_hash,applied_ts) VALUES(?,?,?,?,?,?)",
                    rusqlite::params![camera_id, zone_id, pack.id, REVISION, hash, crate::util::iso_now()],
                )?;
                applied += 1;
            }
            transaction.commit()?;
            response["applied"] = json!(applied);
            if applied > 0 {
                response["policy_revision"] = json!(st.workers.notify_policy_changed());
            }
        }
        Ok(Json(response).into_response())
    })
}
