//! Saved provider settings and the CompreFace recognition adapter.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::Url;

pub const LOCAL_PROFILE: &str = "local-sface";
pub const MAX_PHOTO: usize = 5 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FaceMode {
    #[default]
    Off,
    Local,
    Api,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Default)]
pub struct FaceSelection {
    pub mode: FaceMode,
    pub api: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct FaceApiProfile {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub secret: String,
    pub enabled: bool,
    pub threshold: f32,
    pub timeout_s: u64,
}

impl FaceApiProfile {
    pub fn public(&self) -> Value {
        json!({"id": self.id, "name": self.name, "url": self.base_url, "enabled": self.enabled,
               "threshold": self.threshold, "timeout_s": self.timeout_s, "key_configured": !self.secret.is_empty()})
    }

    pub fn client(&self, data_dir: &Path) -> Result<FaceClient, String> {
        let key = crate::fernet::unprotect(&self.secret, data_dir);
        if key.trim().is_empty() {
            return Err("人脸识别 API 密钥不可用".into());
        }
        Ok(FaceClient {
            base_url: self.base_url.clone(),
            api_key: key,
            threshold: self.threshold,
            timeout_s: self.timeout_s,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct FaceSettings {
    pub active: FaceSelection,
    pub previous: Option<FaceSelection>,
    pub local_enabled: bool,
    pub local_threshold: f32,
    pub retention_days: u32,
    pub apis: Vec<FaceApiProfile>,
}

impl Default for FaceSettings {
    fn default() -> Self {
        Self {
            active: FaceSelection::default(),
            previous: None,
            local_enabled: true,
            local_threshold: 0.5,
            retention_days: 90,
            apis: Vec::new(),
        }
    }
}

impl FaceSettings {
    pub fn load(store: &ls_store::Store, data_dir: &Path) -> Self {
        if let Ok(Some(encoded)) = store.setting_get("face.settings") {
            return serde_json::from_str(&encoded).unwrap_or_default();
        }
        let mut settings = Self::default();
        let Some(client) = FaceClient::from_env() else {
            return settings;
        };
        if let Some(secret) = crate::fernet::protect(&client.api_key, data_dir) {
            let id = format!("legacy-{:x}", Sha256::digest(&client.base_url));
            settings.apis.push(FaceApiProfile {
                id: id.clone(),
                name: "现有 CompreFace".into(),
                base_url: client.base_url,
                secret,
                enabled: true,
                threshold: client.threshold,
                timeout_s: client.timeout_s,
            });
            settings.active.api = id;
            if env_enabled() {
                settings.active.mode = FaceMode::Api;
            }
        }
        settings
    }

    pub fn api(&self, id: &str) -> Option<&FaceApiProfile> {
        self.apis.iter().find(|profile| profile.id == id)
    }

    pub fn active_profile(&self) -> Option<&str> {
        match self.active.mode {
            FaceMode::Local if self.local_enabled => Some(LOCAL_PROFILE),
            FaceMode::Api if self.api(&self.active.api).is_some_and(|p| p.enabled) => {
                Some(&self.active.api)
            }
            _ => None,
        }
    }

    pub fn same_recognition(&self, now: &Self) -> bool {
        let Some(profile) = self.active_profile() else {
            return false;
        };
        if self.active != now.active || now.active_profile() != Some(profile) {
            return false;
        }
        match (self.api(profile), now.api(profile)) {
            (Some(before), Some(after)) => {
                before.threshold == after.threshold && before.timeout_s == after.timeout_s
            }
            (None, None) => self.local_threshold == now.local_threshold,
            _ => false,
        }
    }
}

pub fn env_enabled() -> bool {
    std::env::var("LS_FACE_ENABLED").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

pub fn normalize_url(raw: &str, legacy_http: bool) -> Result<String, String> {
    let mut url = Url::parse(raw.trim()).map_err(|_| "请填写有效的 CompreFace 服务地址")?;
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    let supported_scheme =
        url.scheme() == "https" || (url.scheme() == "http" && (loopback || legacy_http));
    if !supported_scheme
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "远程 CompreFace 地址需要 HTTPS，本机可使用 HTTP。地址中请只填写主机、端口和路径。"
                .into(),
        );
    }
    let path = url.path().trim_end_matches('/');
    let api_path = if path.ends_with("/api/v1") {
        path.to_string()
    } else {
        format!("{path}/api/v1")
    };
    url.set_path(&api_path);
    Ok(url.into())
}

pub struct FaceClient {
    pub base_url: String,
    pub api_key: String,
    pub threshold: f32,
    pub timeout_s: u64,
}

impl FaceClient {
    pub fn from_env() -> Option<Self> {
        let raw =
            std::env::var("LS_COMPREFACE_URL").unwrap_or_else(|_| "http://127.0.0.1:8000".into());
        let api_key = std::env::var("LS_COMPREFACE_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty())?;
        Some(Self {
            base_url: normalize_url(&raw, true).ok()?,
            api_key,
            threshold: std::env::var("LS_FACE_THRESHOLD")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| (0.0..=1.0).contains(v))
                .unwrap_or(0.75),
            timeout_s: std::env::var("LS_FACE_TIMEOUT_S")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5u64)
                .clamp(1, 15),
        })
    }

    fn agent(&self) -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(self.timeout_s.max(1))))
            .max_redirects(0)
            .http_status_as_error(false)
            .build()
            .into()
    }

