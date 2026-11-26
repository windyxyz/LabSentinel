use std::{
    io::{Read, Seek},
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    time::{Duration, Instant},
};

use ls_ingest::child_guard::ChildGuard;
use ls_store::Store;
use serde::Serialize;

use crate::{detect, gpu_enum};

const MIB: u64 = 1024 * 1024;
const RUNTIME_MARGIN_MB: u64 = 512;
const DEFAULT_CV_RESERVE_MB: u64 = 1024;
pub(super) const MAX_MEMORY_MB: u64 = 1_048_576;

#[derive(Clone, Serialize)]
pub(super) struct Device {
    pub(super) id: String,
    name: String,
    total_mb: u64,
    free_mb: u64,
}

pub(super) fn valid_device(value: &str) -> bool {
    value == "auto"
        || value.strip_prefix("Vulkan").is_some_and(|number| {
            number
                .parse::<u32>()
                .is_ok_and(|id| id.to_string() == number)
        })
}

pub(super) fn cv_reserve(store: &Store) -> u64 {
    memory_setting(store, "runtime.cv_vram_reserve_mb", DEFAULT_CV_RESERVE_MB)
}

pub(super) fn budget(store: &Store) -> u64 {
    memory_setting(store, "llama.vram_budget_mb", 0)
}

pub(super) fn preferred_device(store: &Store) -> String {
    store
        .setting_get("llama.vulkan_device")
        .ok()
        .flatten()
        .filter(|device| valid_device(device))
        .unwrap_or_else(|| "auto".into())
}

fn memory_setting(store: &Store, key: &str, default: u64) -> u64 {
    store
        .setting_get(key)
        .ok()
        .flatten()
        .and_then(|setting| setting.parse::<u64>().ok())
        .filter(|setting| *setting <= MAX_MEMORY_MB)
        .unwrap_or(default)
}

fn parse_devices(output: &str) -> Vec<Device> {
    output
        .lines()
        .filter_map(|line| {
            let (id, details) = line.trim().split_once(": ")?;
            if id == "auto" || !valid_device(id) {
                return None;
            }
            let (name, memory) = details.rsplit_once(" (")?;
            let (total, free) = memory.strip_suffix(" MiB free)")?.split_once(" MiB, ")?;
            let total_mb = total.parse::<u64>().ok()?;
            let free_mb = free.parse::<u64>().ok()?.min(total_mb);
            Some(Device {
                id: id.into(),
                name: name.into(),
                total_mb,
                free_mb,
            })
        })
        .collect()
}

fn read_devices(bin: &Path) -> Result<Vec<Device>, String> {
    let mut cmd = Command::new(bin);
    cmd.current_dir(bin.parent().ok_or("找不到离线服务目录")?)
        .arg("--list-devices");
    capture_devices(cmd)
}

fn capture_devices(cmd: Command) -> Result<Vec<Device>, String> {
    let mut output = tempfile::tempfile().map_err(|_| "无法打开显卡枚举临时文件")?;
    let mut child = ChildGuard::spawn_with_stdout(
        cmd,
        output
            .try_clone()
            .map_err(|_| "无法读取显卡枚举输出")?
            .into(),
    )?;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        match child.exit_status() {
            Ok(Some(status)) if status.success() => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => return Err("无法列出本机视觉运行时的 Vulkan 显卡".into()),
        }
    }
    let mut bytes = Vec::new();
    output.rewind().map_err(|_| "无法读取显卡枚举输出")?;
    output
        .take(64 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取显卡枚举输出")?;
    Ok(parse_devices(&String::from_utf8_lossy(&bytes)))
}

pub(super) fn devices(bin: &Path, refresh: bool) -> Result<Vec<Device>, String> {
    type Snapshot = (Instant, PathBuf, Result<Vec<Device>, String>);
    static CACHE: Mutex<Option<Snapshot>> = Mutex::new(None);
    let mut cache = if refresh {
        CACHE.lock().map_err(|_| "无法读取显卡枚举状态")?
    } else {
        CACHE.try_lock().map_err(|_| "正在枚举 Vulkan 显卡")?
    };
    if !refresh {
        if let Some((at, path, devices)) = cache.as_ref() {
            if path == bin && at.elapsed() < Duration::from_secs(10) {
                return devices.clone();
            }
        }
    }
    let result = read_devices(bin);
    *cache = Some((Instant::now(), bin.into(), result.clone()));
    result
}

#[derive(Clone, Serialize)]
pub(super) struct Plan {
    pub(super) device: Device,
    pub(super) layers: u32,
    pub(super) cv_reserve_mb: u64,
    usable_mb: u64,
}

#[derive(Clone, Copy)]
pub(super) struct ModelSize {
    pub(super) weights_bytes: u64,
    pub(super) projector_bytes: u64,
    pub(super) block_count: Option<u32>,
}

