use std::collections::{BTreeMap, HashSet};
use std::sync::LazyLock;

use base64::{engine::general_purpose::STANDARD, Engine};
use ls_core::time;
use ls_store::{Camera, Event, EventsFilter, Silence, Store};
use serde_json::{json, Value};

use crate::handlers::{cameras, events, folder_images, scene_packs, stats, zones_rules};
use crate::semantic::{self, ExploreQuery, SearchError};
use crate::state::Shared;
use crate::util::sha256_hex;

use super::{ToolScope, NOT_ALLOWED};

mod review;

const MAX_SILENCE_S: i64 = 4 * 60 * 60;
const MAX_PAGE: i64 = 100;
const MAX_OFFSET: i64 = 50_000;

// Only non-biometric product events and rule kinds cross the MCP boundary.
const EVENT_TYPES: &[&str] = &[
    "enter",
    "exit",
    "person_enter",
    "person_exit",
    "enter_no_ppe",
    "no_ppe",
    "no_mask",
    "occupancy_over",
    "solo_after_hours",
    "person_fall",
    "camera_tamper",
    "camera_offline",
    "camera_recovered",
    "thermal_overheat",
    "ppe_policy",
    "hood_lean",
    "sash_limit",
    "incubator_door_open",
    "spill_or_leak",
    "glove_contamination",
    "alert_resolved",
    "photo_check",
    "video_check",
    "motion_recording",
];

const STATUSES: &[&str] = &["new", "acked", "false_positive", "ignored"];

#[derive(Clone)]
pub(super) struct Definition {
    pub name: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub write: bool,
    pub high: bool,
}

enum Tier {
    Read,
    Write,
    High,
}

impl Definition {
    fn new(
        name: &'static str,
        label: &'static str,
        description: &'static str,
        properties: Value,
        required: &[&str],
        tier: Tier,
    ) -> Self {
        Self {
            name,
            label,
            description,
            input_schema: json!({"type": "object", "additionalProperties": false,
                "properties": properties, "required": required}),
            write: !matches!(tier, Tier::Read),
            high: matches!(tier, Tier::High),
        }
    }
}

