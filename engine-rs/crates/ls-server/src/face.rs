//! RA2-S6（ADR-0011）：人脸库控制面（CompreFace REST）。契约对齐 Python
//! `server/app/face.py` + `routers/ops.py`：
//! - 默认关（LS_FACE_ENABLED），未配 key（LS_COMPREFACE_API_KEY）→ 503；
//! - 不落人脸图片（只向 CompreFace 发帧，本地只存 subject/同意时间戳）；
//! - 注册 consent=true 强制 + 10MB 上限；失败 502；
//! - 1:N 识别数据面豁免（在 Python/sidecar），控制面只做注册/检索/删除。

use std::time::Duration;

use serde_json::Value;

pub struct FaceClient {
    pub base_url: String,
    pub api_key: String,
    pub threshold: f64,
    pub timeout_s: u64,
}

impl FaceClient {
    pub fn from_env() -> Self {
        FaceClient {
            base_url: std::env::var("LS_COMPREFACE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8000".into())
                .trim_end_matches('/')
                .into(),
            api_key: std::env::var("LS_COMPREFACE_API_KEY").unwrap_or_default(),
            threshold: std::env::var("LS_FACE_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.75),
            timeout_s: std::env::var("LS_FACE_TIMEOUT_S")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),
        }
    }

    pub fn configured(&self) -> bool {
        !self.api_key.is_empty()
    }

    fn agent(&self) -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(self.timeout_s.max(1))))
            .http_status_as_error(false)
            .build()
            .into()
    }

    /// multipart/form-data 手工拼装（ureq 无原生 multipart；仅一个 file 字段）。
    fn multipart_file(field: &str, filename: &str, data: &[u8]) -> (String, Vec<u8>) {
        let boundary = "----LabSentinelFaceBoundary";
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(b"Content-Type: image/jpeg\r\n\r\n");
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        (boundary.to_string(), body)
    }

    pub fn add_person(&self, subject: &str, photo: &[u8]) -> Result<(), String> {
        let (boundary, body) = Self::multipart_file("file", "photo.jpg", photo);
        let url = format!(
            "{}/recognition/faces?subject={}",
            self.base_url,
            urlenc(subject)
        );
        let resp = self
            .agent()
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header(
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .send(&body[..])
            .map_err(|e| format!("CompreFace 注册失败: {e}"))?;
        // 显式校验状态码（http_status_as_error(false) 时 4xx/5xx 仍返回 Ok）
        if resp.status().as_u16() >= 300 {
            return Err(format!("CompreFace 注册 HTTP {}", resp.status().as_u16()));
        }
        Ok(())
    }

    pub fn list_persons(&self) -> Result<Vec<String>, String> {
        let url = format!("{}/recognition/subjects", self.base_url);
        let mut resp = self
            .agent()
            .get(&url)
            .header("x-api-key", &self.api_key)
            .call()
            .map_err(|e| format!("CompreFace 列表失败: {e}"))?;
        if resp.status().as_u16() >= 300 {
            return Err(format!("CompreFace 列表 HTTP {}", resp.status().as_u16()));
        }
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| format!("CompreFace 响应读取失败: {e}"))?;
        let v: Value =
            serde_json::from_str(&text).map_err(|_| "CompreFace 列表非 JSON".to_string())?;
        let subs = v
            .get("subjects")
            .and_then(|x| x.as_array())
            .cloned()
            .or_else(|| v.as_array().cloned())
            .unwrap_or_default();
        Ok(subs
            .iter()
            .filter_map(|s| {
                s.as_str().map(|x| x.to_string()).or_else(|| {
                    s.get("subject")
                        .or_else(|| s.get("name"))
                        .and_then(|x| x.as_str())
                        .map(|x| x.to_string())
                })
            })
            .filter(|s| !s.is_empty())
            .collect())
    }

    pub fn delete_person(&self, subject: &str) -> Result<(), String> {
        let url = format!("{}/recognition/subjects/{}", self.base_url, urlenc(subject));
        let resp = self
            .agent()
            .delete(&url)
            .header("x-api-key", &self.api_key)
            .call()
            .map_err(|e| format!("CompreFace 删除失败: {e}"))?;
        // 显式校验状态码（http_status_as_error(false) 时 4xx/5xx 仍返回 Ok）
        if resp.status().as_u16() >= 300 {
            return Err(format!("CompreFace 删除 HTTP {}", resp.status().as_u16()));
        }
        Ok(())
    }
}

fn urlenc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
