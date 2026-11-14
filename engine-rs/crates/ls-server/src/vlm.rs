//! RA5（ADR-0011 增量）：外部联网 VLM 视觉分析——图片/视频帧通过 API 交外部
//! 模型判定（"AI 看图/看视频"）。设计依据（2025 调研）：支持原生视频输入的
//! API（Gemini 类）是特例；主流做法=均匀抽帧 ≤8 帧作为多图输入
//! （NVIDIA VLM 指引同款），token/帧成本克制。
//!
//! 与 genai.rs 的关系：genai 是"事件事后复核"（单截图、只写标记）；
//! vlm 是"检测面在线判定"（folder 图/视频帧 → persons/violations 真值，
//! 直接驱动事件链）。provider 协议复用同一族（openai_compat 外部联网模型
//! 通用协议 / ollama 本地）。
//!
//! 判定 JSON（宽松解析，缺字段降级跳过）：
//!   {"persons": 2, "no_ppe": 1, "violations": ["no_ppe"], "summary": "..."}
//!
//! 合规红线同 genai：URL 非回环需 LS_VLM_ALLOW_REMOTE=true（防误配外发截图）。

use base64::Engine;
use serde_json::{json, Value};

pub const VLM_PROMPT: &str = "你是实验室安全监控分析员。逐张按顺序查看这些监控画面帧，回答：\
1) 画面中最多同时出现几个人？2) 其中几人未穿实验服？3) 是否有人进入禁区（红色区域语义）？\
只输出 JSON：{\"persons\":<int>,\"no_ppe\":<int>,\"violations\":[\"no_ppe\"|\"zone_intrusion\"],\"summary\":\"<=80字\"}。\
画面模糊无法判断时 persons=0 且 violations=[]，不要臆测。";

pub struct VlmCfg {
    pub provider: String, // "openai_compat" | "ollama" | "gemini"
    pub url: String,
    pub model: String,
    pub api_key: String,
    pub timeout_s: u64,
    pub enabled: bool,
}

