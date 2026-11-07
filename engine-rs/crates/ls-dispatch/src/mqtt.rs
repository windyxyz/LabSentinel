//! T-329：MQTT 事件总线（L2）——与 Python `mqtt_bridge.py` payload v1 同语义。
//!
//! 定位：事件**只读外发**总线，不是告警通道（Webhook/厂商渠道照旧、静默/过滤
//! 不作用于它）。契约：payload v1（`v`/`device_type` 字段）**只加不减**；
//! broker 不可达/发布异常一律吞掉计数，绝不影响事件主链路；密码不进日志/不进 payload。
//!
//! 与 Python 实现的差异（有意）：rumqttc 的 `Client::publish` 内部即有界 channel
//! （容量 = QUEUE_MAX，与 Python queue.Queue(maxsize=1000) 对齐），满时返回错误
//! → `failed`/`dropped` 计数，不再单独建线程排空。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;

pub const PAYLOAD_VERSION: u64 = 1;
pub const QUEUE_MAX: usize = 1000;

/// 连接配置（凭据只进内存；由调用方从环境/配置读取，不在本 crate 写死）。
#[derive(Clone, Default)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub client_id: String,
    pub topic_prefix: String,
    /// 0 | 1 | 2（越界回退 0）
    pub qos: u8,
    pub retain: bool,
    pub tls: bool,
}

impl MqttConfig {
    fn qos(&self) -> rumqttc::QoS {
        match self.qos {
            1 => rumqttc::QoS::AtLeastOnce,
            2 => rumqttc::QoS::ExactlyOnce,
            _ => rumqttc::QoS::AtMostOnce,
        }
    }
}

/// 事件字段（与 Python `build_payload` 输入一致；不含本地绝对路径/凭据）。
pub struct EventFields<'a> {
    pub id: i64,
    pub ts: &'a str,
    pub camera_id: i64,
    pub camera_name: Option<&'a str>,
    pub type_: &'a str,
    pub level: i64,
    pub track_id: i64,
    pub detail: &'a str,
    pub zones: &'a [String],
    pub has_snapshot: bool,
    pub api_base: &'a str,
}

/// payload v1（**只加不减**：与 Python `mqtt_bridge.build_payload` 逐字段一致）。
pub fn build_payload(ev: &EventFields) -> String {
    json!({
        "v": PAYLOAD_VERSION,
        "device_type": "lab-monitor",
        "id": ev.id,
        "ts": ev.ts,
        "camera_id": ev.camera_id,
        "camera_name": ev.camera_name,
        "type": ev.type_,
        "level": ev.level,
        "track_id": ev.track_id,
        "detail": ev.detail,
        "zones": ev.zones,
        "has_snapshot": ev.has_snapshot,
        "snapshot_url": format!("/api/events/{}/snapshot", ev.id),
        "api_base": ev.api_base,
    })
    .to_string()
}

/// 与 Python `topic_for` 一致：`<prefix>/events/<camera_id>/<type>`。
pub fn topic_for(prefix: &str, camera_id: i64, type_: &str) -> String {
    format!("{prefix}/events/{camera_id}/{type_}")
}

/// 发布计数（published/dropped/failed + connected；供 status 观测）。
/// Arc 共享给 eventloop 线程（Atomic 无 Clone）。
#[derive(Default, Clone)]
pub struct MqttStats {
    pub published: std::sync::Arc<AtomicU64>,
    pub dropped: std::sync::Arc<AtomicU64>,
    pub failed: std::sync::Arc<AtomicU64>,
    pub connected: std::sync::Arc<AtomicBool>,
}

impl MqttStats {
    pub fn snapshot(&self) -> (u64, u64, u64, bool) {
        (
            self.published.load(Ordering::Relaxed),
            self.dropped.load(Ordering::Relaxed),
            self.failed.load(Ordering::Relaxed),
            self.connected.load(Ordering::Relaxed),
        )
    }
}

/// MQTT 总线（rumqttc 同步客户端：publish 内部 channel 有界，非阻塞）。
pub struct MqttBus {
    config: MqttConfig,
    client: Mutex<Option<rumqttc::Client>>,
    pub stats: MqttStats,
    /// 连接轮询线程的停止标志（stop 后不再处理事件/不再重连）
    stop: std::sync::Arc<AtomicBool>,
}

