use std::{
    fs::File,
    io::{self, BufReader, Read},
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use ls_ingest::child_guard::ChildGuard;
use serde::Deserialize;
use serde_json::json;

use crate::{
    detect::{profiles_load, profiles_store, Profile},
    state::Shared,
    util::detail_err,
};

const PORT: u16 = 18680;
const PROFILE_ID: &str = "managed-local-vlm";
const ALIAS: &str = "labsentinel-local";
const BINARY: &str = if cfg!(windows) {
    "llama-server.exe"
} else {
    "llama-server"
};

#[derive(Default)]
pub(crate) struct LocalVlm(Mutex<Option<Running>>);

struct Running {
    child: ChildGuard,
    backend: &'static str,
}

impl LocalVlm {
    pub fn stop(&self) {
        if let Ok(mut state) = self.0.lock() {
            *state = None;
        }
    }
}

fn binary() -> Option<PathBuf> {
    let path = crate::deploy::default_install_root()
        .join("tools")
        .join("llama")
        .join(BINARY);
    path.is_file().then_some(path)
}

fn candidate(name: &str, root: &Path) -> Result<PathBuf, String> {
    let path = Path::new(name);
    if name.len() > 255
        || name.contains(['/', '\\', ':'])
        || path
            .extension()
            .is_none_or(|ext| !ext.eq_ignore_ascii_case("gguf"))
    {
        return Err("请选择本地视觉模型目录中的 GGUF 文件".into());
    }
    let root = root.canonicalize().map_err(|_| "本地视觉模型目录不存在")?;
    let asset = root
        .join(path)
        .canonicalize()
        .map_err(|_| "GGUF 文件不存在")?;
    if asset.parent() != Some(root.as_path()) || !asset.is_file() {
        return Err("GGUF 需要位于本地视觉模型目录".into());
    }
    Ok(asset)
}

fn int<const N: usize>(reader: &mut impl Read) -> io::Result<[u8; N]> {
    let mut data = [0; N];
    reader.read_exact(&mut data)?;
    Ok(data)
}

fn u32_le(reader: &mut impl Read) -> io::Result<u32> {
    Ok(u32::from_le_bytes(int(reader)?))
}
fn u64_le(reader: &mut impl Read) -> io::Result<u64> {
    Ok(u64::from_le_bytes(int(reader)?))
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "GGUF metadata")
}

fn skip(reader: &mut impl Read, length: u64) -> io::Result<()> {
    if io::copy(&mut reader.take(length), &mut io::sink())? == length {
        Ok(())
    } else {
        Err(invalid())
    }
}

fn skip_value(reader: &mut impl Read, kind: u32, count: u64) -> io::Result<()> {
    let number_size = match kind {
        0 | 1 | 7 => 1,
        2 | 3 => 2,
        4..=6 => 4,
        10..=12 => 8,
        _ => 0,
    };
    if number_size > 0 {
        return skip(reader, count.checked_mul(number_size).ok_or_else(invalid)?);
    }
    if kind != 8 || count > 2_000_000 {
        return Err(invalid());
    }
    for _ in 0..count {
        let len = u64_le(reader)?;
        skip(reader, len)?;
    }
    Ok(())
}

fn gguf_type(reader: &mut impl Read) -> io::Result<Option<u32>> {
    if int::<4>(reader)? != *b"GGUF" || !(2..=3).contains(&u32_le(reader)?) || u64_le(reader)? == 0
    {
        return Err(invalid());
    }
    let entries = u64_le(reader)?;
    if entries > 16_384 {
        return Err(invalid());
    }
    for _ in 0..entries {
        let len = u64_le(reader)?;
        if len > 4096 {
            return Err(invalid());
        }
        let mut key = vec![0; len as usize];
        reader.read_exact(&mut key)?;
        let kind = u32_le(reader)?;
        if key == b"general.file_type" {
            return if kind == 4 {
                Ok(Some(u32_le(reader)?))
            } else {
                Err(invalid())
            };
        }
        let (kind, count) = if kind == 9 {
            (u32_le(reader)?, u64_le(reader)?)
        } else {
            (kind, 1)
        };
        skip_value(reader, kind, count)?;
    }
    Ok(None)
}