static DEFINITIONS: LazyLock<Vec<Definition>> = LazyLock::new(|| {
    use Tier::{High, Read, Write};
    let identifier = || json!({"type": "integer", "minimum": 1});
    let number = |minimum: i64, maximum: i64| json!({"type": "integer", "minimum": minimum, "maximum": maximum});
    let string = |max: usize| json!({"type": "string", "minLength": 1, "maxLength": max});
    let boolean = || json!({"type": "boolean"});
    let values = |options: &[&str]| json!({"type": "string", "enum": options});
    let camera = || json!({"camera_id": identifier()});
    let event = || json!({"event_id": identifier()});
    vec![
        Definition::new("get_system_status", "查看系统状态", "查看服务和获授权机位的运行状态。", json!({}), &[], Read),
        Definition::new("list_cameras", "列出机位", "列出获授权机位的编号、来源类型和状态。", json!({}), &[], Read),
        Definition::new("get_camera_status", "查看机位状态", "按机位编号查看运行状态、画面健康和最近帧时间。", camera(), &["camera_id"], Read),
        Definition::new("list_events", "列出事件", "分页查询获授权机位的安全事件。日期使用 YYYY-MM-DD 或本地 ISO 秒时间。", json!({
            "camera_id": identifier(), "zone_id": identifier(), "type": values(EVENT_TYPES),
            "level": number(1, 3), "status": values(STATUSES),
            "category": values(&["alert", "check", "recovery"]), "start_ts": string(19), "end_ts": string(19),
            "limit": number(1, MAX_PAGE), "offset": number(0, MAX_OFFSET),
        }), &[], Read),
        Definition::new("get_event_evidence", "查看事件证据", "查看事件的截图摘要与留存状态。提取脱敏缩略图需要额外授权。", json!({
            "event_id": identifier(), "include_thumbnail": boolean(),
        }), &["event_id"], Read),
        Definition::new("semantic_search_events", "搜索相似事件", "使用本机索引进行文字搜图或找相似事件。填写 q 或 similar_to 其中一项。", json!({
            "q": string(256), "similar_to": identifier(), "camera_id": identifier(), "limit": number(1, MAX_PAGE),
        }), &[], Read),
        Definition::new("get_compliance_stats", "查看合规统计", "按获授权的实时机位汇总已记录的合规数据。start 与 end 为本地日期。", json!({
            "camera_id": identifier(), "days": number(1, 365), "start": string(10), "end": string(10),
        }), &[], Read),
        Definition::new("list_zones_and_rules", "列出区域与规则", "查看获授权机位的区域和规则编号、边界和启停状态。", camera(), &[], Read),
        Definition::new("list_alert_silences", "列出告警静默", "查看权限范围内的静默和到期时间。active 可限定为仍生效的静默。", json!({
            "active": boolean(),
        }), &[], Read),
        Definition::new("list_folder_images", "列出已识别图片", "查看文件夹识别记录中的时间、人数和违规状态。", json!({
            "camera_id": identifier(), "key": boolean(), "violation": boolean(), "error_only": boolean(),
            "limit": number(1, MAX_PAGE), "offset": number(0, MAX_OFFSET),
        }), &[], Read),
        Definition::new("list_skill_packs", "列出场景包", "查看内置场景包的版本及其说明。", json!({}), &[], Read),
        Definition::new("trigger_folder_scan", "触发文件夹扫描", "立即扫描一个已配置的文件夹来源。", camera(), &["camera_id"], Write),
        Definition::new("trigger_event_vlm_review", "触发事件 AI 复核", "使用该机位已授权的复核档案和当前预算复核一张现存截图。", event(), &["event_id"], Write),
        Definition::new("add_event_note", "添加事件备注", "向一个获授权事件追加交接备注。", json!({
            "event_id": identifier(), "text": string(500),
        }), &["event_id", "text"], Write),
        Definition::new("triage_event", "处置单个事件", "更新一个事件的处置状态并保存原因。", json!({
            "event_id": identifier(), "status": values(STATUSES), "reason": string(500),
        }), &["event_id", "status", "reason"], Write),
        Definition::new("start_camera", "启动机位", "启动一个已配置的机位分析任务。", camera(), &["camera_id"], Write),
        Definition::new("delete_alert_silence", "撤销告警静默", "撤销一个权限范围内的告警静默。", json!({
            "silence_id": identifier(),
        }), &["silence_id"], Write),
        Definition::new("stop_camera", "停用机位分析", "停用一个机位的分析任务并记录原因。", json!({
            "camera_id": identifier(), "reason": string(500),
        }), &["camera_id", "reason"], High),
        Definition::new("create_alert_silence", "创建告警静默", "创建最多 4 小时的静默。跨机位的全局、级别和类型静默需额外授权。相机静默填写 camera_id 或目标编号字符串。", json!({
            "scope": values(&["camera", "global", "all", "level", "type"]), "target": string(64),
            "camera_id": identifier(), "duration_s": number(1, MAX_SILENCE_S), "reason": string(500),
        }), &["scope", "duration_s", "reason"], High),
        Definition::new("toggle_rule", "启停规则", "调整一个规则的启用状态并记录原因。", json!({
            "rule_id": identifier(), "enabled": boolean(), "reason": string(500),
        }), &["rule_id", "enabled", "reason"], High),
    ]
});

pub(super) fn definitions() -> Vec<Definition> {
    DEFINITIONS.clone()
}

#[derive(Debug)]
pub(super) struct Outcome {
    pub value: Value,
    pub event_id: Option<i64>,
}

impl Outcome {
    fn new(value: Value) -> Self {
        Self {
            value,
            event_id: None,
        }
    }

    fn event(value: Value, event_id: i64) -> Self {
        Self {
            value,
            event_id: Some(event_id),
        }
    }
}

#[derive(Debug)]
pub(super) struct ToolError {
    pub code: i64,
    pub message: String,
}

impl ToolError {
    fn new(code: i64, message: &str) -> Self {
        Self {
            code,
            message: message.to_owned(),
        }
    }

    fn invalid(message: &str) -> Self {
        Self::new(-32602, message)
    }

    fn denied() -> Self {
        Self::new(NOT_ALLOWED, "当前授权不允许访问指定目标或功能")
    }

    fn missing() -> Self {
        Self::new(-32004, "未找到指定记录")
    }

    fn unavailable(message: &str) -> Self {
        Self::new(-32005, message)
    }

    fn internal() -> Self {
        Self::new(-32603, "暂时无法处理此请求")
    }
}

impl From<rusqlite::Error> for ToolError {
    fn from(_: rusqlite::Error) -> Self {
        Self::internal()
    }
}

