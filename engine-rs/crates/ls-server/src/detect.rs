//! UX2（2026-11-20）：检测模式全局策略——设置中心「模型与检测」的数据面。
//!
//! 三模式语义（docs/plan/ux-roadmap-2026-11-20.md）：
//! - `local`   仅本地：ONNX 推理，零 API 成本（默认，与历史行为一致）；
//! - `api`     仅 API：判定直接走外部联网模型（现 vlm 路径）；
//! - `cascade` 本地+API 协作：本地先行扫人与可疑，触发条件的帧才送 API 精判
//!   （省钱是第一验收指标：无触发帧不产生 API 调用）。
//!
//! 数据模型（app_settings 键，密钥 fernet `enc:v1:` 先例）：
//!   detect.mode / detect.vlm.{provider,url,model,api_key,timeout_s,allow_remote}
//!   detect.cascade.{trigger,conf_lo,conf_hi,daily_budget,on_negative}
//!   detect.usage.{date,count}（当日调用计数，跨重启累计）
//!
//! 读取顺序 **DB → env → 默认**：env（LS_VLM_*）只在 DB 无值时兜底——老部署
//! 环境变量继续生效，网页端配置后以网页为准。
//!
//! 合规红线（不降级）：API URL 非回环时，DB `allow_remote` 与 env
//! `LS_VLM_ALLOW_REMOTE` **任一为真**才放行（双确认；未确认拒绝调用并落 error）。

use std::path::Path;

use crate::fernet;
use crate::state::Shared;
use crate::util::*;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};

/// 全局检测策略（load 后的快照；worker 启动时读一次，PUT 后软重启生效）。
#[derive(Debug, Clone, PartialEq)]
pub struct DetectCfg {
    pub mode: String, // local | api | cascade
    pub provider: String,
    pub url: String,
    pub model: String,
    pub api_key: String, // 明文（内存中；DB 落盘为 enc:v1:，GET 一律脱敏）
    pub timeout_s: u64,
    pub allow_remote: bool, // DB 合规确认位
    pub trigger: String,    // persons | rules | both
    pub conf_lo: f64,
    pub conf_hi: f64,
    pub daily_budget: u64,   // 0=不限
    pub on_negative: String, // keep | suppress
}

impl Default for DetectCfg {
    fn default() -> Self {
        DetectCfg {
            mode: "local".into(),
            provider: "openai_compat".into(),
            url: String::new(),
            model: "gpt-4o-mini".into(),
            api_key: String::new(),
            timeout_s: 30,
            allow_remote: false,
            trigger: "both".into(),
            conf_lo: 0.35,
            conf_hi: 0.65,
            daily_budget: 200,
            on_negative: "keep".into(),
        }
    }
}

fn db_get(s: &ls_store::Store, key: &str) -> Option<String> {
    s.setting_get(key)
        .ok()
        .flatten()
        .filter(|v| !v.trim().is_empty())
}

