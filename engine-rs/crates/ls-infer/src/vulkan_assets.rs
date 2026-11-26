//! Verify an opt-in Windows Vulkan package against the repository asset lock.

use lzma_rust2::XzReader;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

const LOCK: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../scripts/assets.lock.json"
));
const READ_SHARE_ONLY: u32 = 1;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const COAT_HEAD: &[u8] = include_bytes!("../assets/coat-head.xz");
static DLL_CACHE_WRITE: Mutex<()> = Mutex::new(());
static COAT_CACHE_WRITE: Mutex<()> = Mutex::new(());

#[derive(Deserialize)]
struct Asset {
    size: u64,
    sha256: String,
}

#[derive(Deserialize)]
struct ProbeLock {
    dll: Asset,
    sources: BTreeMap<String, Asset>,
    members: BTreeMap<String, Asset>,
}

#[derive(Deserialize)]
struct AssetLock {
    vulkan_probe: ProbeLock,
    vulkan_coat_head: CoatHeadLock,
    assets: BTreeMap<String, Asset>,
}

#[derive(Deserialize)]
struct CoatHeadLock {
    bundle: Asset,
    expanded: Asset,
    param: Asset,
    bin: Asset,
}

/// Hold the read-only file handles through SDK loading. Windows refuses writes and
/// renames of these files until the native loader has finished reading them.
pub(crate) struct VerifiedModel {
    pub dll: PathBuf,
    pub model_base: PathBuf,
    pub coat_head: bool,
    _files: [File; 3],
}

fn checked_bytes(reader: impl Read, asset: &Asset, name: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(asset.size + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{}: {error}", name.display()))?;
    validate_asset(&bytes, asset, name)?;
    Ok(bytes)
}

fn validate_asset(bytes: &[u8], asset: &Asset, name: &Path) -> Result<(), String> {
    let mut actual_hash = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut actual_hash, "{byte:02x}").expect("writing to a string cannot fail");
    }
    if bytes.len() as u64 != asset.size || actual_hash != asset.sha256 {
        return Err(format!("{} 与已锁定的 Vulkan 资产不一致", name.display()));
    }
    Ok(())
}

fn locked_reader(path: &Path) -> Result<File, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(format!("{} 不是普通 Vulkan 资产文件", path.display()));
    }
    OpenOptions::new()
        .read(true)
        .share_mode(READ_SHARE_ONLY)
        .open(path)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn locked_file(path: &Path, asset: &Asset) -> Result<File, String> {
    let mut file = locked_reader(path)?;
    checked_bytes(&mut file, asset, path)?;
    Ok(file)
}

fn publish_cache_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut staged = tempfile::NamedTempFile::new_in(path.parent().expect("cache directory"))
        .map_err(|error| error.to_string())?;
    staged
        .write_all(bytes)
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|error| error.to_string())?;
    match staged.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.error.to_string()),
    }
}

fn materialize_cache(
    lock: &Mutex<()>,
    missing: impl Fn() -> bool,
    write: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if missing() {
        let _guard = lock.lock().map_err(|error| error.to_string())?;
        if missing() {
            write()?;
        }
    }
    Ok(())
}

fn load_dll(root: &Path, cache: &Path, lock: &ProbeLock) -> Result<(PathBuf, File), String> {
    let encoded = root.join("ncnn.dll.xz");
    let bytes = checked_bytes(
        File::open(&encoded).map_err(|error| format!("{}: {error}", encoded.display()))?,
        &lock.members["ncnn.dll.xz"],
        &encoded,
    )?;
    fs::create_dir_all(cache).map_err(|error| error.to_string())?;
    let destination = cache
        .canonicalize()
        .map_err(|error| error.to_string())?
        .join(format!("ncnn-{}.dll", lock.dll.sha256));
    materialize_cache(
        &DLL_CACHE_WRITE,
        || !destination.exists(),
        || {
            let started = Instant::now();
            let decoded = checked_bytes(
                XzReader::new_mem_limit(bytes.as_slice(), false, 64 * 1024),
                &lock.dll,
                &destination,
            )?;
            publish_cache_file(&destination, &decoded)?;
            eprintln!(
                "[ls-infer] materialized Vulkan DLL cache in {} ms",
                started.elapsed().as_millis()
            );
            Ok(())
        },
    )?;
    let guard = locked_file(&destination, &lock.dll)?;
    Ok((destination, guard))
}

/// The patch contains fixed-size records (op, source offset, byte count) plus
/// inline literals. All offsets refer to the exact locked ONNX bytes.
fn rebuild_weights(mut patch: &[u8], source: &[u8], size: usize) -> Result<Vec<u8>, String> {
    let invalid = || "损坏的 Vulkan 白大褂复核权重描述".to_owned();
    let mut result = Vec::with_capacity(size);
    while !patch.is_empty() {
        let (header, rest) = patch.split_at_checked(9).ok_or_else(invalid)?;
        patch = rest;
        let offset = u32::from_le_bytes(header[1..5].try_into().unwrap()) as usize;
        let count = u32::from_le_bytes(header[5..9].try_into().unwrap()) as usize;
        let convert = matches!(header[0], 2 | 3);
        let written = if convert { count / 2 } else { count };
        if count == 0 || written > size - result.len() || (convert && !count.is_multiple_of(4)) {
            return Err(invalid());
        }
        if header[0] == 0 {
            let (literal, rest) = patch.split_at_checked(count).ok_or_else(invalid)?;
            result.extend_from_slice(literal);
            patch = rest;
        } else {
            let data = source
                .get(offset..)
                .and_then(|data| data.get(..count))
                .ok_or_else(invalid)?;
            match header[0] {
                1 => result.extend_from_slice(data),
                2 | 3 => {
                    for &word in data.as_chunks::<4>().0 {
                        let bits = u32::from_le_bytes(word);
                        let half = if header[0] == 2 {
                            half::f16::from_f32(f32::from_bits(bits)).to_bits()
                        } else {
                            ncnn_half_bits(bits)
                        };
                        result.extend_from_slice(&half.to_le_bytes());
                    }
                }
                _ => return Err(invalid()),
            }
        }
    }
    if result.len() != size {
        return Err(invalid());
    }
    Ok(result)
}