fn validate(definition: &Definition, arguments: &Value) -> Result<(), ToolError> {
    let object = arguments
        .as_object()
        .ok_or_else(|| ToolError::invalid("参数须为 JSON 对象"))?;
    let schema = &definition.input_schema;
    for name in schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !object.contains_key(name) {
            return Err(ToolError::invalid(&format!("缺少参数 {name}")));
        }
    }
    for (name, value) in object {
        let property = &schema["properties"][name];
        let valid = match property["type"].as_str() {
            Some("integer") => value.as_i64().is_some_and(|x| {
                property["minimum"].as_i64().is_none_or(|min| x >= min)
                    && property["maximum"].as_i64().is_none_or(|max| x <= max)
            }),
            Some("string") => value.as_str().is_some_and(|text| {
                let len = text.chars().count() as u64;
                property["minLength"].as_u64().is_none_or(|min| len >= min)
                    && property["maxLength"].as_u64().is_none_or(|max| len <= max)
            }),
            Some("boolean") => value.is_boolean(),
            _ => return Err(ToolError::invalid("存在未声明的参数")),
        };
        if !valid
            || property["enum"]
                .as_array()
                .is_some_and(|options| !options.contains(value))
        {
            return Err(ToolError::invalid(&format!("参数 {name} 无效")));
        }
    }
    Ok(())
}

pub(super) async fn call(
    st: &Shared,
    scope: ToolScope<'_>,
    name: &str,
    args: &Value,
) -> Result<Outcome, ToolError> {
    let definition = DEFINITIONS
        .iter()
        .find(|definition| definition.name == name)
        .ok_or_else(ToolError::denied)?;
    validate(definition, args)?;
    let st = st.clone();
    let args = args.clone();
    let cameras = scope.cameras.map(<[i64]>::to_vec);
    let actor = scope.actor.to_owned();
    let allow_image_content = scope.allow_image_content;
    let allow_global_silence = scope.allow_global_silence;
    tokio::task::spawn_blocking(move || {
        let scope = ToolScope {
            cameras: cameras.as_deref(),
            actor: &actor,
            allow_image_content,
            allow_global_silence,
        };
        let store = Store::open(&st.db_path)?;
        dispatch(&st, &store, scope, definition.name, &args)
    })
    .await
    .map_err(|_| ToolError::internal())?
}

fn camera_visible(scope: ToolScope<'_>, cid: i64) -> bool {
    scope.cameras.is_none_or(|ids| ids.contains(&cid))
}

fn check_camera_scope(scope: ToolScope<'_>, cid: i64) -> Result<(), ToolError> {
    if !camera_visible(scope, cid) {
        return Err(ToolError::denied());
    }
    Ok(())
}

fn camera(store: &Store, scope: ToolScope<'_>, cid: i64) -> Result<Camera, ToolError> {
    check_camera_scope(scope, cid)?;
    store.camera(cid)?.ok_or_else(ToolError::missing)
}

fn unavailable_target(scope: ToolScope<'_>) -> ToolError {
    if scope.cameras.is_some() {
        ToolError::denied()
    } else {
        ToolError::missing()
    }
}

fn event(store: &Store, scope: ToolScope<'_>, id: i64) -> Result<Event, ToolError> {
    store
        .event_by_id(id)?
        .filter(|row| {
            EVENT_TYPES.contains(&row.type_.as_str()) && camera_visible(scope, row.camera_id)
        })
        .ok_or_else(|| unavailable_target(scope))
}

fn note_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, ToolError> {
    args[name]
        .as_str()
        .and_then(events::event_note_text)
        .ok_or_else(|| ToolError::invalid(&format!("参数 {name} 须填写 1 至 500 字")))
}

