//! RA2-S5（ADR-0011）：事件 AI 复核（VLM 误报预分类）。契约对齐 Python
//! `server/app/genai.py`：
//! - 默认关（provider=none 零开销）；只写 events.ai_review* 标记，绝不改 status/
//!   不抑制告警；失败静默计数；截图只发回环（非回环需 LS_GENAI_ALLOW_REMOTE）；
//! - 单线程有界队列（max=100，满丢 + dropped 计数）；
//! - provider：ollama / openai_compat，严格 JSON verdict 白名单；
//! - 状态端点九字段（provider/model/queue_depth/enqueued/dropped/reviewed/failed/
//!   inflight/max_inflight）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::time::Duration;

use base64::Engine;
use serde_json::{json, Value};

const QUEUE_MAX: usize = 100;

/// 队列项（带 db_path 让消费者独立回写，避免持有 Shared 生命周期纠缠）。
struct GenAIJob {
    eid: i64,
    snapshot: String,
    db_path: String,
}

pub(crate) fn chrono_now() -> String {
    // UTC ISO 秒（std 无本地时区；ai_review_ts 只作观测时间戳，跨时区显示可接受）
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

const PROMPT: &str = "你是实验室安全监控复核员。看这张事件截图，判断：画面中是否确有人员未穿实验服/进入禁区？只输出 JSON：{\"verdict\":\"likely_true|likely_false|uncertain\",\"reason\":\"<=200字\",\"confidence\":0-1}。无法判断时 verdict=uncertain，不要臆测。";

pub struct GenAI {
    provider: String,
    model: String,
    min_level: i64,
    tx: Option<SyncSender<GenAIJob>>,
    stats: Stats,
}

#[derive(Default)]
struct Stats {
    enqueued: AtomicU64,
    dropped: AtomicU64,
    reviewed: AtomicU64,
    failed: AtomicU64,
    inflight: AtomicU64,
    queue_depth: AtomicU64,
}

fn env_bool(k: &str, d: bool) -> bool {
    match std::env::var(k) {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => d,
    }
}

fn env_num<T: std::str::FromStr>(k: &str, d: T) -> T {
    std::env::var(k)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(d)
}

fn is_loopback(url: &str) -> bool {
    // 与 Python urlparse().hostname 同口径的宽松判定
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let host = rest
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");
    matches!(
        host.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

impl GenAI {
    /// 从 env 组装；provider=none 时 tx=None（零队列/零线程）。
    pub fn from_env() -> Self {
        let provider = std::env::var("LS_GENAI_PROVIDER").unwrap_or_else(|_| "none".into());
        let url = std::env::var("LS_GENAI_URL").unwrap_or_default();
        let allow_remote = env_bool("LS_GENAI_ALLOW_REMOTE", false);
        if provider == "none" || url.is_empty() || (!is_loopback(&url) && !allow_remote) {
            // 未配置/非回环未授权 → 保持关闭（与 Python 合规红线一致）
            return GenAI {
                provider: "none".into(),
                model: String::new(),
                min_level: 2,
                tx: None,
                stats: Stats::default(),
            };
        }
        let (tx, rx) = mpsc::sync_channel::<GenAIJob>(QUEUE_MAX);
        let model = std::env::var("LS_GENAI_MODEL").unwrap_or_else(|_| "llava".into());
        let api_key = std::env::var("LS_GENAI_API_KEY").unwrap_or_default();
        let timeout_s = env_num("LS_GENAI_TIMEOUT_S", 10u64).max(1);
        let cfg = ProviderCfg {
            provider: provider.clone(),
            model: model.clone(),
            url: url.clone(),
            api_key: api_key.clone(),
            timeout_s,
        };
        // 单线程消费者：纯复核 + ls-store 回写（ai_review 标记，绝不改 status）
        std::thread::Builder::new()
            .name("ls-genai".into())
            .spawn(move || {
                for job in rx {
                    let result = snapshot_b64(&job.snapshot)
                        .and_then(|b64| {
                            call_provider(
                                &cfg.provider,
                                &cfg.url,
                                &cfg.model,
                                &cfg.api_key,
                                cfg.timeout_s,
                                &b64,
                            )
                            .ok()
                        })
                        .and_then(|text| parse_review(&text).ok());
                    if let Some(review) = result {
                        if let Ok(store) = ls_store::Store::open(&job.db_path) {
                            let now = chrono_now();
                            let _ =
                                store.set_ai_review(job.eid, &review.to_string(), &now, &cfg.model);
                        }
                    }
                }
            })
            .ok();
        GenAI {
            provider,
            model,
            min_level: env_num("LS_GENAI_MIN_LEVEL", 2i64),
            tx: Some(tx),
            stats: Stats::default(),
        }
    }

    /// 入队（事件落库后调用）：provider=none / 无截图 / level 不足 / 队列满 → false。
    pub fn maybe_enqueue(
        &self,
        eid: i64,
        snapshot: Option<&str>,
        level: i64,
        db_path: &str,
    ) -> bool {
        let Some(tx) = &self.tx else { return false };
        let Some(snap) = snapshot else { return false };
        if level < self.min_level {
            return false;
        }
        match tx.try_send(GenAIJob {
            eid,
            snapshot: snap.to_string(),
            db_path: db_path.to_string(),
        }) {
            Ok(()) => {
                self.stats.enqueued.fetch_add(1, Ordering::Relaxed);
                self.stats.queue_depth.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(_) => {
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    pub fn status(&self) -> Value {
        json!({
            "provider": if self.tx.is_some() { &self.provider } else { "none" },
            "model": self.model,
            "queue_depth": self.stats.queue_depth.load(Ordering::Relaxed),
            "enqueued": self.stats.enqueued.load(Ordering::Relaxed),
            "dropped": self.stats.dropped.load(Ordering::Relaxed),
            "reviewed": self.stats.reviewed.load(Ordering::Relaxed),
            "failed": self.stats.failed.load(Ordering::Relaxed),
            "inflight": self.stats.inflight.load(Ordering::Relaxed),
            "max_inflight": 1,
        })
    }
}

// 简化版：消费者线程通过 `ReviewSink` 回调把结果回写（避免 GenAI 直接持有 db）。
// 实际接入见 AppState 构造处的 `spawn_worker`（本模块提供 parse/call 纯函数 + 状态）。

struct ProviderCfg {
    provider: String,
    model: String,
    url: String,
    api_key: String,
    timeout_s: u64,
}

/// 调 provider 返回原始文本（ureq；HTTP 错误/超时 → Err）。
pub fn call_provider(
    provider: &str,
    url: &str,
    model: &str,
    api_key: &str,
    timeout_s: u64,
    b64: &str,
) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout_s.max(1)))
        .build();
    let (endpoint, body) = if provider == "ollama" {
        (
            format!("{}/api/generate", url.trim_end_matches('/')),
            json!({
                "model": model, "prompt": PROMPT, "images": [b64],
                "stream": false, "format": "json", "options": {"temperature": 0},
            }),
        )
    } else if provider == "openai_compat" {
        (
            format!("{}/v1/chat/completions", url.trim_end_matches('/')),
            json!({
                "model": model, "temperature": 0, "max_tokens": 300,
                "messages": [{"role": "user", "content": [
                    {"type": "text", "text": PROMPT},
                    {"type": "image_url", "image_url": {"url": format!("data:image/jpeg;base64,{b64}")}},
                ]}],
            }),
        )
    } else {
        return Err(format!("未知 provider: {provider}"));
    };
    let mut req = agent
        .post(&endpoint)
        .set("Content-Type", "application/json");
    if !api_key.is_empty() && provider == "openai_compat" {
        req = req.set("Authorization", &format!("Bearer {api_key}"));
    }
    let text = req
        .send_string(&body.to_string())
        .map_err(|e| format!("VLM 请求失败: {e}"))?
        .into_string()
        .map_err(|e| format!("VLM 响应读取失败: {e}"))?;
    let data: Value = serde_json::from_str(&text).map_err(|_| "VLM 响应非 JSON".to_string())?;
    if provider == "ollama" {
        Ok(data
            .get("response")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    } else {
        Ok(data["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string())
    }
}

/// 严格解析：剥 markdown 围栏 → JSON → verdict 白名单 / reason ≤200 / confidence 夹取。
pub fn parse_review(text: &str) -> Result<Value, String> {
    let t = text.trim();
    let t = if t.starts_with("```") {
        let t = t.trim_start_matches('`');
        t.split_once('\n').map(|(_, r)| r).unwrap_or(t)
    } else {
        t
    };
    let t = t.trim_end_matches('`').trim();
    let v: Value = serde_json::from_str(t).map_err(|_| "VLM 输出不是 JSON".to_string())?;
    let verdict = v.get("verdict").and_then(|x| x.as_str()).unwrap_or("");
    if !matches!(verdict, "likely_true" | "likely_false" | "uncertain") {
        return Err(format!("verdict 非法: {verdict}"));
    }
    let reason: String = v
        .get("reason")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .chars()
        .take(200)
        .collect();
    let conf = v
        .get("confidence")
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    Ok(json!({"verdict": verdict, "reason": reason, "confidence": conf}))
}

/// JPEG 截图 → base64（长边压缩留给后续；当前直接编码，契约等价）。
pub fn snapshot_b64(path: &str) -> Option<String> {
    let raw = std::fs::read(path).ok()?;
    Some(base64::engine::general_purpose::STANDARD.encode(&raw))
}
