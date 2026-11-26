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
    extract::{Query, State},
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

mod gpu;

pub(crate) fn owns_profile(url: &str, model: &str) -> bool {
    model == ALIAS && url == profile_url()
}

fn profile_url() -> String {
    format!("http://127.0.0.1:{PORT}/v1")
}

#[derive(Default)]
pub(crate) struct LocalVlm(Mutex<Option<Running>>);

struct Running {
    child: ChildGuard,
    status: LaunchStatus,
}

#[derive(Clone, serde::Serialize)]
struct LaunchStatus {
    status: &'static str,
    gpu: Option<gpu::Plan>,
    fallback_reason: Option<String>,
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

#[derive(Default)]
struct GgufMetadata {
    file_type: Option<u32>,
    block_count: Option<u32>,
}

fn gguf_metadata(reader: &mut impl Read, need_blocks: bool) -> io::Result<GgufMetadata> {
    if int::<4>(reader)? != *b"GGUF" || !(2..=3).contains(&u32_le(reader)?) || u64_le(reader)? == 0
    {
        return Err(invalid());
    }
    let entries = u64_le(reader)?;
    if entries > 16_384 {
        return Err(invalid());
    }
    let mut metadata = GgufMetadata::default();
    let mut block_key = Vec::new();
    for _ in 0..entries {
        let len = u64_le(reader)?;
        if len > 4096 {
            return Err(invalid());
        }
        let mut key = vec![0; len as usize];
        reader.read_exact(&mut key)?;
        let kind = u32_le(reader)?;
        if key == b"general.file_type" || (!block_key.is_empty() && key == block_key) {
            if kind != 4 {
                return Err(invalid());
            }
            let value = u32_le(reader)?;
            if key == b"general.file_type" {
                metadata.file_type = Some(value);
            } else {
                metadata.block_count = (value > 0).then_some(value);
            }
            if metadata.file_type.is_some() && (!need_blocks || metadata.block_count.is_some()) {
                break;
            }
            continue;
        }
        if need_blocks && key == b"general.architecture" && kind == 8 {
            let len = u64_le(reader)?;
            if len > 128 {
                return Err(invalid());
            }
            block_key.resize(len as usize, 0);
            reader.read_exact(&mut block_key)?;
            block_key.extend_from_slice(b".block_count");
            continue;
        }
        let (kind, count) = if kind == 9 {
            (u32_le(reader)?, u64_le(reader)?)
        } else {
            (kind, 1)
        };
        skip_value(reader, kind, count)?;
    }
    Ok(metadata)
}

fn check_gguf(
    path: &Path,
    projector: bool,
    need_blocks: bool,
) -> Result<(u64, Option<u32>), String> {
    let file = File::open(path).map_err(|_| "无法读取 GGUF 文件")?;
    let size = file.metadata().map_err(|_| "无法读取 GGUF 大小")?.len();
    let mut reader = BufReader::new(file).take(64 * 1024 * 1024);
    let metadata =
        gguf_metadata(&mut reader, need_blocks).map_err(|_| "GGUF 文件头无效或版本不支持")?;
    match metadata.file_type {
        Some(0 | 1 | 7 | 18 | 32) => Ok((size, metadata.block_count)),
        None if projector => Ok((size, None)),
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
    vulkan_device: Option<String>,
    vram_budget_mb: Option<u64>,
    cv_vram_reserve_mb: Option<u64>,
}

fn launch(
    bin: &Path,
    model: &Path,
    projector: &Path,
    key: &str,
    gpu: Option<&gpu::Plan>,
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
            "--device",
            gpu.map_or("none", |plan| plan.device.id.as_str()),
            "--n-gpu-layers",
            &gpu.map_or(0, |plan| plan.layers).to_string(),
            "--model",
        ])
        .arg(model)
        .arg("--mmproj")
        .arg(projector);
    if let Some(plan) = gpu {
        cmd.args(["--fit", "off", "--mmproj-device", &plan.device.id]);
    } else {
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

#[derive(Deserialize)]
pub(crate) struct StatusQuery {
    #[serde(default)]
    probe: bool,
}

pub(crate) async fn local_vlm_status(
    State(st): State<Shared>,
    Query(query): Query<StatusQuery>,
) -> Response {
    let status = match st.local_vlm.0.try_lock() {
        Ok(mut state) => state
            .as_mut()
            .and_then(|running| running.child.is_running().then(|| running.status.clone()))
            .map_or_else(|| json!({"status":"stopped"}), |status| json!(status)),
        Err(_) => json!({"status":"starting"}),
    };
    let store = match ls_store::Store::open(&st.db_path) {
        Ok(store) => store,
        Err(_) => return detail_err(StatusCode::INTERNAL_SERVER_ERROR, "无法读取本机视觉设置"),
    };
    let mut status = status;
    status["gpu_settings"] = json!({
        "vulkan_device": gpu::preferred_device(&store),
        "vram_budget_mb": gpu::budget(&store),
        "cv_vram_reserve_mb": gpu::cv_reserve(&store),
    });
    drop(store);
    let bin = binary();
    status["runtime_available"] = json!(bin.is_some());
    status["directory"] = json!("models/local-vlm");
    status["profile_id"] = json!(PROFILE_ID);
    if let Some(bin) = bin.filter(|_| query.probe) {
        match tokio::task::spawn_blocking(move || gpu::devices(&bin, false)).await {
            Ok(Ok(devices)) => status["vulkan_devices"] = json!(devices),
            Ok(Err(error)) => status["device_error"] = json!(error),
            Err(_) => status["device_error"] = json!("无法枚举本机 Vulkan 显卡"),
        }
    }
    Json(status).into_response()
}

pub(crate) async fn local_vlm_start(
    State(st): State<Shared>,
    Json(config): Json<LocalConfig>,
) -> Response {
    match tokio::task::spawn_blocking(move || start(&st, config)).await {
        Ok(Ok(status)) => {
            let mut body = json!(status);
            body["profile_id"] = json!(PROFILE_ID);
            Json(body).into_response()
        }
        Ok(Err(error)) => detail_err(StatusCode::UNPROCESSABLE_ENTITY, &error),
        Err(_) => detail_err(StatusCode::INTERNAL_SERVER_ERROR, "离线视觉服务启动失败"),
    }
}

fn start(st: &Shared, config: LocalConfig) -> Result<LaunchStatus, String> {
    let root = Path::new(&st.data_dir).join("models").join("local-vlm");
    let model = candidate(&config.model, &root)?;
    let projector = candidate(&config.projector, &root)?;
    if model == projector || config.gpu_layers > 128 {
        return Err("权重与投影需要分别选择，GPU 层数须在 0 至 128 之间".into());
    }
    if config
        .vulkan_device
        .as_deref()
        .is_some_and(|v| !gpu::valid_device(v))
        || [config.vram_budget_mb, config.cv_vram_reserve_mb]
            .into_iter()
            .flatten()
            .any(|value| value > gpu::MAX_MEMORY_MB)
    {
        return Err("请选择自动或有效的 Vulkan 设备，并填写有效的显存预算".into());
    }
    let (weights_bytes, block_count) = check_gguf(&model, false, config.gpu_layers > 0)?;
    let (projector_bytes, _) = check_gguf(&projector, true, false)?;
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
    let store = ls_store::Store::open(&st.db_path).map_err(|_| "无法保存本地接入档案")?;
    let device = config
        .vulkan_device
        .unwrap_or_else(|| gpu::preferred_device(&store));
    let budget = config.vram_budget_mb.unwrap_or_else(|| gpu::budget(&store));
    let reserve = config
        .cv_vram_reserve_mb
        .unwrap_or_else(|| gpu::cv_reserve(&store));
    let mut status = LaunchStatus {
        status: "cpu",
        gpu: None,
        fallback_reason: None,
    };
    let child = if config.gpu_layers == 0 {
        launch(&bin, &model, &projector, &key, None)?
    } else {
        status.status = "cpu_fallback";
        match gpu::devices(&bin, true).and_then(|devices| {
            gpu::choose(
                &store,
                &devices,
                &device,
                budget,
                reserve,
                config.gpu_layers,
                gpu::ModelSize {
                    weights_bytes,
                    projector_bytes,
                    block_count,
                },
            )
            .ok_or_else(|| "未找到所选的 Vulkan 显卡".into())
        }) {
            Ok(plan) => status.gpu = Some(plan),
            Err(reason) => status.fallback_reason = Some(reason),
        }
        let gpu_child = status.gpu.as_ref().and_then(|plan| {
            if plan.layers == 0 {
                status.fallback_reason = Some("预留检测显存后，剩余预算不足以加载视觉模型".into());
                return None;
            }
            match launch(&bin, &model, &projector, &key, Some(plan)) {
                Ok(child) => Some(child),
                Err(error) => {
                    status.fallback_reason = Some(error);
                    None
                }
            }
        });
        if let Some(child) = gpu_child {
            status.status = "gpu_requested";
            child
        } else {
            launch(&bin, &model, &projector, &key, None)?
        }
    };
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
        url: profile_url(),
        model: ALIAS.into(),
        api_key: key,
        timeout_s: 30,
        allow_remote: false,
    });
    profiles_store(&store, data, &profiles)?;
    for (key, value) in [
        ("llama.vulkan_device", device),
        ("llama.vram_budget_mb", budget.to_string()),
        ("runtime.cv_vram_reserve_mb", reserve.to_string()),
    ] {
        store
            .setting_set(key, &value)
            .map_err(|err| err.to_string())?;
    }
    tx.commit().map_err(|err| err.to_string())?;
    *active = Some(Running {
        child,
        status: status.clone(),
    });
    crate::detect::reload_verifier(st, &store, &before);
    Ok(status)
}

pub(crate) async fn local_vlm_stop(State(st): State<Shared>) -> Response {
    match tokio::task::spawn_blocking(move || st.local_vlm.stop()).await {
        Ok(()) => Json(json!({"status": "stopped"})).into_response(),
        Err(_) => detail_err(StatusCode::INTERNAL_SERVER_ERROR, "无法停止离线视觉服务"),
    }
}
