// 2026-11-12 batch A split: shared state.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::broadcast;

/// 应用共享状态
pub struct AppState {
    pub db_path: String,
    pub admin_token: String,
    /// R2-103 对齐：只读令牌（LS_READ_TOKEN；写永拒）
    pub read_token: String,
    pub read_protected: bool,
    pub(crate) access: crate::access::AccessControl,
    pub trusted_hosts: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub web_dist: String,
    pub data_dir: String,
    pub version: String,
    pub events_tx: broadcast::Sender<String>,
    /// T-330：后台作业（retention / backup / escalation），启动时 start()
    pub jobs: Arc<ls_jobs::Jobs>,
    /// R2-309：配对码/失败计数/设备 touch 节流（进程内，与 Python devices.py 同语义）
    pub pair_state: std::sync::Mutex<PairState>,
    /// R2-108 对齐（2026-11-11 深审补齐）：测试主机名 testserver 仅在测试态放行
    /// （LS_TESTING=1，与 Python settings.testing / PYTEST_CURRENT_TEST 同口径）。
    pub testing: bool,
    /// RA1.2（ADR-0011）：进程内分析 worker 注册表（start/stop_camera 真实现）
    pub workers: crate::worker::Workers,
    /// RA2-S2：MQTT 事件桥（默认关——LS_MQTT_ENABLED=1 才构造；None=未启用）
    pub mqtt: Option<Arc<ls_dispatch::mqtt::MqttBus>>,
    /// RA2-S7：插件管理器（manifest 校验/启停/日志）
    pub plugins: crate::plugins::PluginManager,
    /// RA2-S5：事件 AI 复核（默认 provider=none 零开销）
    pub genai: crate::genai::GenAI,
    /// LS-051：go2rtc 进程内托管（CREATE_NO_WINDOW + Job Object 防孤儿）
    pub go2rtc: crate::go2rtc::SharedGo2rtc,
    /// LS-051：实时检测元数据广播（≤5Hz，WS /ws/detections/:cid 消费）
    pub(crate) detections: DetectionStreams,
}

#[derive(Default)]
pub(crate) struct DetectionStreams(Mutex<HashMap<i64, Weak<broadcast::Sender<String>>>>);

impl DetectionStreams {
    pub fn subscribe(&self, camera: i64) -> Arc<broadcast::Sender<String>> {
        let mut streams = self.0.lock().unwrap_or_else(|e| e.into_inner());
        streams.retain(|_, sender| sender.strong_count() > 0);
        if let Some(sender) = streams.get(&camera).and_then(Weak::upgrade) {
            return sender;
        }
        let sender = Arc::new(broadcast::Sender::new(4));
        streams.insert(camera, Arc::downgrade(&sender));
        sender
    }

    pub fn subscribed(&self, camera: i64) -> Option<Arc<broadcast::Sender<String>>> {
        let streams = self.0.lock().ok()?;
        streams
            .get(&camera)
            .and_then(Weak::upgrade)
            .filter(|sender| sender.receiver_count() > 0)
    }
}

#[derive(Default)]

pub struct PairState {
    pub(crate) pending: std::collections::HashMap<String, crate::handlers::pairing::PendingPairing>,
    /// device_id -> 上次 touch（60s 节流，R2-103）
    pub touch: std::collections::HashMap<i64, std::time::Instant>,
}

pub type Shared = Arc<AppState>;