fn env_get(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn env_bool(key: &str) -> Option<bool> {
    env_get(key).map(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn parse_f64(v: &str) -> Option<f64> {
    v.trim().parse::<f64>().ok().filter(|x| x.is_finite())
}

impl DetectCfg {
    /// DB → env → 默认（见模块注释）。`data_dir` 供 fernet 解密 api_key。
    pub fn load(s: &ls_store::Store, data_dir: &Path) -> DetectCfg {
        let d = DetectCfg::default();
        let mode = db_get(s, "detect.mode")
            .or_else(|| env_get("LS_DETECT_MODE"))
            .unwrap_or_else(|| d.mode.clone());
        let provider = db_get(s, "detect.vlm.provider")
            .or_else(|| env_get("LS_VLM_PROVIDER"))
            .unwrap_or_else(|| d.provider.clone());
        let url = db_get(s, "detect.vlm.url")
            .or_else(|| env_get("LS_VLM_URL"))
            .map(|u| u.trim_end_matches('/').to_string())
            .unwrap_or_default();
        let model = db_get(s, "detect.vlm.model")
            .or_else(|| env_get("LS_VLM_MODEL"))
            .unwrap_or_else(|| d.model.clone());
        // api_key：DB（enc:v1: 解密）→ env → 空
        let api_key = match db_get(s, "detect.vlm.api_key") {
            Some(enc) => fernet::unprotect(&enc, data_dir),
            None => env_get("LS_VLM_API_KEY").unwrap_or_default(),
        };
        let timeout_s = db_get(s, "detect.vlm.timeout_s")
            .or_else(|| env_get("LS_VLM_TIMEOUT_S"))
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|v| v.max(1))
            .unwrap_or(d.timeout_s);
        // 合规位：DB 与 env 任一为真即放行（双确认不降级）
        let allow_remote = db_get(s, "detect.vlm.allow_remote")
            .map(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"))
            .or_else(|| env_bool("LS_VLM_ALLOW_REMOTE"))
            .unwrap_or(false);
        let trigger = db_get(s, "detect.cascade.trigger").unwrap_or_else(|| d.trigger.clone());
        let conf_lo = db_get(s, "detect.cascade.conf_lo")
            .and_then(|v| parse_f64(&v))
            .unwrap_or(d.conf_lo);
        let conf_hi = db_get(s, "detect.cascade.conf_hi")
            .and_then(|v| parse_f64(&v))
            .unwrap_or(d.conf_hi);
        let daily_budget = db_get(s, "detect.cascade.daily_budget")
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(d.daily_budget);
        let on_negative =
            db_get(s, "detect.cascade.on_negative").unwrap_or_else(|| d.on_negative.clone());
        DetectCfg {
            mode,
            provider,
            url,
            model,
            api_key,
            timeout_s,
            allow_remote,
            trigger,
            conf_lo,
            conf_hi,
            daily_budget,
            on_negative,
        }
    }

    /// 合规红线（与模式无关）：URL 非空且（回环免确认 / DB 或 env 确认位任一）。
    /// per-camera 覆盖（analyzer=vlm/cascade）在全局 mode=local 时也必须能判——
    /// worker 层用本方法而不是 vlm_enabled。
    pub fn compliance_ok(&self) -> bool {
        !self.url.is_empty()
            && (crate::vlm::is_loopback(&self.url)
                || self.allow_remote
                || env_bool("LS_VLM_ALLOW_REMOTE").unwrap_or(false))
    }

    /// 外部联网判定可用性：`mode!=local` 且合规。`mode==local` 时恒 false
    /// （零 API 成本红线）——API 层（detect/test）用；worker 层用 compliance_ok。
    pub fn vlm_enabled(&self) -> bool {
        self.mode != "local" && self.compliance_ok()
    }

    /// 组装 vlm::VlmCfg（复用 analyze_frames 既有实现）。
    pub fn to_vlm_cfg(&self) -> crate::vlm::VlmCfg {
        crate::vlm::VlmCfg {
            provider: self.provider.clone(),
            url: self.url.clone(),
            model: self.model.clone(),
            api_key: self.api_key.clone(),
            timeout_s: self.timeout_s,
            enabled: self.vlm_enabled(),
        }
    }
}

/// 相机级覆盖解析：`default`=跟随全局；`rust`/`python`（历史）=仅本地；
/// `vlm`=仅 API；`cascade`=协作。未知值回退全局。
pub fn effective_mode(analyzer: &str, global_mode: &str) -> String {
    match analyzer {
        "rust" | "python" => "local".into(),
        "vlm" => "api".into(),
        "cascade" => "cascade".into(),
        _ => global_mode.to_string(),
    }
}

/// cascade 触发判定：规则命中恒触发；人员触发看置信度灰区（< conf_lo 视为
/// 噪声不复核；≥ conf_hi 本地已确信不复核省钱；带间才交 API 精判）。
pub fn cascade_should_review(
    cfg: &DetectCfg,
    persons: i64,
    max_conf: f32,
    rules_hit: bool,
) -> bool {
    match cfg.trigger.as_str() {
        "persons" => persons_triggered(persons, max_conf, cfg),
        "rules" => rules_hit,
        _ => rules_hit || persons_triggered(persons, max_conf, cfg), // both（缺省）
    }
}

fn persons_triggered(persons: i64, max_conf: f32, cfg: &DetectCfg) -> bool {
    if persons < 1 {
        return false;
    }
    let c = max_conf as f64;
    c >= cfg.conf_lo && (cfg.conf_hi <= 0.0 || c < cfg.conf_hi)
}

// ---- 用量计数（跨重启持久化 app_settings）----

fn today_local() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let (y, m, d) = crate::util::civil_from_days(secs.div_euclid(86400));
    format!("{y:04}-{m:02}-{d:02}")
}

pub struct Usage {
    pub date: String,
    pub count: u64,
    pub budget: u64,
}

impl Usage {
    pub fn exhausted(&self) -> bool {
        self.budget > 0 && self.count >= self.budget
    }
}

/// 今日用量（detect.usage.date 与本地日期不一致 = 跨天，按 0 计）。
pub fn usage_today(s: &ls_store::Store, cfg: &DetectCfg) -> Usage {
    let today = today_local();
    let (date, count) = match (
        db_get(s, "detect.usage.date"),
        db_get(s, "detect.usage.count"),
    ) {
        (Some(d), Some(c)) if d == today => (d, c.trim().parse::<u64>().unwrap_or(0)),
        _ => (today, 0),
    };
    Usage {
        date,
        count,
        budget: cfg.daily_budget,
    }
}

/// 调用计数 +1（跨天自动归零）。返回 {是否已超预算, 计后计数}。
pub fn usage_bump(s: &ls_store::Store, cfg: &DetectCfg) -> (bool, u64) {
    let mut u = usage_today(s, cfg);
    u.count += 1;
    let _ = s.setting_set("detect.usage.date", &u.date);
    let _ = s.setting_set("detect.usage.count", &u.count.to_string());
    (u.exhausted(), u.count)
}

// ---- HTTP handlers（新增三条，不改既有契约）----

const MODES: [&str; 3] = ["local", "api", "cascade"];
const PROVIDERS: [&str; 3] = ["openai_compat", "ollama", "gemini"];
const TRIGGERS: [&str; 3] = ["persons", "rules", "both"];
const ON_NEGATIVE: [&str; 2] = ["keep", "suppress"];

/// GET /api/detect/config——api_key 一律脱敏 ***（泄漏面控制）。
pub(crate) async fn detect_config_get(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let cfg = DetectCfg::load(s, Path::new(&st.data_dir));
        let usage = usage_today(s, &cfg);
        Ok(Json(json!({
            "mode": cfg.mode,
            "vlm": {
                "provider": cfg.provider,
                "url": cfg.url,
                "model": cfg.model,
                "api_key_set": !cfg.api_key.is_empty(),
                "api_key": "***",
                "timeout_s": cfg.timeout_s,
                "allow_remote": cfg.allow_remote,
            },
            "cascade": {
                "trigger": cfg.trigger,
                "conf_lo": cfg.conf_lo,
                "conf_hi": cfg.conf_hi,
                "daily_budget": cfg.daily_budget,
                "on_negative": cfg.on_negative,
            },
            "usage": {"date": usage.date, "count": usage.count,
                      "budget": usage.budget, "exhausted": usage.exhausted()},
        }))
        .into_response())
    })
}