impl MqttBus {
    /// 启动总线（enabled 判定由调用方负责）；连接失败不阻塞——rumqttc
    /// eventloop 自动重连，Connack 后发 `online`（retain），LWT 发 `offline`。
    pub fn start(config: MqttConfig) -> Result<Self, String> {
        if config.host.is_empty() {
            return Err("mqtt host 为空".into());
        }
        let mut opts = rumqttc::MqttOptions::new(
            if config.client_id.is_empty() {
                format!("labsentinel-{}", std::process::id())
            } else {
                config.client_id.clone()
            },
            &config.host,
            config.port,
        );
        opts.set_keep_alive(Duration::from_secs(20));
        if !config.username.is_empty() {
            opts.set_credentials(config.username.clone(), config.password.clone());
        }
        if config.tls {
            // 系统信任源（rustls-native-certs，与 Python `tls_set()` 同口径）；
            // 不提供跳过校验开关。平台无根证书时连接失败（与 Python 一致）。
            let mut roots = rustls::RootCertStore::empty();
            if let Ok(certs) = rustls_native_certs::load_native_certs() {
                for cert in certs {
                    let _ = roots.add(cert);
                }
            }
            let tls_cfg = rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            opts.set_transport(rumqttc::Transport::Tls(rumqttc::TlsConfiguration::Rustls(
                std::sync::Arc::new(tls_cfg),
            )));
        }
        let status_topic = format!("{}/status", config.topic_prefix);
        opts.set_last_will(rumqttc::LastWill::new(
            status_topic.clone(),
            "offline",
            config.qos(),
            true,
        ));
        let (client, mut connection) = rumqttc::Client::new(opts, QUEUE_MAX);
        let stats = MqttStats::default();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        // 连接轮询：ConnAck → online（retain）；断开清 connected
        let st = stats.clone();
        let ev_stop = stop.clone();
        let status_topic = status_topic.clone();
        let qos = config.qos();
        let retain = config.retain;
        let prefix = config.topic_prefix.clone();
        let ev_client = client.clone();
        std::thread::Builder::new()
            .name("mqtt-eventloop".into())
            .spawn(move || {
                for ev in connection.iter() {
                    if ev_stop.load(Ordering::Relaxed) {
                        return;
                    }
                    match ev {
                        Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_))) => {
                            st.connected.store(true, Ordering::Relaxed);
                            let _ = ev_client.publish(&status_topic, qos, true, "online");
                        }
                        Ok(_) => {}
                        Err(_) => {
                            st.connected.store(false, Ordering::Relaxed);
                        }
                    }
                }
                let _ = (qos, retain, prefix);
            })
            .map_err(|e| format!("mqtt eventloop 启动失败: {e}"))?;
        Ok(MqttBus {
            config,
            client: Mutex::new(Some(client)),
            stats,
            stop,
        })
    }

    /// 事件入队即返回（与 Python publish_event 同语义：绝不抛、绝不阻塞主链路）。
    pub fn publish_event(&self, ev: &EventFields) -> bool {
        let guard = self.client.lock().unwrap_or_else(|e| e.into_inner());
        let Some(client) = guard.as_ref() else {
            return false;
        };
        let topic = topic_for(&self.config.topic_prefix, ev.camera_id, ev.type_);
        let payload = build_payload(ev);
        // try_publish = 非阻塞 try_send（rumqttc 0.24 的 publish 是阻塞 send，
        // 会反压主链路；Python 语义是入队即返回、满则丢弃计数）。
        match client.try_publish(&topic, self.config.qos(), self.config.retain, payload) {
            Ok(()) => {
                self.stats.published.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(rumqttc::ClientError::TryRequest(_)) => {
                // 队列满或非法 topic：满丢最新 + 计数（Python queue.Full 同语义）
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
            Err(_) => {
                self.stats.failed.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut c) = self.client.lock() {
            if let Some(client) = c.take() {
                let _ = client.publish(
                    format!("{}/status", self.config.topic_prefix),
                    self.config.qos(),
                    true,
                    "offline",
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev<'a>(zones: &'a [String]) -> EventFields<'a> {
        EventFields {
            id: 12,
            ts: "2026-01-01T00:00:01",
            camera_id: 2,
            camera_name: Some("试剂区"),
            type_: "enter_no_ppe",
            level: 3,
            track_id: 7,
            detail: "未穿实验服",
            zones,
            has_snapshot: true,
            api_base: "http://127.0.0.1:8000",
        }
    }

    #[test]
    fn payload_v1_field_set_matches_python() {
        // 与 Python mqtt_bridge.build_payload 的字段集合逐一对齐（只加不减契约）
        let zones = vec!["试剂区".to_string()];
        let v: serde_json::Value = serde_json::from_str(&build_payload(&ev(&zones))).unwrap();
        for key in [
            "v",
            "device_type",
            "id",
            "ts",
            "camera_id",
            "camera_name",
            "type",
            "level",
            "track_id",
            "detail",
            "zones",
            "has_snapshot",
            "snapshot_url",
            "api_base",
        ] {
            assert!(v.get(key).is_some(), "缺字段 {key}");
        }
        assert_eq!(v["v"], 1);
        assert_eq!(v["device_type"], "lab-monitor");
        assert_eq!(v["snapshot_url"], "/api/events/12/snapshot");
        assert_eq!(v["has_snapshot"], true);
        assert_eq!(v["zones"], serde_json::json!(["试剂区"]));
    }

    #[test]
    fn topic_matches_python() {
        assert_eq!(
            topic_for("labsentinel", 2, "enter_no_ppe"),
            "labsentinel/events/2/enter_no_ppe"
        );
    }

    #[test]
    fn stats_snapshot_initial() {
        let s = MqttStats::default();
        assert_eq!(s.snapshot(), (0, 0, 0, false));
    }
}