fn dispatch(
    st: &Shared,
    store: &Store,
    scope: ToolScope<'_>,
    name: &str,
    args: &Value,
) -> Result<Outcome, ToolError> {
    let camera_id = args["camera_id"].as_i64();
    if let Some(cid) = camera_id {
        check_camera_scope(scope, cid)?;
    }
    let value = match name {
        "get_system_status" => system_status(st, store, scope)?,
        "list_cameras" => {
            let rows = store
                .cameras()?
                .iter()
                .filter(|row| camera_visible(scope, row.id))
                .map(camera_info)
                .collect::<Vec<_>>();
            json!({"cameras": rows})
        }
        "get_camera_status" => camera_status(st, store, scope, camera_id.unwrap())?,
        "list_events" => list_events(store, scope, args)?,
        "get_event_evidence" => return evidence(st, store, scope, args),
        "semantic_search_events" => semantic_search(st, store, scope, args)?,
        "get_compliance_stats" => compliance(store, scope, args)?,
        "list_zones_and_rules" => zones_and_rules(store, scope, camera_id)?,
        "list_alert_silences" => silences(store, scope, args["active"].as_bool().unwrap_or(false))?,
        "list_folder_images" => folder_records(store, scope, args)?,
        "list_skill_packs" => scene_packs::scene_packs_json(),
        "trigger_folder_scan" => {
            let cid = camera_id.unwrap();
            let started =
                folder_images::request_folder_scan(st, store, cid).map_err(
                    |error| match error {
                        folder_images::FolderScanError::Missing => ToolError::missing(),
                        folder_images::FolderScanError::WrongType => {
                            ToolError::invalid("机位来源不是图片文件夹")
                        }
                        folder_images::FolderScanError::Failed(_) => ToolError::internal(),
                    },
                )?;
            json!({"ok": true, "camera_id": cid, "started": started})
        }
        "trigger_event_vlm_review" => {
            let event = event(store, scope, args["event_id"].as_i64().unwrap())?;
            return review::review_event(st, store, &event);
        }
        "add_event_note" => {
            let event = event(store, scope, args["event_id"].as_i64().unwrap())?;
            let note = store
                .add_event_note(event.id, scope.actor, note_arg(args, "text")?)?
                .ok_or_else(ToolError::missing)?;
            return Ok(Outcome::event(
                json!({"ok": true, "event_id": event.id, "note_id": note.id}),
                event.id,
            ));
        }
        "triage_event" => {
            let event = event(store, scope, args["event_id"].as_i64().unwrap())?;
            let status = args["status"].as_str().unwrap();
            let reason = note_arg(args, "reason")?;
            let saved = events::record_event_status(
                st,
                store,
                event.id,
                status,
                scope.actor,
                Some(reason),
            )?
            .ok_or_else(ToolError::missing)?;
            return Ok(Outcome::event(
                json!({"ok": true, "event_id": event.id, "status": status,
                "handled_ts": saved["handled_ts"], "sample_saved": saved["sample_id"].is_number()}),
                event.id,
            ));
        }
        "start_camera" => {
            let cid = camera_id.unwrap();
            let started = cameras::start_camera_worker(st, store, cid)
                .map_err(|_| ToolError::internal())?
                .ok_or_else(ToolError::missing)?;
            json!({"ok": true, "camera_id": cid, "started": started})
        }
        "stop_camera" => {
            note_arg(args, "reason")?;
            let cid = camera_id.unwrap();
            let was_running =
                cameras::stop_camera_worker(st, store, cid)?.ok_or_else(ToolError::missing)?;
            json!({"ok": true, "camera_id": cid, "was_running": was_running})
        }
        "delete_alert_silence" => {
            delete_silence(store, scope, args["silence_id"].as_i64().unwrap())?
        }
        "create_alert_silence" => create_silence(store, scope, args)?,
        "toggle_rule" => toggle_rule(st, store, scope, args)?,
        _ => return Err(ToolError::denied()),
    };
    Ok(Outcome::new(value))
}

fn public_camera_status(value: &str) -> &str {
    match value {
        "starting" | "running" | "reconnecting" | "stopped" | "error" | "completed" => value,
        _ => "unknown",
    }
}

fn camera_info(camera: &Camera) -> Value {
    let kind = match camera.type_.as_str() {
        "rtsp" | "file" | "usb" | "folder" => &camera.type_,
        _ => "unknown",
    };
    let source = (kind == "rtsp").then_some("rtsp://[redacted]");
    json!({"id": camera.id, "name": format!("机位 {}", camera.id), "type": kind,
        "status": public_camera_status(&camera.status), "source": source})
}

fn system_status(st: &Shared, store: &Store, scope: ToolScope<'_>) -> Result<Value, ToolError> {
    let cameras = store.cameras()?;
    let mut counts: BTreeMap<_, usize> = BTreeMap::new();
    for camera in cameras
        .iter()
        .filter(|camera| camera_visible(scope, camera.id))
    {
        *counts
            .entry(public_camera_status(&camera.status))
            .or_default() += 1;
    }
    let total: usize = counts.values().sum();
    Ok(
        json!({"status": "ok", "version": st.version, "backend": "rust",
        "schema_version": store.schema_version()?, "camera_count": total, "cameras_by_status": counts}),
    )
}

fn camera_status(
    st: &Shared,
    store: &Store,
    scope: ToolScope<'_>,
    cid: i64,
) -> Result<Value, ToolError> {
    let row = camera(store, scope, cid)?;
    let (frame_age, last_frame, reconnects) = cameras::capture_health(st, cid);
    let mut info = camera_info(&row);
    info["last_frame_at"] = json!(last_frame);
    info["frame_age_s"] = json!(frame_age);
    info["reconnect_count"] = json!(reconnects);
    info["image_health"] = json!(st.workers.image_health(cid));
    info["offline_since"] = json!(store
        .camera_outage_since(cid)?
        .as_deref()
        .and_then(public_timestamp));
    Ok(info)
}

fn public_timestamp(value: &str) -> Option<String> {
    time::parse_local_datetime(value).map(time::format_iso)
}

