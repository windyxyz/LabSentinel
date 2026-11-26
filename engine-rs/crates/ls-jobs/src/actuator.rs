//! Optional sound/light output. Only the actor thread writes to the device.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const SETTING_KEY: &str = "alert.actuator.config";
const RESET_KEY: &str = "actuator.pending_reset";
const RETRY_INTERVAL: Duration = Duration::from_secs(2);
const SYNC_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub target: Option<Target>,
    pub minimum_level: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    Http {
        url: String,
    },
    Modbus {
        port: String,
        baud: u32,
        unit: u8,
        first_coil: u16,
    },
}

#[derive(Clone, Default, Serialize)]
pub struct Status {
    pub desired_level: u8,
    pub applied_level: Option<u8>,
    pub last_error: Option<String>,
    pub reconfiguring: bool,
    #[serde(skip)]
    target: Option<Target>,
}

impl Config {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !matches!(self.minimum_level.unwrap_or(2), 1..=3) {
            return Err("联动起始级别须为 L1、L2 或 L3");
        }
        match self.target.as_ref() {
            None => Ok(()),
            Some(Target::Http { url }) if valid_url(url) => Ok(()),
            Some(Target::Http { .. }) => {
                Err("继电器地址须使用内网 IPv4 地址，禁止在 URL 中附带凭据")
            }
            Some(Target::Modbus {
                port,
                baud,
                unit,
                first_coil,
            }) => {
                if !valid_port(port)
                    || !matches!(baud, 4800 | 9600 | 19200 | 38400 | 57600 | 115200)
                    || !matches!(unit, 1..=247)
                    || *first_coil > u16::MAX - 2
                {
                    return Err("串口、波特率、设备地址或线圈起始地址无效");
                }
                Ok(())
            }
        }
    }
}

impl Status {
    pub fn for_config(mut self, config: &Config) -> Self {
        if self.target != config.target {
            self.applied_level = None;
            self.reconfiguring = true;
        }
        self
    }
}

fn valid_url(raw: &str) -> bool {
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.query().is_none()
        && matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip.is_private() || ip.is_loopback())
}

fn valid_port(port: &str) -> bool {
    #[cfg(windows)]
    return port
        .strip_prefix("COM")
        .and_then(|number| number.parse::<u16>().ok())
        .is_some_and(|n| (1..=256).contains(&n));
    #[cfg(not(windows))]
    return ["/dev/ttyUSB", "/dev/ttyACM"].into_iter().any(|prefix| {
        port.strip_prefix(prefix)
            .is_some_and(|port| !port.is_empty() && port.bytes().all(|x| x.is_ascii_digit()))
    });
}

fn crc(bytes: &[u8]) -> [u8; 2] {
    let mut crc = 0xffff_u16;
    for &byte in bytes {
        crc ^= u16::from(byte);
        for _ in 0..8 {
            let low = crc & 1 != 0;
            crc >>= 1;
            if low {
                crc ^= 0xa001;
            }
        }
    }
    crc.to_le_bytes()
}

fn modbus_request(unit: u8, first_coil: u16, level: u8) -> [u8; 10] {
    let [hi, lo] = first_coil.to_be_bytes();
    let mask = if level == 0 { 0 } else { 1 << (level - 1) };
    let mut request = [unit, 0x0f, hi, lo, 0, 3, 1, mask, 0, 0];
    let checksum = crc(&request[..8]);
    request[8..].copy_from_slice(&checksum);
    request
}

fn send(target: &Target, level: u8) -> Result<(), String> {
    match target {
        Target::Http { url } => {
            let agent = ureq::Agent::config_builder()
                .proxy(None)
                .max_redirects(0)
                .timeout_global(Some(Duration::from_secs(1)))
                .build()
                .new_agent();
            let request = format!("{{\"level\":{level}}}");
            let response = agent
                .post(url)
                .header("Content-Type", "application/json")
                .send(request.as_str())
                .map_err(|_| "HTTP 继电器未确认状态".to_owned())?;
            if !response.status().is_success() {
                return Err("HTTP 继电器未返回成功状态".into());
            }
        }
        Target::Modbus {
            port,
            baud,
            unit,
            first_coil,
        } => {
            let mut serial = serialport::new(port, *baud)
                .data_bits(serialport::DataBits::Eight)
                .parity(serialport::Parity::Even)
                .stop_bits(serialport::StopBits::One)
                .timeout(Duration::from_millis(500))
                .open()
                .map_err(|error| format!("串口打开失败: {error}"))?;
            let request = modbus_request(*unit, *first_coil, level);
            serial
                .clear(serialport::ClearBuffer::Input)
                .map_err(|err| format!("串口清理失败: {err}"))?;
            serial
                .write_all(&request)
                .map_err(|err| format!("继电器写入失败: {err}"))?;
            let mut reply = [0u8; 8];
            serial
                .read_exact(&mut reply[..2])
                .map_err(|err| format!("继电器响应超时: {err}"))?;
            if reply[..2] == [*unit, 0x8f] {
                return Err("继电器拒绝写入线圈".into());
            }
            serial
                .read_exact(&mut reply[2..])
                .map_err(|err| format!("继电器响应不完整: {err}"))?;
            if reply[..6] != request[..6] || reply[6..] != crc(&reply[..6]) {
                return Err("Modbus 响应与请求不匹配".into());
            }
        }
    }
    Ok(())
}

