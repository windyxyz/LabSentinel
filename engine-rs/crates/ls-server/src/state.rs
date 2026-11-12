// 2026-11-12 batch A split: shared state.
use std::sync::Arc;
use tokio::sync::broadcast;

/// 应用共享状态
pub struct AppState {
    pub db_path: String,
    pub admin_token: String,
    /// R2-103 对齐：只读令牌（LS_READ_TOKEN；写永拒）
    pub read_token: String,
    pub read_protected: bool,
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
}

#[derive(Default)]

pub struct PairState {
    /// code -> expire_unix_ts
    pub pending: std::collections::HashMap<String, i64>,
    /// 连续 claim 失败计数（5 次作废全部待配对码，N-17/R2-110）
    pub failures: u32,
    /// device_id -> 上次 touch（60s 节流，R2-103）
    pub touch: std::collections::HashMap<i64, std::time::Instant>,
}

pub type Shared = Arc<AppState>;