pub fn cfg_from_env() -> VlmCfg {
    let provider = std::env::var("LS_VLM_PROVIDER").unwrap_or_else(|_| "openai_compat".into());
    let url: String = std::env::var("LS_VLM_URL")
        .unwrap_or_default()
        .trim_end_matches('/')
        .into();
    let allow_remote = env_bool("LS_VLM_ALLOW_REMOTE", false);
    let enabled =
        env_bool("LS_VLM_ENABLED", false) && !url.is_empty() && (is_loopback(&url) || allow_remote);
    VlmCfg {
        provider,
        url,
        model: std::env::var("LS_VLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
        api_key: std::env::var("LS_VLM_API_KEY").unwrap_or_default(),
        timeout_s: env_u64("LS_VLM_TIMEOUT_S", 30).max(1),
        enabled,
    }
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

fn env_u64(k: &str, d: u64) -> u64 {
    std::env::var(k)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(d)
}

fn is_loopback(url: &str) -> bool {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let host = rest
        .split('/')
        .next()
        .unwrap_or("")
        .rsplit(':')
        .next()
        .unwrap_or("");
    matches!(
        host.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

/// 多帧判定：frames = JPEG 字节序列（按时间序）。返回判定 JSON（宽松）；
/// HTTP/解析失败 → Err（调用方降级跳过，绝不阻塞主检测链）。
pub fn analyze_frames(cfg: &VlmCfg, prompt: &str, frames: &[Vec<u8>]) -> Result<Value, String> {
    if frames.is_empty() {
        return Err("无帧可分析".into());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(cfg.timeout_s)))
        .build()
        .into();
    let imgs: Vec<String> = frames
        .iter()
        .map(|f| base64::engine::general_purpose::STANDARD.encode(f))
        .collect();
    let text = match cfg.provider.as_str() {
        // 外部联网模型通用协议：content 数组逐帧 image_url（多图=时序序列）
        "openai_compat" => {
            let mut content = vec![json!({"type": "text", "text": prompt})];
            for b64 in &imgs {
                content.push(json!({
                    "type": "image_url",
                    "image_url": {"url": format!("data:image/jpeg;base64,{b64}")}
                }));
            }
            let mut resp = agent
                .post(&format!("{}/v1/chat/completions", cfg.url))
                .header("Content-Type", "application/json")
                .send(
                    json!({
                        "model": cfg.model, "temperature": 0, "max_tokens": 400,
                        "messages": [{"role": "user", "content": content}],
                    })
                    .to_string(),
                )
                .map_err(|e| format!("VLM 请求失败: {e}"))?;
            let status = resp.status().as_u16();
            let text = resp
                .body_mut()
                .read_to_string()
                .map_err(|e| format!("响应读取失败: {e}"))?;
            if status >= 300 {
                return Err(format!("VLM HTTP {status}"));
            }
            let data: Value =
                serde_json::from_str(&text).map_err(|_| "VLM 响应非 JSON".to_string())?;
            data["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_string()
        }
        // ollama 本地：images 数组多元素
        "ollama" => {
            let mut resp = agent
                .post(&format!("{}/api/generate", cfg.url))
                .header("Content-Type", "application/json")
                .send(
                    json!({
                        "model": cfg.model, "prompt": prompt,
                        "images": imgs, "stream": false,
                        "format": "json", "options": {"temperature": 0},
                    })
                    .to_string(),
                )
                .map_err(|e| format!("VLM 请求失败: {e}"))?;
            let status = resp.status().as_u16();
            let text = resp
                .body_mut()
                .read_to_string()
                .map_err(|e| format!("响应读取失败: {e}"))?;
            if status >= 300 {
                return Err(format!("VLM HTTP {status}"));
            }
            let data: Value =
                serde_json::from_str(&text).map_err(|_| "VLM 响应非 JSON".to_string())?;
            data.get("response")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        }
        // Gemini 原生视频输入（"连续观看视频"通路）：JPEG 帧序列 → ffmpeg 临时
        // mp4 → inline_data base64 → generateContent。inline 限制 <20MB（base64
        // 膨胀 33%），超限如实报错；更大视频走 Files API（RA 后续）。
        // 请求格式（官方 video-understanding 文档）：
        //   POST {url}/models/{model}:generateContent?key=<KEY>
        //   contents[0].parts = [{text}, {inline_data:{mime_type:"video/mp4",data}}]
        "gemini" => {
            let tmp = std::env::temp_dir().join(format!("ls-vlm-{}.mp4", std::process::id()));
            ls_analyzer::evidence::encode_mp4(frames, 5.0, &tmp)
                .map_err(|e| format!("帧序列转视频失败: {e}"))?;
            let video = std::fs::read(&tmp).map_err(|e| format!("临时视频读取失败: {e}"))?;
            let _ = std::fs::remove_file(&tmp);
            if video.len() > 20_000_000 {
                return Err("视频超过 20MB（Gemini inline 限制）".into());
            }
            let b64 = base64::engine::general_purpose::STANDARD.encode(&video);
            let url = format!(
                "{}/models/{}:generateContent?key={}",
                cfg.url.trim_end_matches('/'),
                cfg.model,
                cfg.api_key
            );
            let mut resp = agent
                .post(&url)
                .header("Content-Type", "application/json")
                .send(
                    json!({
                        "contents": [{"parts": [
                            {"text": prompt},
                            {"inline_data": {"mime_type": "video/mp4", "data": b64}},
                        ]}],
                        "generationConfig": {"temperature": 0},
                    })
                    .to_string(),
                )
                .map_err(|e| format!("VLM 请求失败: {e}"))?;
            let status = resp.status().as_u16();
            let text = resp
                .body_mut()
                .read_to_string()
                .map_err(|e| format!("响应读取失败: {e}"))?;
            if status >= 300 {
                return Err(format!("VLM HTTP {status}"));
            }
            let data: Value =
                serde_json::from_str(&text).map_err(|_| "VLM 响应非 JSON".to_string())?;
            // candidates[0].content.parts[*].text 拼接（多 part 兼容）
            let out: String = data["candidates"][0]["content"]["parts"]
                .as_array()
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();
            out
        }
        p => return Err(format!("未知 provider: {p}")),
    };
    parse_verdict(&text)
}

/// 宽松判定解析：剥围栏 → JSON → persons/violations 规整（未知 violation 丢弃）。
pub fn parse_verdict(text: &str) -> Result<Value, String> {
    let t = text.trim();
    let t = if t.starts_with("```") {
        let inner = t.trim_start_matches('`');
        let inner = inner.split_once('\n').map(|(_, r)| r).unwrap_or(inner);
        inner.trim_end_matches('`').trim()
    } else {
        t
    };
    let v: Value = serde_json::from_str(t).map_err(|_| "VLM 输出不是 JSON".to_string())?;
    const ALLOWED: [&str; 2] = ["no_ppe", "zone_intrusion"];
    let violations: Vec<String> = v
        .get("violations")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .filter(|s| ALLOWED.contains(s))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    let persons = v
        .get("persons")
        .and_then(|x| x.as_i64())
        .unwrap_or(0)
        .max(0);
    let no_ppe = v
        .get("no_ppe")
        .and_then(|x| x.as_i64())
        .unwrap_or(0)
        .clamp(0, persons);
    Ok(json!({
        "persons": persons,
        "no_ppe": no_ppe,
        "violations": violations,
        "summary": v.get("summary").and_then(|x| x.as_str()).unwrap_or("").chars().take(200).collect::<String>(),
    }))
}

/// 均匀抽帧：n 帧序列 → 最多 max 帧（首尾保留，间隔均匀）。
pub fn sample_frames(frames: &[Vec<u8>], max: usize) -> Vec<Vec<u8>> {
    if frames.is_empty() || max == 0 {
        return Vec::new();
    }
    if frames.len() <= max {
        return frames.to_vec();
    }
    let last = frames.len() - 1;
    let step = last as f64 / (max - 1) as f64;
    (0..max)
        .map(|i| frames[((i as f64 * step).round() as usize).min(last)].clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_verdict_strict_loose_and_clamps() {
        let ok = parse_verdict(
            "```json\n{\"persons\":2,\"no_ppe\":1,\"violations\":[\"no_ppe\",\"bogus\"],\"summary\":\"ok\"}\n```",
        )
        .unwrap();
        assert_eq!(ok["persons"], 2);
        assert_eq!(ok["no_ppe"], 1);
        assert_eq!(
            ok["violations"],
            serde_json::json!(["no_ppe"]),
            "未知 violation 丢弃"
        );
        assert!(parse_verdict("hello").is_err());
        let c = parse_verdict("{\"persons\":-3,\"violations\":[],\"no_ppe\":9}").unwrap();
        assert_eq!(c["persons"], 0, "负数夹取");
        assert_eq!(c["no_ppe"], 0, "no_ppe 不得超 persons");
    }

    #[test]
    fn sample_frames_uniform_keeps_ends() {
        let f: Vec<Vec<u8>> = (0..10).map(|i| vec![i as u8]).collect();
        let s = sample_frames(&f, 4);
        assert_eq!(s.len(), 4);
        assert_eq!(s[0], vec![0u8], "首帧保留");
        assert_eq!(s[3], vec![9u8], "尾帧保留");
        let s2 = sample_frames(&f, 16);
        assert_eq!(s2.len(), 10, "不足 max 时全保留");
    }

    #[test]
    fn cfg_gate_requires_loopback_or_optin() {
        std::env::set_var("LS_VLM_ENABLED", "1");
        std::env::set_var("LS_VLM_URL", "https://api.example.com");
        std::env::remove_var("LS_VLM_ALLOW_REMOTE");
        assert!(!cfg_from_env().enabled, "外发截图未授权 → 关");
        std::env::set_var("LS_VLM_ALLOW_REMOTE", "true");
        assert!(cfg_from_env().enabled);
        std::env::set_var("LS_VLM_URL", "http://127.0.0.1:9999");
        assert!(cfg_from_env().enabled, "回环不需要授权");
        std::env::set_var("LS_VLM_ENABLED", "0");
        assert!(!cfg_from_env().enabled, "显式关");
        std::env::remove_var("LS_VLM_ENABLED");
        std::env::remove_var("LS_VLM_URL");
        std::env::remove_var("LS_VLM_ALLOW_REMOTE");
    }
}