fn canonical_timestamp(value: &str) -> bool {
    public_timestamp(value).is_some_and(|parsed| parsed == value)
}

fn event_filter_time(value: &str) -> bool {
    if value.len() == 10 {
        stats::valid_date_range(value, value)
    } else {
        canonical_timestamp(value)
    }
}

fn public_digest(value: &str) -> Option<&str> {
    (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(value)
}

fn review_verdict(event: &Event) -> Option<&'static str> {
    [event.ai_review.as_deref(), event.vlm_verdict.as_deref()]
        .into_iter()
        .flatten()
        .filter_map(|raw| serde_json::from_str::<Value>(raw).ok())
        .find_map(|review| match review["verdict"].as_str() {
            Some("likely_true") => Some("likely_true"),
            Some("likely_false") => Some("likely_false"),
            Some("uncertain") => Some("uncertain"),
            _ => None,
        })
}

fn event_info(event: &Event) -> Value {
    let category = match event.type_.as_str() {
        "photo_check" | "video_check" | "motion_recording" => "check",
        "alert_resolved" => "recovery",
        _ => "alert",
    };
    let status = if events::valid_event_status(&event.status) {
        &event.status
    } else {
        "unknown"
    };
    json!({"id": event.id, "camera_id": event.camera_id, "ts": public_timestamp(&event.ts),
        "type": event.type_, "category": category, "level": event.level, "status": status,
        "episode_id": event.episode_id, "duration_s": event.duration_s, "hit_count": event.hit_count,
        "has_snapshot": event.snapshot.is_some(), "has_video": event.video.is_some(),
        "ai_verdict": review_verdict(event)})
}

fn list_events(store: &Store, scope: ToolScope<'_>, args: &Value) -> Result<Value, ToolError> {
    let start = args["start_ts"].as_str();
    let end = args["end_ts"].as_str();
    if start.is_some_and(|ts| !event_filter_time(ts))
        || end.is_some_and(|ts| !event_filter_time(ts))
    {
        return Err(ToolError::invalid("事件时间须采用本地日期或 ISO 秒时间"));
    }
    if let (Some(start), Some(end)) = (start, end) {
        let comparable_end = if end.len() == 10 {
            format!("{end}T23:59:59")
        } else {
            end.to_owned()
        };
        if start > comparable_end.as_str() {
            return Err(ToolError::invalid("事件起始时间晚于结束时间"));
        }
    }
    let zone_id = args["zone_id"].as_i64();
    if let Some(zone_id) = zone_id {
        let (_, cid, _, _) = store
            .zone(zone_id)?
            .ok_or_else(|| unavailable_target(scope))?;
        check_camera_scope(scope, cid)?;
    }
    let filter = EventsFilter {
        camera_id: args["camera_id"].as_i64(),
        zone_id,
        allowed_cameras: scope.cameras,
        type_: args["type"].as_str(),
        level: args["level"].as_i64(),
        status: args["status"].as_str(),
        category: args["category"].as_str(),
        start_ts: start,
        end_ts: end,
        limit: args["limit"].as_i64().unwrap_or(50),
        offset: args["offset"].as_i64().unwrap_or(0),
        ..Default::default()
    };
    let (events, total) = store.events_query_allowed_types(&filter, EVENT_TYPES)?;
    let rows = events.iter().map(event_info).collect::<Vec<_>>();
    Ok(json!({"events": rows, "total": total, "limit": filter.limit, "offset": filter.offset}))
}

fn compliance(store: &Store, scope: ToolScope<'_>, args: &Value) -> Result<Value, ToolError> {
    let days = args["days"].as_i64().unwrap_or(1);
    let default_start = time::local_days_ago_date(days - 1);
    let today = time::local_today_iso();
    let start = args["start"].as_str().unwrap_or(&default_start);
    let end = args["end"].as_str().unwrap_or(&today);
    if !stats::valid_date_range(start, end) {
        return Err(ToolError::invalid("统计日期范围无效"));
    }
    Ok(stats::compliance_stats(
        store,
        start,
        end,
        args["camera_id"].as_i64(),
        None,
        scope.cameras,
    )?)
}