fn check_gguf(path: &Path, projector: bool) -> Result<(), String> {
    let mut file =
        BufReader::new(File::open(path).map_err(|_| "无法读取 GGUF 文件")?).take(64 * 1024 * 1024);
    let file_type = gguf_type(&mut file).map_err(|_| "GGUF 文件头无效或版本不支持")?;
    match file_type {
        Some(0 | 1 | 7 | 18 | 32) => Ok(()),
        None if projector => Ok(()),
        _ => Err(
            "请选择 F16、BF16、Q6_K 或 Q8_0 等受支持的权重。当前量化类型无法用于离线视觉复核。"
                .into(),
        ),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalConfig {
    model: String,
    projector: String,
    #[serde(default)]
    gpu_layers: u32,
}

fn launch(
    bin: &Path,
    model: &Path,
    projector: &Path,
    key: &str,
    layers: u32,
) -> Result<ChildGuard, String> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, PORT)).map_err(|_| "回环服务端口已被占用")?;
    let mut cmd = Command::new(bin);
    cmd.current_dir(bin.parent().ok_or("找不到离线服务目录")?)
        .args([
            "--host",
            "127.0.0.1",
            "--port",
            &PORT.to_string(),
            "--parallel",
            "1",
            "--ctx-size",
            "4096",
            "--batch-size",
            "256",
            "--ubatch-size",
            "128",
            "--alias",
            ALIAS,
            "--api-key",
            key,
            "--n-gpu-layers",
            &layers.to_string(),
            "--model",
        ])
        .arg(model)
        .arg("--mmproj")
        .arg(projector);
    if layers == 0 {
        cmd.arg("--no-mmproj-offload");
    }
    let mut child = ChildGuard::spawn(cmd)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .timeout_global(Some(Duration::from_millis(500)))
        .build()
        .into();
    let deadline = Instant::now() + Duration::from_secs(45);
    while child.is_running() && Instant::now() < deadline {
        if agent
            .get(format!("http://127.0.0.1:{PORT}/health"))
            .header("Authorization", &format!("Bearer {key}"))
            .call()
            .is_ok()
        {
            return Ok(child);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(
        "离线视觉进程提前退出或模型加载超时。请核对 llama.cpp 版本、模型和视觉投影是否配套。"
            .into(),
    )
}

pub(crate) async fn local_vlm_status(State(st): State<Shared>) -> Response {
    let status = match st.local_vlm.0.try_lock() {
        Ok(mut state) => state.as_mut().map_or("stopped", |running| {
            if running.child.is_running() {
                running.backend
            } else {
                "stopped"
            }
        }),
        Err(_) => "starting",
    };
    Json(json!({"status": status, "runtime_available": binary().is_some(), "directory": "models/local-vlm", "profile_id": PROFILE_ID})).into_response()
}

pub(crate) async fn local_vlm_start(
    State(st): State<Shared>,
    Json(config): Json<LocalConfig>,
) -> Response {
    match tokio::task::spawn_blocking(move || start(&st, config)).await {
        Ok(Ok(backend)) => {
            Json(json!({"status": backend, "profile_id": PROFILE_ID})).into_response()
        }
        Ok(Err(error)) => detail_err(StatusCode::UNPROCESSABLE_ENTITY, &error),
        Err(_) => detail_err(StatusCode::INTERNAL_SERVER_ERROR, "离线视觉服务启动失败"),
    }
}

fn start(st: &Shared, config: LocalConfig) -> Result<&'static str, String> {
    let root = Path::new(&st.data_dir).join("models").join("local-vlm");
    let model = candidate(&config.model, &root)?;
    let projector = candidate(&config.projector, &root)?;
    if model == projector || config.gpu_layers > 128 {
        return Err("权重与投影需要分别选择，GPU 层数须在 0 至 128 之间".into());
    }
    check_gguf(&model, false)?;
    check_gguf(&projector, true)?;
    let bin = binary().ok_or("请先在软件 tools/llama 目录放入官方 llama.cpp Vulkan 运行时")?;
    let mut active = st
        .local_vlm
        .0
        .try_lock()
        .map_err(|_| "已有离线模型正在启动")?;
    if active.as_mut().is_some_and(|s| s.child.is_running()) {
        return Err("请先停止当前离线视觉服务".into());
    }
    *active = None;
    let mut random = [0; 24];
    getrandom::getrandom(&mut random).map_err(|_| "生成回环服务令牌失败")?;
    let key = crate::util::sha256_hex(&random);
    let (child, backend) = match launch(&bin, &model, &projector, &key, config.gpu_layers) {
        Ok(child) => (
            child,
            if config.gpu_layers == 0 {
                "cpu"
            } else {
                "gpu_requested"
            },
        ),
        Err(_) if config.gpu_layers > 0 => {
            (launch(&bin, &model, &projector, &key, 0)?, "cpu_fallback")
        }
        Err(error) => return Err(error),
    };
    let store = ls_store::Store::open(&st.db_path).map_err(|_| "无法保存本地接入档案")?;
    let data = Path::new(&st.data_dir);
    let before = crate::detect::DetectCfg::for_role(&store, data, "verifier");
    let tx = store
        .conn()
        .unchecked_transaction()
        .map_err(|err| err.to_string())?;
    let mut profiles = profiles_load(&store, data);
    profiles.retain(|profile| profile.id != PROFILE_ID);
    profiles.push(Profile {
        id: PROFILE_ID.into(),
        name: "离线视觉（本机）".into(),
        provider: "openai_compat".into(),
        url: format!("http://127.0.0.1:{PORT}/v1"),
        model: ALIAS.into(),
        api_key: key,
        timeout_s: 30,
        allow_remote: false,
    });
    profiles_store(&store, data, &profiles)?;
    tx.commit().map_err(|err| err.to_string())?;
    *active = Some(Running { child, backend });
    crate::detect::reload_verifier(st, &store, &before);
    Ok(backend)
}

pub(crate) async fn local_vlm_stop(State(st): State<Shared>) -> Response {
    match tokio::task::spawn_blocking(move || st.local_vlm.stop()).await {
        Ok(()) => Json(json!({"status": "stopped"})).into_response(),
        Err(_) => detail_err(StatusCode::INTERNAL_SERVER_ERROR, "无法停止离线视觉服务"),
    }
}
