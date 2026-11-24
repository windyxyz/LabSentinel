//! Legacy opt-in event review queue. All model requests use the shared VLM transport.

use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};

const QUEUE_MAX: usize = 100;

/// 队列项（带 db_path 让消费者独立回写，避免持有 Shared 生命周期纠缠）。
struct GenAIJob {
    eid: i64,
    snapshot: String,
    db_path: String,
}

pub(crate) fn chrono_now() -> String {
    ls_core::time::local_now_iso()
}

const PROMPT: &str = "你是实验室安全监控复核员。看这张事件截图，判断：画面中是否确有人员未穿实验服/进入禁区？只输出 JSON：{\"verdict\":\"likely_true|likely_false|uncertain\",\"reason\":\"<=200字\",\"confidence\":0-1}。无法判断时 verdict=uncertain，不要臆测。";

pub struct GenAI {
    provider: String,
    model: String,
    min_level: i64,
    tx: Option<SyncSender<GenAIJob>>,
    stats: std::sync::Arc<Stats>,
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

impl GenAI {
    /// 从 env 组装；provider=none 时 tx=None（零队列/零线程）。
    pub fn from_env() -> Self {
        let provider = std::env::var("LS_GENAI_PROVIDER").unwrap_or_else(|_| "none".into());
        let url = std::env::var("LS_GENAI_URL").unwrap_or_default();
        let allow_remote = env_bool("LS_GENAI_ALLOW_REMOTE", false);
        if !matches!(provider.as_str(), "openai_compat" | "ollama")
            || url.is_empty()
            || (!crate::vlm::is_loopback(&url) && !allow_remote)
        {
            // 未配置/非回环未授权 → 保持关闭（与 Python 合规红线一致）
            return GenAI {
                provider: "none".into(),
                model: String::new(),
                min_level: 2,
                tx: None,
                stats: std::sync::Arc::new(Stats::default()),
            };
        }
        let (tx, rx) = mpsc::sync_channel::<GenAIJob>(QUEUE_MAX);
        let model = std::env::var("LS_GENAI_MODEL").unwrap_or_else(|_| "llava".into());
        let api_key = std::env::var("LS_GENAI_API_KEY").unwrap_or_default();
        let timeout_s = env_num("LS_GENAI_TIMEOUT_S", 10u64).max(1);
        let cfg = crate::vlm::VlmCfg {
            provider: provider.clone(),
            model: model.clone(),
            url: url.clone(),
            api_key: api_key.clone(),
            timeout_s,
            enabled: true,
        };
        let stats = std::sync::Arc::new(Stats::default());
        let rx_stats = std::sync::Arc::clone(&stats);
        // 单线程消费者：纯复核 + ls-store 回写（ai_review 标记，绝不改 status）。
        // RA2 反思修正：reviewed/failed/queue_depth 由消费者维护——此前 stats
        // 未共享给消费线程，status 端点的计数恒 0（queue_depth 只增不减）
        std::thread::Builder::new()
            .name("ls-genai".into())
            .spawn(move || {
                for job in rx {
                    rx_stats.queue_depth.fetch_sub(1, Ordering::Relaxed);
                    rx_stats.inflight.fetch_add(1, Ordering::Relaxed);
                    let result = ls_jobs::media::snapshot_jpeg(std::path::Path::new(&job.snapshot))
                        .and_then(|jpeg| crate::vlm::visual_response(&cfg, PROMPT, &[jpeg]).ok())
                        .and_then(|text| parse_review(&text).ok());
                    match result {
                        Some(review) => {
                            rx_stats.reviewed.fetch_add(1, Ordering::Relaxed);
                            if let Ok(store) = ls_store::Store::open(&job.db_path) {
                                let now = chrono_now();
                                let _ = store.set_ai_review(
                                    job.eid,
                                    &review.to_string(),
                                    &now,
                                    &cfg.model,
                                );
                            }
                        }
                        None => {
                            rx_stats.failed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    rx_stats.inflight.fetch_sub(1, Ordering::Relaxed);
                }
            })
            .ok();
        GenAI {
            provider,
            model,
            min_level: env_num("LS_GENAI_MIN_LEVEL", 2i64),
            tx: Some(tx),
            stats,
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
        self.stats.queue_depth.fetch_add(1, Ordering::Relaxed);
        match tx.try_send(GenAIJob {
            eid,
            snapshot: snap.to_string(),
            db_path: db_path.to_string(),
        }) {
            Ok(()) => {
                self.stats.enqueued.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(_) => {
                self.stats.queue_depth.fetch_sub(1, Ordering::Relaxed);
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