impl Plan {
    fn for_device(
        device: &Device,
        user_budget: u64,
        cv_reserve_mb: u64,
        layers: u32,
        model: ModelSize,
    ) -> Self {
        let available = device.free_mb.saturating_sub(cv_reserve_mb);
        let usable_mb = if user_budget == 0 {
            available
        } else {
            available.min(user_budget)
        };
        // The projector and workspace/KV headroom are accounted for separately.
        let overhead = model
            .projector_bytes
            .div_ceil(MIB)
            .saturating_add(RUNTIME_MARGIN_MB);
        let for_weights = usable_mb.saturating_sub(overhead).saturating_mul(MIB);
        let affordable = if for_weights >= model.weights_bytes {
            model
                .block_count
                .map_or(layers, |blocks| layers.min(blocks.saturating_add(1)))
        } else if let Some(blocks) = model.block_count {
            // llama.cpp loads the final blocks first. Use a margin over the
            // average and exclude the output layer unless the full file fits.
            let per_layer = model
                .weights_bytes
                .saturating_mul(5)
                .div_ceil(u64::from(blocks) * 4)
                .max(1);
            (for_weights / per_layer).min(u64::from(layers.min(blocks))) as u32
        } else {
            0
        };
        Self {
            device: device.clone(),
            layers: affordable,
            cv_reserve_mb,
            usable_mb,
        }
    }
}

pub(super) fn choose(
    store: &Store,
    devices: &[Device],
    preferred: &str,
    user_budget: u64,
    cv_reserve_mb: u64,
    layers: u32,
    model: ModelSize,
) -> Option<Plan> {
    let backend = detect::runtime_backend(store);
    let adapters = if backend == "cpu" || cv_reserve_mb == 0 {
        Vec::new()
    } else {
        gpu_enum::adapters()
    };
    #[cfg(all(windows, feature = "vulkan-cv"))]
    let ncnn = detect::vulkan_requested(&backend)
        .then(|| ls_infer::ncnn::Api::loaded_adapter_luid(detect::runtime_vulkan_adapter(store)))
        .flatten();
    #[cfg(not(all(windows, feature = "vulkan-cv")))]
    let ncnn: Option<[u8; 8]> = None;
    let ncnn = ncnn.and_then(|luid| gpu_enum::by_luid(&adapters, luid));
    let dml = detect::runtime_adapter(store);
    devices
        .iter()
        .filter(|device| preferred == "auto" || device.id == preferred)
        .map(|device| {
            let shared = backend != "cpu"
                && !gpu_enum::unique_vulkan_adapter(&adapters, &device.name).is_some_and(
                    |adapter| {
                        let dml_separate = gpu_enum::distinct_dxgi(&adapters, adapter, dml);
                        dml_separate
                            && (!detect::vulkan_requested(&backend)
                                || ncnn.is_some_and(|cv| !std::ptr::eq(cv, adapter)))
                    },
                );
            let reserve = if shared { cv_reserve_mb } else { 0 };
            Plan::for_device(device, user_budget, reserve, layers, model)
        })
        .max_by_key(|plan| (plan.layers, plan.cv_reserve_mb == 0, plan.usable_mb))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: ModelSize = ModelSize {
        weights_bytes: 1024 * MIB,
        projector_bytes: 64 * MIB,
        block_count: Some(20),
    };

    #[test]
    fn parses_only_llama_vulkan_ids_and_pinned_output() {
        let devices = parse_devices(
            "Available devices:\n  Vulkan0: Radeon (TM) 880M (4096 MiB, 3072 MiB free)\n\
             CUDA0: Other (4096 MiB, 4096 MiB free)\n  Vulkan01: invalid (400 MiB, 300 MiB free)\n",
        );
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "Radeon (TM) 880M");
        assert_eq!(devices[0].free_mb, 3072);
        assert!(!valid_device("Vulkan0,Vulkan1"));
        assert!(!valid_device("none"));
    }

    #[test]
    #[cfg(windows)]
    fn guarded_process_captures_device_list() {
        let mut command = Command::new("cmd.exe");
        command.args([
            "/d",
            "/c",
            "echo Vulkan2: Synthetic GPU (8192 MiB, 6144 MiB free)",
        ]);
        let devices = capture_devices(command).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "Vulkan2");
        assert_eq!(devices[0].free_mb, 6144);
    }

    #[test]
    fn budgets_cap_layers_after_cv_reserve_and_workspace() {
        let device = parse_devices("Vulkan0: GPU (4096 MiB, 2048 MiB free)").remove(0);
        let full = Plan::for_device(&device, 0, 0, 99, MODEL);
        assert_eq!((full.layers, full.usable_mb), (21, 2048));
        let shared = Plan::for_device(&device, 0, 1024, 99, MODEL);
        assert_eq!((shared.layers, shared.usable_mb), (7, 1024));
        assert_eq!(Plan::for_device(&device, 650, 1024, 99, MODEL).layers, 1);
        assert_eq!(Plan::for_device(&device, 512, 1024, 99, MODEL).layers, 0);
        assert_eq!(Plan::for_device(&device, 0, 4096, 99, MODEL).layers, 0);
        assert_eq!(Plan::for_device(&device, 0, 0, 6, MODEL).layers, 6);
    }
}