/// PUT /api/detect/config——增量写（缺字段/空串=不变更；api_key 仅在给出非
/// "***" 且非空时改写并加密）。生效：运行中「跟随全局」的相机软重启。
pub(crate) async fn detect_config_put(
    State(st): State<Shared>,
    Json(body): Json<Value>,
) -> Response {
    // 形校验先行（不合法整体拒绝，避免半套配置入库）
    let mode = body.get("mode").and_then(|v| v.as_str());
    if let Some(m) = mode {
        if !MODES.contains(&m) {
            return detail_err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "mode 必须是 local|api|cascade",
            )
            .into_response();
        }
    }
    let vlm = body.get("vlm");
    let provider = vlm.and_then(|v| v.get("provider")).and_then(|x| x.as_str());
    if let Some(p) = provider {
        if !PROVIDERS.contains(&p) {
            return detail_err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "provider 必须是 openai_compat|ollama|gemini",
            )
            .into_response();
        }
    }
    let cascade = body.get("cascade");
    let trig = cascade
        .and_then(|v| v.get("trigger"))
        .and_then(|x| x.as_str());
    if let Some(t) = trig {
        if !TRIGGERS.contains(&t) {
            return detail_err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "trigger 必须是 persons|rules|both",
            )
            .into_response();
        }
    }
    let on_neg = cascade
        .and_then(|v| v.get("on_negative"))
        .and_then(|x| x.as_str());
    if let Some(o) = on_neg {
        if !ON_NEGATIVE.contains(&o) {
            return detail_err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "on_negative 必须是 keep|suppress",
            )
            .into_response();
        }
    }
    let url_in = vlm
        .and_then(|v| v.get("url"))
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string());
    if let Some(u) = &url_in {
        if !u.is_empty() && !u.starts_with("http://") && !u.starts_with("https://") {
            return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "url 必须是 http(s) 地址")
                .into_response();
        }
    }

    let out = with_store(&st, |s| {
        let data_dir = Path::new(&st.data_dir);
        let mut writes: Vec<(String, String)> = Vec::new();
        let mut errs: Vec<String> = Vec::new();
        if let Some(m) = mode {
            writes.push(("detect.mode".into(), m.into()));
        }
        if let Some(v) = vlm {
            if let Some(p) = v.get("provider").and_then(|x| x.as_str()) {
                writes.push(("detect.vlm.provider".into(), p.into()));
            }
            if let Some(u) = &url_in {
                writes.push(("detect.vlm.url".into(), u.trim_end_matches('/').into()));
            }
            if let Some(m) = v.get("model").and_then(|x| x.as_str()) {
                writes.push(("detect.vlm.model".into(), m.into()));
            }
            // api_key：非空且非掩码才改写（空串=清除）；加密失败拒绝保存（拒绝明文语义）
            if let Some(k) = v.get("api_key").and_then(|x| x.as_str()) {
                if k != "***" {
                    if k.is_empty() {
                        writes.push(("detect.vlm.api_key".into(), String::new()));
                    } else {
                        match fernet::protect(k, data_dir) {
                            Some(enc) => writes.push(("detect.vlm.api_key".into(), enc)),
                            None => {
                                errs.push("api_key 加密失败（密钥存储不可用），已拒绝保存".into())
                            }
                        }
                    }
                }
            }
            if let Some(t) = v.get("timeout_s").and_then(|x| x.as_u64()) {
                writes.push(("detect.vlm.timeout_s".into(), t.max(1).to_string()));
            }
            if let Some(a) = v.get("allow_remote").and_then(|x| x.as_bool()) {
                writes.push((
                    "detect.vlm.allow_remote".into(),
                    if a { "1".into() } else { "0".into() },
                ));
            }
        }
        if let Some(c) = cascade {
            if let Some(t) = c.get("trigger").and_then(|x| x.as_str()) {
                writes.push(("detect.cascade.trigger".into(), t.into()));
            }
            if let Some(lo) = c.get("conf_lo").and_then(|x| x.as_f64()) {
                writes.push(("detect.cascade.conf_lo".into(), lo.to_string()));
            }
            if let Some(hi) = c.get("conf_hi").and_then(|x| x.as_f64()) {
                writes.push(("detect.cascade.conf_hi".into(), hi.to_string()));
            }
            if let Some(b) = c.get("daily_budget").and_then(|x| x.as_u64()) {
                writes.push(("detect.cascade.daily_budget".into(), b.to_string()));
            }
            if let Some(o) = c.get("on_negative").and_then(|x| x.as_str()) {
                writes.push(("detect.cascade.on_negative".into(), o.into()));
            }
        }
        for (k, v) in &writes {
            s.setting_set(k, v)?;
        }
        if !errs.is_empty() {
            return Ok(
                detail_err(StatusCode::INTERNAL_SERVER_ERROR, &errs.join("；")).into_response(),
            );
        }
        // 即时生效：运行中「跟随全局」的相机软重启（对齐 analyzer PUT 语义）
        let restarted = crate::worker::restart_following_global(&st, s);
        let cfg = DetectCfg::load(s, data_dir);
        let usage = usage_today(s, &cfg);
        Ok(Json(json!({
            "ok": true, "written": writes.len(), "restarted": restarted,
            "usage": {"count": usage.count, "budget": usage.budget,
                      "exhausted": usage.exhausted()},
        }))
        .into_response())
    });
    out
}