pub fn load(store: &ls_store::Store) -> Result<Config, String> {
    let Some(raw) = store
        .setting_get(SETTING_KEY)
        .map_err(|error| error.to_string())?
    else {
        return Ok(Config::default());
    };
    let config: Config = serde_json::from_str(&raw).map_err(|_| "无法读取声光联动配置")?;
    config.validate()?;
    Ok(config)
}

fn pending_reset(store: &ls_store::Store) -> Result<Option<Target>, String> {
    let Some(raw) = store
        .setting_get(RESET_KEY)
        .map_err(|err| err.to_string())?
    else {
        return Ok(None);
    };
    let target = serde_json::from_str(&raw).map_err(|_| "无法读取待复位设备")?;
    let config = Config {
        target: Some(target),
        minimum_level: None,
    };
    config.validate()?;
    Ok(config.target)
}

fn write_device(
    store: &ls_store::Store,
    device: &Target,
    level: u8,
    armed: bool,
) -> Result<(), String> {
    // Record the potentially energized target before I/O. A timeout or crash
    // can happen after it accepts a command but before we receive a reply.
    if level != 0 && !armed {
        let value = serde_json::to_string(device).map_err(|err| err.to_string())?;
        store
            .setting_set(RESET_KEY, &value)
            .map_err(|err| err.to_string())?;
    }
    send(device, level)?;
    if level == 0 && armed {
        store
            .settings_delete(&[RESET_KEY.to_owned()])
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn reconcile(
    store: &ls_store::Store,
    status: &mut Status,
    next_sync: &mut Instant,
) -> Result<(), String> {
    let config = load(store)?;
    let armed = pending_reset(store)?;
    let now = Instant::now();
    status.desired_level = if config.target.is_some() {
        let local = ls_core::time::local_now_iso();
        let since = ls_core::time::format_iso(ls_core::time::local_offset_secs(-15));
        let level = store
            .actuator_level(&local, &since)
            .map_err(|err| err.to_string())?;
        if level < config.minimum_level.unwrap_or(2) {
            0
        } else {
            level
        }
    } else {
        0
    };
    if let Some(old) = armed
        .as_ref()
        .filter(|&old| Some(old) != config.target.as_ref())
    {
        if !status.reconfiguring || status.target.as_ref() != Some(old) {
            *next_sync = now;
            status.target = Some(old.clone());
        }
        status.reconfiguring = true;
        status.applied_level = None;
        if now >= *next_sync {
            status.last_error = write_device(store, old, 0, true)
                .err()
                .map(|err| format!("原设备复位失败，正在重试。{err}"));
            *next_sync = Instant::now() + RETRY_INTERVAL;
        }
        return Ok(());
    }

    if status.target != config.target || status.reconfiguring {
        status.target = config.target;
        status.reconfiguring = false;
        status.applied_level = None;
        status.last_error = None;
        *next_sync = now;
    }
    if let Some(device) = status.target.as_ref().filter(|_| {
        status
            .applied_level
            .is_some_and(|level| level != status.desired_level)
            || now >= *next_sync
    }) {
        match write_device(store, device, status.desired_level, armed.is_some()) {
            Ok(()) => {
                status.applied_level = Some(status.desired_level);
                status.last_error = None;
                *next_sync = Instant::now() + SYNC_INTERVAL;
            }
            Err(error) => {
                status.applied_level = None;
                status.last_error = Some(error);
                *next_sync = Instant::now() + RETRY_INTERVAL;
            }
        }
    }
    Ok(())
}

pub(crate) fn run(path: &str, stop: &AtomicBool, shared: &Mutex<Status>) {
    let mut status = Status::default();
    let mut next_sync = Instant::now();
    let mut store = None;
    while !stop.load(Ordering::Acquire) {
        let update = (|| {
            if store.is_none() {
                store = Some(ls_store::Store::open(path).map_err(|error| error.to_string())?);
            }
            reconcile(
                store.as_ref().expect("actuator store"),
                &mut status,
                &mut next_sync,
            )
        })();
        if let Err(error) = update {
            store = None;
            status.applied_level = None;
            status.last_error = Some(error);
        }
        *super::lock(shared) = status.clone();
        std::thread::sleep(Duration::from_millis(400));
    }
    if let Some(store) = store.or_else(|| ls_store::Store::open(path).ok()) {
        if let Ok(Some(device)) = pending_reset(&store) {
            let _ = write_device(&store, &device, 0, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actuator_modbus_packet_writes_all_three_coils_together() {
        assert_eq!(crc(&[0x02, 0x07]), [0x41, 0x12]);
        assert_eq!(&modbus_request(1, 256, 3)[..8], &[1, 15, 1, 0, 0, 3, 1, 4]);
        assert_eq!(&modbus_request(1, 256, 0)[..8], &[1, 15, 1, 0, 0, 3, 1, 0]);
    }

    #[test]
    fn actuator_http_target_is_local_without_ambiguous_credentials() {
        assert!(valid_url("http://192.168.1.9:8080/signal"));
        for bad in [
            "http://8.8.8.8/notify",
            "http://user:pass@127.0.0.1/",
            "http://169.254.169.254/",
            "https://localhost/",
            "http://127.0.0.1?token=1",
        ] {
            assert!(!valid_url(bad));
        }
    }

    #[test]
    fn actuator_updates_http_relay_after_disconnect_and_recovery() {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::sync::atomic::AtomicU8;

        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/signal", listener.local_addr().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let path = path.to_str().unwrap();
        let store = ls_store::Store::open(path).unwrap();
        store.ensure_schema().unwrap();
        let config = Config {
            target: Some(Target::Http { url: endpoint }),
            minimum_level: Some(2),
        };
        store
            .setting_set(SETTING_KEY, &serde_json::to_string(&config).unwrap())
            .unwrap();
        let stop = AtomicBool::new(false);
        let offline = AtomicBool::new(false);
        let received = AtomicU8::new(255);
        let status = Mutex::new(Status::default());
        std::thread::scope(|scope| {
            scope.spawn(|| {
                while !stop.load(Ordering::Acquire) {
                    let Ok((mut socket, _)) = listener.accept() else {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    };
                    socket.set_nonblocking(false).unwrap();
                    socket.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
                    let mut reader = BufReader::new(&socket);
                    let mut length = 0;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                            break;
                        }
                        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                            length = value.trim().parse().unwrap();
                        }
                    }
                    let mut payload = vec![0; length];
                    reader.read_exact(&mut payload).unwrap();
                    if offline.load(Ordering::Acquire) {
                        let _ = socket.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    } else {
                        let level = serde_json::from_slice::<serde_json::Value>(&payload).unwrap()["level"].as_u64().unwrap();
                        received.store(level as u8, Ordering::Release);
                        let _ = socket.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            });
            scope.spawn(|| run(path, &stop, &status));
            let wait = |expected: u8, healthy: bool| {
                let deadline = Instant::now() + Duration::from_secs(5);
                while Instant::now() < deadline {
                    let snapshot = super::super::lock(&status).clone();
                    if snapshot.desired_level == expected
                        && if healthy {
                            snapshot.applied_level == Some(expected)
                                && received.load(Ordering::Acquire) == expected
                        } else {
                            snapshot.applied_level.is_none() && snapshot.last_error.is_some()
                        }
                    {
                        return true;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                false
            };
            let exercise = || -> rusqlite::Result<[bool; 5]> {
                use ls_store::{
                    AlertDispatchKind, EpisodeEvidence, EventEvidence, NewEpisode, NewEvent,
                };
                let alert = |camera_id, level| -> rusqlite::Result<_> {
                    let ts = ls_core::time::local_now_iso();
                    let episode = store.add_episode(&NewEpisode {
                        camera_id,
                        rule_id: camera_id,
                        zone_id: None,
                        track_id: 1,
                        behavior: "no_mask",
                        evidence: EpisodeEvidence {
                            first_ts: &ts,
                            last_ts: &ts,
                            duration_s: 1.0,
                            hits: 1,
                            peak_conf: 0.9,
                            tier: "alert",
                        },
                    })?;
                    store.add_event_with_outbox(
                        &NewEvent {
                            camera_id,
                            type_: "no_mask",
                            level,
                            track_id: 1,
                            detail: "fixture",
                            snapshot: None,
                            ts: &ts,
                        },
                        EventEvidence {
                            episode_id: Some(episode),
                            ..Default::default()
                        },
                        None,
                        Some(AlertDispatchKind::Initial),
                    )?;
                    Ok(episode)
                };
                let initial = wait(0, true);
                alert(1, 2)?;
                let warning = wait(2, true);
                offline.store(true, Ordering::Release);
                let severe = alert(2, 3)?;
                let disconnected = wait(3, false);
                offline.store(false, Ordering::Release);
                let resumed = wait(3, true);
                store.close_episode(severe, &ls_core::time::local_now_iso(), "recovered", None)?;
                let lowered = wait(2, true);
                Ok([initial, warning, disconnected, resumed, lowered])
            }();
            stop.store(true, Ordering::Release);
            assert_eq!(exercise.unwrap(), [true; 5]);
        });
    }
}