// The pinned ncnn writer flushes subnormal values and rounds ties away from zero.
fn ncnn_half_bits(bits: u32) -> u16 {
    let sign = (bits >> 16) as u16 & 0x8000;
    let exponent = ((bits >> 23) & 0xff) as i32 - 112;
    let fraction = bits & 0x7f_ffff;
    let value = match exponent {
        ..=0 => 0,
        143 if fraction != 0 => 0x7e00,
        31.. => 0x7c00,
        _ => ((exponent as u32) << 10) + ((fraction + 0x1000) >> 13),
    };
    sign | value as u16
}

fn coat_files(
    onnx: &Path,
    cache: &Path,
    lock: &AssetLock,
) -> Result<(PathBuf, File, File), String> {
    let mut source = locked_reader(onnx)?;
    let bytes = checked_bytes(&mut source, &lock.assets["labcoat-confirm.onnx"], onnx)?;
    let coat = &lock.vulkan_coat_head;
    fs::create_dir_all(cache).map_err(|error| error.to_string())?;
    let base = cache
        .canonicalize()
        .map_err(|error| error.to_string())?
        .join(format!("labcoat-head-{}", coat.bundle.sha256));
    let param_path = base.with_extension("param");
    let bin_path = base.with_extension("bin");
    materialize_cache(
        &COAT_CACHE_WRITE,
        || !param_path.exists() || !bin_path.exists(),
        || {
            let started = Instant::now();
            validate_asset(COAT_HEAD, &coat.bundle, Path::new("coat-head.xz"))?;
            let expanded = checked_bytes(
                XzReader::new_mem_limit(COAT_HEAD, false, 64 * 1024),
                &coat.expanded,
                Path::new("coat-head.xz"),
            )?;
            let (param, patch) = expanded.split_at(coat.param.size as usize);
            if !param_path.exists() {
                validate_asset(param, &coat.param, &param_path)?;
                publish_cache_file(&param_path, param)?;
            }
            if !bin_path.exists() {
                let weights = rebuild_weights(patch, &bytes, coat.bin.size as usize)?;
                validate_asset(&weights, &coat.bin, &bin_path)?;
                publish_cache_file(&bin_path, &weights)?;
            }
            eprintln!(
                "[ls-infer] materialized Vulkan coat cache in {} ms",
                started.elapsed().as_millis()
            );
            Ok(())
        },
    )?;
    let parameters = locked_file(&param_path, &coat.param)?;
    let weights = locked_file(&bin_path, &coat.bin)?;
    Ok((base, parameters, weights))
}

pub(crate) fn prepare(
    install_root: &Path,
    cache: &Path,
    onnx: &Path,
) -> Result<VerifiedModel, String> {
    let lock = serde_json::from_str::<AssetLock>(LOCK).map_err(|error| error.to_string())?;
    let name = onnx
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("缺少 ONNX 文件名")?;
    let root = install_root
        .join("vulkan")
        .canonicalize()
        .map_err(|error| format!("Vulkan 资产目录: {error}"))?;
    let coat_head = name == "labcoat-confirm.onnx";
    let (model_base, parameters, weights) = if coat_head {
        coat_files(onnx, cache, &lock)?
    } else {
        let base = match name {
            "yolo11n.onnx" => "spike-yolo11n-fp16.ncnn",
            "yolo11n_ppe.onnx" => "spike-yolo11n_ppe-fp16.ncnn",
            _ => return Err(format!("当前 Vulkan 包没有对应的转换模型: {name}")),
        };
        let probe = &lock.vulkan_probe;
        locked_file(onnx, &probe.sources[name])?;
        if name == "yolo11n_ppe.onnx" {
            locked_file(
                &onnx.with_extension("classes.txt"),
                &probe.sources["yolo11n_ppe.classes.txt"],
            )?;
        }
        let param = format!("{base}.param");
        let bin = format!("{base}.bin");
        (
            root.join(base),
            locked_file(&root.join(&param), &probe.members[&param])?,
            locked_file(&root.join(&bin), &probe.members[&bin])?,
        )
    };
    let (dll, dll_guard) = load_dll(&root, cache, &lock.vulkan_probe)?;
    Ok(VerifiedModel {
        dll,
        model_base,
        coat_head,
        _files: [dll_guard, parameters, weights],
    })
}

/// Check all packaged models and the shared DLL without initializing a GPU.
pub fn verify_package(install_root: &Path, cache: &Path) -> Result<(), String> {
    for name in ["yolo11n.onnx", "yolo11n_ppe.onnx", "labcoat-confirm.onnx"] {
        prepare(install_root, cache, &install_root.join("models").join(name))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Barrier;

    #[test]
    fn cold_cache_can_retry_and_only_one_worker_publishes() {
        let lock = Mutex::new(());
        let published = AtomicBool::new(false);
        let attempts = AtomicUsize::new(0);
        let missing = || !published.load(Ordering::Acquire);
        assert!(materialize_cache(&lock, missing, || Err("first write failed".into())).is_err());

        let start = Barrier::new(4);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    start.wait();
                    materialize_cache(&lock, missing, || {
                        attempts.fetch_add(1, Ordering::Relaxed);
                        published.store(true, Ordering::Release);
                        Ok(())
                    })
                    .unwrap();
                });
            }
        });
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
        assert!(!missing());
    }
}