/// POST /api/detect/test——「测试连接」：用当前配置对一张测试图发一次真实判定。
/// body: {"camera_id": <可选>}；给了且 worker 在跑 → 取该路最新帧，否则内置样张。
/// 返回判定 JSON / 耗时 / 错误（不落事件）。
pub(crate) async fn detect_test(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let Some(s) = ls_store::Store::open(&st.db_path).ok() else {
        return server_err();
    };
    let cfg = DetectCfg::load(&s, Path::new(&st.data_dir));
    if cfg.url.is_empty() {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "未配置 API 地址（detect.vlm.url）",
        )
        .into_response();
    }
    if !cfg.vlm_enabled() {
        return detail_err(
            StatusCode::FORBIDDEN,
            "外部服务未授权：URL 非回环时需勾选「允许将截图发送到该外部服务」",
        )
        .into_response();
    }
    // 测试图：优先取运行中相机的最新帧，否则内置纯色样张
    let jpeg: Option<Vec<u8>> = body
        .get("camera_id")
        .and_then(|v| v.as_i64())
        .and_then(|cid| st.workers.get(cid))
        .and_then(|h| {
            let g = h.latest_jpeg.lock().ok()?;
            g.clone()
        });
    let jpeg = match jpeg {
        Some(j) if !j.is_empty() => j,
        _ => built_in_sample_jpeg(),
    };
    if jpeg.is_empty() {
        return detail_err(StatusCode::INTERNAL_SERVER_ERROR, "测试图编码失败").into_response();
    }
    // 用量计数：测试调用与真实复核同口径计入预算（仅 cascade 模式受限——
    // api 模式用户显式选择全量判定，预算只提示不阻断）
    let (exhausted, count) = usage_bump(&s, &cfg);
    let usage_json = json!({"count": count, "exhausted": exhausted});
    if exhausted && cfg.mode == "cascade" {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(
                json!({"detail": "今日 API 复核预算已用尽（detect.cascade.daily_budget）",
                        "usage": usage_json}),
            ),
        )
            .into_response();
    }
    let vlm_cfg = cfg.to_vlm_cfg();
    let started = std::time::Instant::now();
    match crate::vlm::analyze_frames(
        &vlm_cfg,
        crate::vlm::VLM_PROMPT,
        std::slice::from_ref(&jpeg),
    ) {
        Ok(v) => Json(json!({
            "ok": true,
            "elapsed_ms": started.elapsed().as_millis() as u64,
            "verdict": v,
            "usage": usage_json,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(
                json!({"ok": false, "elapsed_ms": started.elapsed().as_millis() as u64,
                        "detail": e, "usage": usage_json}),
            ),
        )
            .into_response(),
    }
}

/// GET /api/detect/usage——今日调用数/预算余量（横幅 + 仪表盘共用）。
pub(crate) async fn detect_usage(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let cfg = DetectCfg::load(s, Path::new(&st.data_dir));
        let usage = usage_today(s, &cfg);
        Ok(Json(json!({
            "mode": cfg.mode,
            "date": usage.date,
            "count": usage.count,
            "budget": usage.budget,
            "left": if usage.budget > 0 { usage.budget.saturating_sub(usage.count) } else { 0 },
            "unlimited": usage.budget == 0,
            "exhausted": usage.exhausted(),
            "degraded": usage.exhausted(),
        }))
        .into_response())
    })
}