fn semantic_search(
    st: &Shared,
    store: &Store,
    scope: ToolScope<'_>,
    args: &Value,
) -> Result<Value, ToolError> {
    let selected = args["camera_id"].as_i64().map(|id| [id]);
    let cameras: Option<&[i64]> = selected
        .as_ref()
        .map(|ids| ids.as_slice())
        .or(scope.cameras);
    let query = ExploreQuery {
        q: args["q"].as_str().map(|text| text.trim().to_owned()),
        similar_to: args["similar_to"].as_i64(),
        limit: args["limit"].as_i64(),
    };
    if let Some(id) = query.similar_to {
        let anchor = event(store, scope, id)?;
        if cameras.is_some_and(|ids| !ids.contains(&anchor.camera_id)) {
            return Err(ToolError::invalid("相似检索的起始事件须属于指定机位"));
        }
    }
    match semantic::search_events(st, store, &query, cameras, Some(EVENT_TYPES)) {
        Ok(rows) => {
            let items = rows.iter().map(|(event, similarity)| {
                json!({"event_id": event.id, "event": event_info(event), "similarity": similarity})
            }).collect::<Vec<_>>();
            Ok(json!({"available": true, "items": items}))
        }
        Err(SearchError::Disabled) => {
            Ok(json!({"available": false, "items": [], "status": "disabled"}))
        }
        Err(SearchError::InvalidQuery | SearchError::Encoding) => Err(ToolError::invalid(
            "请填写 1 至 256 个字符或提供一个起始事件",
        )),
        Err(SearchError::MissingEvent) => Err(unavailable_target(scope)),
        Err(SearchError::Unindexed) => Err(ToolError::unavailable("该事件截图尚未编入索引")),
        Err(SearchError::Busy) => Err(ToolError::unavailable("离线搜图繁忙")),
        Err(SearchError::ModelUnavailable) => Err(ToolError::unavailable("离线搜图模型不可用")),
        Err(SearchError::Database) => Err(ToolError::internal()),
    }
}

fn clock_time(value: &str) -> Option<&str> {
    let (hour, minute) = value.split_once(':')?;
    (hour.len() == 2
        && minute.len() == 2
        && hour.parse::<u8>().is_ok_and(|value| value < 24)
        && minute.parse::<u8>().is_ok_and(|value| value < 60))
    .then_some(value)
}

fn zones_and_rules(
    store: &Store,
    scope: ToolScope<'_>,
    cid: Option<i64>,
) -> Result<Value, ToolError> {
    let zones = store.zones_spatial(cid)?;
    let mut zone_keys = HashSet::new();
    let zones = zones
        .iter()
        .filter(|zone| camera_visible(scope, zone.camera_id))
        .map(|zone| {
            zone_keys.insert((zone.id, zone.camera_id));
            let polygon = serde_json::from_str::<Value>(&zone.polygon)
                .ok()
                .filter(zones_rules::valid_zone_polygon);
            json!({"id": zone.id, "camera_id": zone.camera_id, "name": format!("区域 {}", zone.id),
            "polygon": polygon, "spatial": zone.spatial})
        })
        .collect::<Vec<_>>();
    let rules = store.rules_all()?.into_iter()
        .filter(|row| camera_visible(scope, row.1) && cid.is_none_or(|id| id == row.1))
        .filter(|row| EVENT_TYPES.contains(&row.3.as_str()))
        .map(|row| {
            let zone_id = row.2.filter(|zone| zone_keys.contains(&(*zone, row.1)));
            let weekdays = row.10.as_deref().unwrap_or_default().split(',')
                .filter_map(|value| value.trim().parse::<u8>().ok())
                .filter(|value| (1..=7).contains(value)).collect::<Vec<_>>();
            json!({"id": row.0, "camera_id": row.1, "zone_id": zone_id, "behavior": row.3,
                "level": row.4, "cooldown_s": row.5, "time_start": row.6.as_deref().and_then(clock_time),
                "time_end": row.7.as_deref().and_then(clock_time), "threshold": row.8, "enabled": row.9 != 0,
                "days_of_week": weekdays, "holiday_only": row.11, "grace_s": row.12, "warn_ratio": row.13})
        }).collect::<Vec<_>>();
    Ok(json!({"zones": zones, "rules": rules}))
}