    /// multipart/form-data 手工拼装（ureq 无原生 multipart；仅一个 file 字段）。
    fn multipart_file(field: &str, filename: &str, data: &[u8]) -> (String, Vec<u8>) {
        let boundary = format!("----LabSentinelFace-{}", crate::envfile::random_token());
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
        (boundary, body)
    }

    fn post_photo(&self, path: &str, photo: &[u8]) -> Result<Value, String> {
        let (boundary, body) = Self::multipart_file("file", "photo.jpg", photo);
        let url = format!("{}{path}", self.base_url);
        let mut resp = self
            .agent()
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header(
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .send(&body[..])
            .map_err(|_| "CompreFace 请求失败，请检查连接和超时设置")?;
        if !resp.status().is_success() {
            return Err(format!(
                "CompreFace HTTP {}，请检查照片是否为单人且清晰",
                resp.status().as_u16()
            ));
        }
        response_json(resp.body_mut())
    }

    pub fn add_person(&self, subject: &str, photo: &[u8]) -> Result<String, String> {
        let response = self.post_photo(
            &format!("/recognition/faces?subject={}", urlenc(subject)),
            photo,
        )?;
        response
            .get("image_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .ok_or("CompreFace 未返回样本编号".into())
    }

    pub fn recognize(&self, photo: &[u8]) -> Result<Vec<(String, f32)>, String> {
        let data = self.post_photo("/recognition/recognize?limit=2&prediction_count=3", photo)?;
        let results = data
            .get("result")
            .and_then(Value::as_array)
            .ok_or("CompreFace 响应缺少识别结果")?;
        if results.len() != 1 {
            return Ok(Vec::new());
        }
        let subjects = results[0]
            .get("subjects")
            .and_then(Value::as_array)
            .ok_or("CompreFace 响应缺少候选人员")?;
        let mut matches: Vec<_> = subjects
            .iter()
            .filter_map(|entry| {
                let subject = entry.get("subject")?.as_str()?;
                let similarity = entry.get("similarity")?.as_f64()? as f32;
                (0.0..=1.0)
                    .contains(&similarity)
                    .then(|| (subject.to_string(), similarity))
            })
            .collect();
        matches.sort_by(|a, b| b.1.total_cmp(&a.1));
        Ok(matches)
    }

    pub fn list_persons(&self) -> Result<Vec<String>, String> {
        let url = format!("{}/recognition/subjects", self.base_url);
        let mut resp = self
            .agent()
            .get(&url)
            .header("x-api-key", &self.api_key)
            .call()
            .map_err(|_| "CompreFace 请求失败，请检查连接和超时设置")?;
        if resp.status().as_u16() >= 300 {
            return Err(format!("CompreFace 列表 HTTP {}", resp.status().as_u16()));
        }
        let v = response_json(resp.body_mut())?;
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
        self.delete(&format!("/recognition/subjects/{}", urlenc(subject)))
    }

    fn delete(&self, path: &str) -> Result<(), String> {
        let url = format!("{}{path}", self.base_url);
        let resp = self
            .agent()
            .delete(&url)
            .header("x-api-key", &self.api_key)
            .call()
            .map_err(|_| "CompreFace 请求失败，请检查连接和超时设置")?;
        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            return Err(format!("CompreFace 删除 HTTP {}", resp.status().as_u16()));
        }
        Ok(())
    }
}

fn response_json(body: &mut ureq::Body) -> Result<Value, String> {
    let text = body
        .with_config()
        .limit(1024 * 1024)
        .read_to_string()
        .map_err(|_| "无法读取 CompreFace 响应")?;
    serde_json::from_str(&text).map_err(|_| "CompreFace 返回了无效的 JSON".into())
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