/// 内置样张：640×360 纯色 BGR → JPEG（连通性测试；判定内容无关紧要）。
fn built_in_sample_jpeg() -> Vec<u8> {
    let w = 640usize;
    let h = 360usize;
    let mut bgr = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            // 深蓝底 + 中央浅色块（给模型一点非均匀内容）；i32 差值防 usize 下溢
            let dx = (x as i64 - (w / 2) as i64).pow(2);
            let dy = (y as i64 - (h / 2) as i64).pow(2);
            let center = dx + dy < 80 * 80;
            if center {
                bgr.extend_from_slice(&[200, 210, 220]);
            } else {
                bgr.extend_from_slice(&[80, 60, 40]);
            }
        }
    }
    ls_analyzer::evidence::encode_jpeg_bgr(&bgr, w as u32, h as u32).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed() -> (tempfile::TempDir, ls_store::Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = ls_store::Store::open(dir.path().join("t.db").to_str().unwrap()).unwrap();
        let _ = s.ensure_schema();
        let _ = s.migrate();
        (dir, s)
    }

    #[test]
    fn defaults_when_no_db_no_env() {
        let (_dir, s) = seed();
        let cfg = DetectCfg::load(&s, _dir.path());
        assert_eq!(cfg.mode, "local");
        assert!(!cfg.vlm_enabled());
        assert_eq!(cfg.daily_budget, 200);
    }

    #[test]
    fn db_overrides_env_and_defaults() {
        let (_dir, s) = seed();
        s.setting_set("detect.mode", "cascade").unwrap();
        s.setting_set("detect.cascade.daily_budget", "50").unwrap();
        let cfg = DetectCfg::load(&s, _dir.path());
        assert_eq!(cfg.mode, "cascade");
        assert_eq!(cfg.daily_budget, 50);
    }

    #[test]
    fn api_key_encrypted_roundtrip_and_mask() {
        let (dir, s) = seed();
        let enc = fernet::protect("sk-secret", dir.path()).unwrap();
        assert!(enc.starts_with("enc:v1:"));
        s.setting_set("detect.vlm.api_key", &enc).unwrap();
        s.setting_set("detect.vlm.url", "http://127.0.0.1:8899")
            .unwrap();
        s.setting_set("detect.mode", "api").unwrap();
        let cfg = DetectCfg::load(&s, dir.path());
        assert_eq!(cfg.api_key, "sk-secret");
        // 回环 URL 无需远程确认即可用
        assert!(cfg.vlm_enabled());
    }

    #[test]
    fn per_camera_override_ignores_global_mode_gate() {
        // UX2 回归：全局 mode=local + 回环 URL 时，per-camera vlm/cascade 覆盖
        // 必须能工作（compliance_ok 与全局 mode 解耦；vlm_enabled 仍受模式门控）
        let (dir, s) = seed();
        s.setting_set("detect.vlm.url", "http://127.0.0.1:8899")
            .unwrap();
        // 全局 mode 缺省 local
        let cfg = DetectCfg::load(&s, dir.path());
        assert_eq!(cfg.mode, "local");
        assert!(!cfg.vlm_enabled(), "全局 local → API 层不可用");
        assert!(cfg.compliance_ok(), "合规门独立判定（回环免确认）");
        // effective_mode 的 per-camera 覆盖语义（worker 层组合）
        assert_eq!(effective_mode("vlm", &cfg.mode), "api");
        assert_eq!(effective_mode("cascade", &cfg.mode), "cascade");
    }

    #[test]
    fn remote_url_requires_allow_remote() {
        let (dir, s) = seed();
        s.setting_set("detect.vlm.url", "https://api.example.com")
            .unwrap();
        s.setting_set("detect.mode", "api").unwrap();
        let cfg = DetectCfg::load(&s, dir.path());
        assert!(!cfg.vlm_enabled(), "远程未确认必须拒绝");
        s.setting_set("detect.vlm.allow_remote", "1").unwrap();
        let cfg = DetectCfg::load(&s, dir.path());
        assert!(cfg.vlm_enabled(), "DB 确认位放行");
        let _ = dir;
    }

    #[test]
    fn effective_mode_matrix() {
        assert_eq!(effective_mode("default", "cascade"), "cascade");
        assert_eq!(effective_mode("", "api"), "api");
        assert_eq!(effective_mode("rust", "api"), "local");
        assert_eq!(effective_mode("python", "api"), "local");
        assert_eq!(effective_mode("vlm", "local"), "api");
        assert_eq!(effective_mode("cascade", "local"), "cascade");
        assert_eq!(effective_mode("bogus", "cascade"), "cascade");
    }

    #[test]
    fn cascade_trigger_grey_band() {
        let mut cfg = DetectCfg::default(); // conf_lo=0.35 conf_hi=0.65 trigger=both
        assert!(cascade_should_review(&cfg, 1, 0.50, false), "灰区人员→复核");
        assert!(
            !cascade_should_review(&cfg, 1, 0.20, false),
            "低置信噪声→不复核"
        );
        assert!(
            !cascade_should_review(&cfg, 1, 0.90, false),
            "高置信本地已定→省钱不复核"
        );
        assert!(cascade_should_review(&cfg, 0, 0.0, true), "规则命中→恒复核");
        assert!(
            !cascade_should_review(&cfg, 0, 0.0, false),
            "无人无规则→0 API 调用"
        );
        cfg.trigger = "rules".into();
        assert!(
            !cascade_should_review(&cfg, 1, 0.50, false),
            "persons 模式忽略人员触发"
        );
        cfg.trigger = "persons".into();
        assert!(
            !cascade_should_review(&cfg, 0, 0.50, true),
            "rules 模式忽略规则触发"
        );
    }

    #[test]
    fn usage_counts_and_budget_rollover() {
        let (_dir, s) = seed();
        let cfg = DetectCfg {
            daily_budget: 3,
            ..Default::default()
        };
        let (ex, n) = usage_bump(&s, &cfg);
        assert!(!ex);
        assert_eq!(n, 1);
        let (ex, n) = usage_bump(&s, &cfg);
        assert!(!ex);
        assert_eq!(n, 2);
        let (ex, n) = usage_bump(&s, &cfg);
        assert!(ex, "count 达到 budget=3 时即视为耗尽");
        assert_eq!(n, 3);
        assert!(usage_today(&s, &cfg).exhausted());
        // 跨天归零：把 usage.date 改成昨天
        s.setting_set("detect.usage.date", "2000-01-01").unwrap();
        let u = usage_today(&s, &cfg);
        assert_eq!(u.count, 0, "跨天计数归零");
        assert!(!u.exhausted());
    }

    #[test]
    fn budget_zero_means_unlimited() {
        let (_dir, s) = seed();
        let cfg = DetectCfg {
            daily_budget: 0,
            ..Default::default()
        };
        for _ in 0..10 {
            let (ex, _) = usage_bump(&s, &cfg);
            assert!(!ex);
        }
    }
}