fn folder_records(store: &Store, scope: ToolScope<'_>, args: &Value) -> Result<Value, ToolError> {
    let cid = args["camera_id"].as_i64();
    let limit = args["limit"].as_i64().unwrap_or(50);
    let offset = args["offset"].as_i64().unwrap_or(0);
    let camera_scope = scope.cameras.map(|ids| json!(ids).to_string());
    let mut statement = store.conn().prepare(
        "SELECT f.id,f.camera_id,f.ts,f.processed_ts,f.width,f.height,COALESCE(f.persons,0),
                COALESCE(f.in_zone,0),COALESCE(f.ppe_no,0),COALESCE(f.ppe_wearing,0),
                COALESCE(f.key,0)!=0,COALESCE(f.violation,0)!=0,f.error IS NOT NULL
         FROM folder_images f JOIN cameras c ON c.id=f.camera_id
         WHERE c.type='folder' AND (?1 IS NULL OR f.camera_id IN (SELECT value FROM json_each(?1)))
           AND (?2 IS NULL OR f.camera_id=?2) AND (?3 IS NULL OR COALESCE(f.key,0)=?3)
           AND (?4 IS NULL OR COALESCE(f.violation,0)=?4) AND (NOT ?5 OR f.error IS NOT NULL)
         ORDER BY f.id DESC LIMIT ?6 OFFSET ?7",
    )?;
    let rows = statement.query_map(rusqlite::params![camera_scope, cid, args["key"].as_bool(),
        args["violation"].as_bool(), args["error_only"].as_bool().unwrap_or(false), limit, offset], |row| {
        Ok(json!({"id": row.get::<_, i64>(0)?, "camera_id": row.get::<_, i64>(1)?,
            "ts": row.get::<_, Option<String>>(2)?.as_deref().and_then(public_timestamp),
            "processed_ts": row.get::<_, Option<String>>(3)?.as_deref().and_then(public_timestamp),
            "width": row.get::<_, Option<i64>>(4)?, "height": row.get::<_, Option<i64>>(5)?,
            "persons": row.get::<_, i64>(6)?, "in_zone": row.get::<_, i64>(7)?,
            "ppe_no": row.get::<_, i64>(8)?, "ppe_wearing": row.get::<_, i64>(9)?,
            "key": row.get::<_, bool>(10)?, "violation": row.get::<_, bool>(11)?, "has_error": row.get::<_, bool>(12)?}))
    })?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({"items": rows, "limit": limit, "offset": offset}))
}

fn snapshot_bytes(st: &Shared, store: &Store, event: &Event) -> Option<(Vec<u8>, String)> {
    let file = events::event_image_path(st, store, event)?;
    let bytes = ls_jobs::media::read_snapshot(&file)?;
    let digest = sha256_hex(&bytes);
    if event
        .snapshot_sha256
        .as_deref()
        .is_some_and(|expected| !digest.eq_ignore_ascii_case(expected))
    {
        return None;
    }
    Some((bytes, digest))
}

fn anonymized_thumbnail(bytes: &[u8]) -> Option<Value> {
    use image::{imageops::FilterType, DynamicImage};

    // Average the whole frame down to a coarse grayscale grid. This bound also
    // applies to undetected faces, screen text, and images without box metadata.
    let mut blocks = ls_jobs::media::decode_image(bytes)?
        .thumbnail(8, 6)
        .into_luma8();
    for pixel in blocks.pixels_mut() {
        pixel[0] = (pixel[0] / 32) * 32;
    }
    let thumbnail = DynamicImage::ImageLuma8(blocks);
    let width = thumbnail.width() * 24;
    let height = thumbnail.height() * 24;
    let thumbnail = thumbnail.resize_exact(width, height, FilterType::Nearest);
    let jpeg = ls_jobs::media::encode_jpeg(&thumbnail, 82)?;
    Some(
        json!({"mimeType": "image/jpeg", "data": STANDARD.encode(jpeg), "width": width, "height": height}),
    )
}

fn evidence(
    st: &Shared,
    store: &Store,
    scope: ToolScope<'_>,
    args: &Value,
) -> Result<Outcome, ToolError> {
    let event = event(store, scope, args["event_id"].as_i64().unwrap())?;
    let thumbnail_requested = args["include_thumbnail"].as_bool().unwrap_or(false);
    if thumbnail_requested && !scope.allow_image_content {
        return Err(ToolError::denied());
    }
    let mut result = json!({"event_id": event.id, "event": event_info(&event),
        "snapshot_sha256": event.snapshot_sha256.as_deref().and_then(public_digest),
        "has_snapshot": events::event_image_path(st, store, &event).is_some(),
        "has_video": event.video.is_some(), "has_overlay": event.detections_json.is_some()});
    if thumbnail_requested {
        let thumbnail = snapshot_bytes(st, store, &event).and_then(|(bytes, digest)| {
            let thumbnail = anonymized_thumbnail(&bytes)?;
            result["snapshot_sha256"] = json!(digest);
            Some(thumbnail)
        });
        result["thumbnail"] = json!(thumbnail);
    }
    Ok(Outcome::event(result, event.id))
}

#[derive(Clone, Copy)]
enum SilenceTarget {
    Camera(i64),
    Global,
    Level(i64),
    Type(&'static str),
}

impl SilenceTarget {
    fn parse(scope: &str, target: Option<&str>) -> Option<Self> {
        match (scope, target) {
            ("global" | "all", _) => Some(Self::Global),
            ("camera", Some(target)) => {
                let id = target.parse::<i64>().ok()?;
                (id > 0 && target == id.to_string()).then_some(Self::Camera(id))
            }
            ("level", Some("1")) => Some(Self::Level(1)),
            ("level", Some("2")) => Some(Self::Level(2)),
            ("level", Some("3")) => Some(Self::Level(3)),
            ("type", Some(target)) => EVENT_TYPES
                .iter()
                .copied()
                .find(|kind| *kind == target)
                .map(Self::Type),
            _ => None,
        }
    }

