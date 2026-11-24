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

// UX2（2026-11-20）：env 组装（原 cfg_from_env）迁入 detect.rs 的
// DetectCfg::load（DB → env → 默认统一链；VlmCfg 由 DetectCfg::to_vlm_cfg 组装）。

/// URL 是否回环（UX2 合规判定复用：detect.rs 远程放行检查）。
/// LS-041/LS-044 安全加固：拒绝 authority 中包含 `@`（防 `http://127.0.0.1:80@evil.com` 绕过）。
pub(crate) fn is_loopback(url: &str) -> bool {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let hostport = rest.split('/').next().unwrap_or("");
    if hostport.contains('@') {
        return false;
    }
    let host = if let Some(rest6) = hostport.strip_prefix('[') {
        rest6.split(']').next().unwrap_or("")
    } else {
        hostport.split(':').next().unwrap_or("")
    };
    let h = host.to_ascii_lowercase();
    matches!(h.as_str(), "127.0.0.1" | "localhost" | "::1") || h.starts_with("127.")
}

fn redact_secret(msg: String, secret: &str) -> String {
    let s = secret.trim();
    if s.is_empty() {
        msg
    } else {
        msg.replace(s, "***")
    }
}

fn openai_endpoint(base: &str, suffix: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if b.ends_with("/v1") {
        format!("{b}/{suffix}")
    } else {
        format!("{b}/v1/{suffix}")
    }
}

fn gemini_endpoint(base: &str, suffix: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if b.contains("/v1beta") || b.contains("/v1") {
        format!("{b}/{suffix}")
    } else {
        format!("{b}/v1beta/{suffix}")
    }
}

struct TempFileGuard(std::path::PathBuf);
impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

static TEMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// 多帧判定：frames = JPEG 字节序列（按时间序）。返回判定 JSON（宽松）；
/// HTTP/解析失败 → Err（调用方降级跳过，绝不阻塞主检测链）。
pub fn analyze_frames(cfg: &VlmCfg, prompt: &str, frames: &[Vec<u8>]) -> Result<Value, String> {
    if frames.is_empty() {
        return Err("无帧可分析".into());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(cfg.timeout_s)))
        .http_status_as_error(false)
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
            let ep = openai_endpoint(&cfg.url, "chat/completions");
            let mut req = agent.post(&ep).header("Content-Type", "application/json");
            if !cfg.api_key.trim().is_empty() {
                req = req.header("Authorization", &format!("Bearer {}", cfg.api_key.trim()));
            }
            let mut resp = req
                .send(
                    json!({
                        "model": cfg.model, "temperature": 0, "max_tokens": 400,
                        "messages": [{"role": "user", "content": content}],
                    })
                    .to_string(),
                )
                .map_err(|e| redact_secret(format!("VLM 请求失败: {e}"), &cfg.api_key))?;
            let status = resp.status().as_u16();
            let text = resp
                .body_mut()
                .read_to_string()
                .map_err(|e| format!("响应读取失败: {e}"))?;
            if status >= 300 {
                let brief: String = text.chars().take(240).collect();
                return Err(redact_secret(
                    format!("VLM HTTP {status}: {brief}"),
                    &cfg.api_key,
                ));
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
                .post(&format!("{}/api/generate", cfg.url.trim_end_matches('/')))
                .header("Content-Type", "application/json")
                .send(
                    json!({
                        "model": cfg.model, "prompt": prompt,
                        "images": imgs, "stream": false,
                        "format": "json", "options": {"temperature": 0},
                    })
                    .to_string(),
                )
                .map_err(|e| redact_secret(format!("VLM 请求失败: {e}"), &cfg.api_key))?;
            let status = resp.status().as_u16();
            let text = resp
                .body_mut()
                .read_to_string()
                .map_err(|e| format!("响应读取失败: {e}"))?;
            if status >= 300 {
                let brief: String = text.chars().take(240).collect();
                return Err(redact_secret(
                    format!("VLM HTTP {status}: {brief}"),
                    &cfg.api_key,
                ));
            }
            let data: Value =
                serde_json::from_str(&text).map_err(|_| "VLM 响应非 JSON".to_string())?;
            data.get("response")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        }
        // Gemini 原生视频输入（"连续观看视频"通路）：JPEG 帧序列 → ffmpeg 临时
        // mp4 → inline_data base64 → generateContent。使用官方 `x-goog-api-key` 头，
        // 严禁将密钥拼入 URL query 防止日志/报错明文泄漏。
        "gemini" => {
            let seq = TEMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let tmp_guard = TempFileGuard(
                std::env::temp_dir().join(format!("ls-vlm-{}-{seq}.mp4", std::process::id())),
            );
            ls_analyzer::evidence::encode_mp4(frames, 5.0, &tmp_guard.0)
                .map_err(|e| format!("帧序列转视频失败: {e}"))?;
            let video =
                std::fs::read(&tmp_guard.0).map_err(|e| format!("临时视频读取失败: {e}"))?;
            drop(tmp_guard);
            if video.len() > 20_000_000 {
                return Err("视频超过 20MB（Gemini inline 限制）".into());
            }
            let b64 = base64::engine::general_purpose::STANDARD.encode(&video);
            let url = gemini_endpoint(&cfg.url, &format!("models/{}:generateContent", cfg.model));
            let mut req = agent.post(&url).header("Content-Type", "application/json");
            if !cfg.api_key.trim().is_empty() {
                req = req.header("x-goog-api-key", cfg.api_key.trim());
            }
            let mut resp = req
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
                .map_err(|e| redact_secret(format!("VLM 请求失败: {e}"), &cfg.api_key))?;
            let status = resp.status().as_u16();
            let text = resp
                .body_mut()
                .read_to_string()
                .map_err(|e| format!("响应读取失败: {e}"))?;
            if status >= 300 {
                let brief: String = text.chars().take(240).collect();
                return Err(redact_secret(
                    format!("VLM HTTP {status}: {brief}"),
                    &cfg.api_key,
                ));
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

/// 上游模型清单（0.4.0：网页端「自动获取模型」）——仅 GET 元数据端点，
/// 不发送监控画面；openai_compat / ollama / gemini 三协议。
pub fn list_models(
    provider: &str,
    url: &str,
    api_key: &str,
    timeout_s: u64,
) -> Result<Vec<String>, String> {
    let base = url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("API 地址为空".into());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(
            timeout_s.clamp(3, 120),
        )))
        .http_status_as_error(false)
        .build()
        .into();
    let (endpoint, auth_mode) = match provider {
        "openai_compat" => (openai_endpoint(base, "models"), "bearer"),
        "ollama" => (format!("{base}/api/tags"), "none"),
        "gemini" => (gemini_endpoint(base, "models"), "gemini"),
        p => return Err(format!("未知 provider: {p}")),
    };
    let mut req = agent.get(&endpoint);
    if auth_mode == "bearer" && !api_key.trim().is_empty() {
        req = req.header("Authorization", &format!("Bearer {}", api_key.trim()));
    } else if auth_mode == "gemini" && !api_key.trim().is_empty() {
        req = req.header("x-goog-api-key", api_key.trim());
    }
    let mut resp = req
        .call()
        .map_err(|e| redact_secret(format!("连接失败: {e}"), api_key))?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("响应读取失败: {e}"))?;
    if status >= 300 {
        // 失败时回显截断的响应体，便于识别 401/404 与错误原因（脱敏密钥）
        let brief: String = text.chars().take(240).collect();
        return Err(redact_secret(format!("HTTP {status}: {brief}"), api_key));
    }
    let data: Value = serde_json::from_str(&text).map_err(|_| "响应非 JSON".to_string())?;
    let mut out: Vec<String> = match provider {
        "openai_compat" => data["data"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("id").and_then(|v| v.as_str()))
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default(),
        "ollama" => data["models"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("name").and_then(|v| v.as_str()))
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default(),
        _ => data["models"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| {
                        let name = m.get("name").and_then(|v| v.as_str())?;
                        // Gemini 同时列出 embedding 等；只保留可 generateContent 的
                        let ok = m
                            .get("supportedGenerationMethods")
                            .and_then(|v| v.as_array())
                            .map(|ms| ms.iter().any(|x| x.as_str() == Some("generateContent")))
                            .unwrap_or(true);
                        ok.then(|| name.trim_start_matches("models/").to_string())
                    })
                    .collect()
            })
            .unwrap_or_default(),
    };
    out.sort();
    out.dedup();
    if out.is_empty() {
        return Err("服务可连通，但未返回任何模型（检查地址/密钥/账号权限）".into());
    }
    Ok(out)
}

/// 宽松判定解析：剥围栏 → JSON（失败则回退提取首尾 `{...}`）→ persons/violations 规整。
pub fn parse_verdict(text: &str) -> Result<Value, String> {
    let t = text.trim();
    let t = if t.starts_with("```") {
        let inner = t.trim_start_matches('`');
        let inner = inner.split_once('\n').map(|(_, r)| r).unwrap_or(inner);
        inner.trim_end_matches('`').trim()
    } else {
        t
    };
    let v: Value = serde_json::from_str(t)
        .or_else(|_| {
            let start = t.find('{').ok_or(())?;
            let end = t.rfind('}').ok_or(())?;
            if end > start {
                serde_json::from_str(&t[start..=end]).map_err(|_| ())
            } else {
                Err(())
            }
        })
        .map_err(|_| "VLM 输出不是 JSON".to_string())?;
    let labels = |field: &str, allowed: &[&str]| -> Vec<String> {
        v[field]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|s| allowed.contains(s))
            .map(str::to_owned)
            .collect()
    };
    let violations = labels("violations", &["no_ppe", "no_mask", "zone_intrusion"]);
    let compliant = labels("compliant", &["no_ppe", "no_mask"]);
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
        "compliant": compliant,
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
    fn is_loopback_classification() {
        // UX2 合规门依赖：回环免确认，其余（含内网/公网）需显式放行
        assert!(is_loopback("http://127.0.0.1:8899/v1"));
        assert!(is_loopback("http://localhost:11434"));
        assert!(!is_loopback("https://api.example.com/v1"));
        assert!(!is_loopback("http://192.168.1.10:8000"));
    }
}