    fn visible(self, scope: ToolScope<'_>) -> bool {
        match self {
            Self::Camera(id) => camera_visible(scope, id),
            Self::Global | Self::Level(_) | Self::Type(_) => scope.cameras.is_none(),
        }
    }

    fn fields(self) -> (&'static str, Option<String>) {
        match self {
            Self::Camera(id) => ("camera", Some(id.to_string())),
            Self::Global => ("global", None),
            Self::Level(level) => ("level", Some(level.to_string())),
            Self::Type(kind) => ("type", Some(kind.to_owned())),
        }
    }
}

fn visible_silence(row: &Silence, scope: ToolScope<'_>) -> Option<SilenceTarget> {
    SilenceTarget::parse(&row.scope, row.target.as_deref()).filter(|target| target.visible(scope))
}

fn silences(store: &Store, scope: ToolScope<'_>, active_only: bool) -> Result<Value, ToolError> {
    let now = time::local_now_iso();
    let items = store.silences()?.iter().filter_map(|row| {
        let silence = visible_silence(row, scope)?;
        let active = row.until_ts > now;
        if active_only && !active {
            return None;
        }
        let (scope, target) = silence.fields();
        Some(json!({"id": row.id, "scope": scope, "target": target, "active": active,
            "created_ts": public_timestamp(&row.created_ts), "until_ts": public_timestamp(&row.until_ts)}))
    }).collect::<Vec<_>>();
    Ok(json!({"items": items}))
}

fn create_silence(store: &Store, scope: ToolScope<'_>, args: &Value) -> Result<Value, ToolError> {
    let reason = note_arg(args, "reason")?;
    let kind = args["scope"].as_str().unwrap();
    let camera_id = args["camera_id"].as_i64();
    let raw_target = args["target"].as_str();
    let target = match (kind, camera_id, raw_target) {
        ("camera", Some(id), None) => Some(SilenceTarget::Camera(id)),
        ("global" | "all", None, None) => Some(SilenceTarget::Global),
        ("camera" | "level" | "type", None, Some(target)) => {
            SilenceTarget::parse(kind, Some(target))
        }
        _ => None,
    }
    .ok_or_else(|| ToolError::invalid("请为所选静默范围提供有效的目标"))?;
    if !target.visible(scope)
        || (!matches!(target, SilenceTarget::Camera(_)) && !scope.allow_global_silence)
    {
        return Err(ToolError::denied());
    }
    if let SilenceTarget::Camera(cid) = target {
        camera(store, scope, cid)?;
    }
    let (kind, target) = target.fields();
    let seconds = args["duration_s"].as_i64().unwrap();
    let until = time::format_iso(time::local_offset_secs(seconds));
    let id = store.add_silence(kind, target.as_deref(), Some(reason), &until)?;
    Ok(json!({"id": id, "scope": kind, "target": target, "until_ts": until}))
}

fn delete_silence(store: &Store, scope: ToolScope<'_>, id: i64) -> Result<Value, ToolError> {
    let allowed = store
        .silences()?
        .iter()
        .find(|row| row.id == id)
        .and_then(|row| visible_silence(row, scope))
        .is_some();
    if !allowed {
        return Err(unavailable_target(scope));
    }
    if !store.delete_silence(id)? {
        return Err(ToolError::missing());
    }
    Ok(json!({"ok": true, "silence_id": id}))
}

fn toggle_rule(
    st: &Shared,
    store: &Store,
    scope: ToolScope<'_>,
    args: &Value,
) -> Result<Value, ToolError> {
    note_arg(args, "reason")?;
    let id = args["rule_id"].as_i64().unwrap();
    let enabled = args["enabled"].as_bool().unwrap();
    let (cid, _) = zones_rules::rule_target(store, id)?
        .filter(|(cid, behavior)| {
            camera_visible(scope, *cid) && EVENT_TYPES.contains(&behavior.as_str())
        })
        .ok_or_else(|| unavailable_target(scope))?;
    let revision = zones_rules::set_rule_enabled(st, store, cid, id, enabled)?
        .ok_or_else(ToolError::missing)?;
    Ok(
        json!({"ok": true, "rule_id": id, "camera_id": cid, "enabled": enabled, "policy_revision": revision}),
    )
}
