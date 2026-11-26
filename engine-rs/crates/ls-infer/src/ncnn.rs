//! Adapter for the pinned Windows x64 ncnn SDK used by the probe and the gray-test loader.
#![allow(unsafe_code)]

use crate::yolo::{record_vulkan_inference, InferBackend, VulkanAdapter};
use ash::vk;
use libloading::os::windows::{
    Library, LOAD_LIBRARY_SEARCH_APPLICATION_DIR, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
    LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{self, NonNull};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

mod cache_file;

type Handle = *mut c_void;
type Destroy = unsafe extern "C" fn(Handle);

static LOADED_API: Mutex<Option<(PathBuf, Arc<Api>)>> = Mutex::new(None);

struct DeviceIdentity {
    luid: Option<[u8; 8]>,
    uuid: [u8; 16],
}

macro_rules! api {
    ($($name:ident: $ty:ty,)*) => {
        pub struct Api {
            $($name: $ty,)*
            get_gpu_count: unsafe extern "C" fn() -> i32,
            get_default_gpu_index: unsafe extern "C" fn() -> i32,
            get_gpu_info: unsafe extern "C" fn(i32) -> Handle,
            broken_pipeline_cache: unsafe extern "C" fn(Handle) -> bool,
            device_identities: OnceLock<Vec<Option<DeviceIdentity>>>,
            pipeline_caches: Mutex<HashMap<i32, CachedPipelines>>,
            library_path: PathBuf,
            _library: Library,
        }
        impl Api {
            pub fn load(path: &Path) -> Result<Arc<Self>, String> {
                // ncnn retains process-wide Vulkan resources after dropping the nets.
                // Keep its code mapped through Windows process teardown.
                let path = path.canonicalize().map_err(|error| error.to_string())?;
                let mut cached = LOADED_API.lock().map_err(|error| error.to_string())?;
                if let Some((active_path, api)) = cached.as_ref() {
                    if active_path != &path {
                        return Err("a different ncnn library is already loaded".into());
                    }
                    return Ok(Arc::clone(api));
                }
                let api = unsafe {
                    let library = Library::load_with_flags(
                        &path,
                        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_APPLICATION_DIR
                            | LOAD_LIBRARY_SEARCH_SYSTEM32,
                    ).map_err(|error| error.to_string())?;
                    Arc::new(Self {
                        $($name: *library.get::<$ty>(stringify!($name).as_bytes())
                            .map_err(|error| format!("{}: {error}", stringify!($name)))?,)*
                        // The pinned Windows SDK exports this argument-free MSVC symbol.
                        // Its C API setter otherwise falls back to the default device for bad indices.
                        get_gpu_count: *library.get::<unsafe extern "C" fn() -> i32>(b"?get_gpu_count@ncnn@@YAHXZ\0")
                            .map_err(|error| format!("ncnn GPU enumeration unavailable: {error}"))?,
                        get_default_gpu_index: *library.get::<unsafe extern "C" fn() -> i32>(b"?get_default_gpu_index@ncnn@@YAHXZ\0")
                            .map_err(|error| format!("ncnn default GPU unavailable: {error}"))?,
                        get_gpu_info: *library.get::<unsafe extern "C" fn(i32) -> Handle>(b"?get_gpu_info@ncnn@@YAAEBVGpuInfo@1@H@Z\0")
                            .map_err(|error| format!("ncnn GPU info unavailable: {error}"))?,
                        broken_pipeline_cache: *library.get::<unsafe extern "C" fn(Handle) -> bool>(b"?bug_corrupted_online_pipeline_cache@GpuInfo@ncnn@@QEBA_NXZ\0")
                            .map_err(|error| format!("ncnn cache compatibility unavailable: {error}"))?,
                        device_identities: OnceLock::new(),
                        pipeline_caches: Mutex::new(HashMap::new()),
                        library_path: path.clone(),
                        _library: library,
                    })
                };
                *cached = Some((path, Arc::clone(&api)));
                Ok(api)
            }
        }
    };
}

api! {
    ncnn_net_create: unsafe extern "C" fn() -> Handle,
    ncnn_net_destroy: Destroy,
    ncnn_net_get_option: unsafe extern "C" fn(Handle) -> Handle,
    ncnn_option_set_use_vulkan_compute: unsafe extern "C" fn(Handle, i32),
    ncnn_option_get_use_vulkan_compute: unsafe extern "C" fn(Handle) -> i32,
    ncnn_option_set_use_fp16_packed: unsafe extern "C" fn(Handle, i32),
    ncnn_option_set_use_fp16_storage: unsafe extern "C" fn(Handle, i32),
    ncnn_option_set_use_fp16_arithmetic: unsafe extern "C" fn(Handle, i32),
    ncnn_option_set_num_threads: unsafe extern "C" fn(Handle, i32),
    ncnn_option_set_pipeline_cache: unsafe extern "C" fn(Handle, Handle),
    ncnn_pipelinecache_create: unsafe extern "C" fn(i32) -> Handle,
    ncnn_pipelinecache_destroy: Destroy,
    ncnn_pipelinecache_get_size: unsafe extern "C" fn(Handle) -> usize,
    ncnn_pipelinecache_load_memory: unsafe extern "C" fn(Handle, *const u8, usize) -> i32,
    ncnn_pipelinecache_save_memory: unsafe extern "C" fn(Handle, *mut u8, *mut usize) -> i32,
    ncnn_net_set_vulkan_device: unsafe extern "C" fn(Handle, i32),
    ncnn_net_load_param_w: unsafe extern "C" fn(Handle, *const u16) -> i32,
    ncnn_net_load_model_w: unsafe extern "C" fn(Handle, *const u16) -> i32,
    ncnn_mat_create_external_3d: unsafe extern "C" fn(i32, i32, i32, Handle, Handle) -> Handle,
    ncnn_mat_destroy: Destroy,
    ncnn_mat_get_dims: unsafe extern "C" fn(Handle) -> i32,
    ncnn_mat_get_w: unsafe extern "C" fn(Handle) -> i32,
    ncnn_mat_get_h: unsafe extern "C" fn(Handle) -> i32,
    ncnn_mat_get_elempack: unsafe extern "C" fn(Handle) -> i32,
    ncnn_mat_get_elemsize: unsafe extern "C" fn(Handle) -> usize,
    ncnn_mat_get_data: unsafe extern "C" fn(Handle) -> Handle,
    ncnn_extractor_create: unsafe extern "C" fn(Handle) -> Handle,
    ncnn_extractor_destroy: Destroy,
    ncnn_extractor_input: unsafe extern "C" fn(Handle, *const c_char, Handle) -> i32,
    ncnn_extractor_extract: unsafe extern "C" fn(Handle, *const c_char, *mut Handle) -> i32,
}

impl Api {
    /// The selected device's Windows identity, if CV already loaded the SDK.
    pub fn loaded_adapter_luid(selection: VulkanAdapter) -> Option<[u8; 8]> {
        let api = Arc::clone(&LOADED_API.lock().ok()?.as_ref()?.1);
        let index = api.resolve_adapter(selection).ok()?;
        api.identities().get(index as usize)?.as_ref()?.luid
    }

    pub(crate) fn resolve_adapter(&self, selection: VulkanAdapter) -> Result<i32, String> {
        let index = match selection {
            VulkanAdapter::Auto => self.default_adapter(),
            VulkanAdapter::Index(index) => index,
            VulkanAdapter::Luid(_) | VulkanAdapter::Uuid(_) => {
                let mut matches = self
                    .identities()
                    .iter()
                    .enumerate()
                    .filter(|(_, candidate)| {
                        candidate.as_ref().is_some_and(|id| match selection {
                            VulkanAdapter::Luid(luid) => id.luid == Some(luid),
                            VulkanAdapter::Uuid(uuid) => uuid != [0; 16] && id.uuid == uuid,
                            _ => false,
                        })
                    });
                let (index, _) = matches
                    .next()
                    .filter(|_| matches.next().is_none())
                    .ok_or_else(|| {
                        format!(
                            "ncnn Vulkan device {} is unavailable or ambiguous",
                            selection.id()
                        )
                    })?;
                index as i32
            }
        };
        self.validate_adapter(index).map(|_| index)
    }

    fn identities(&self) -> &[Option<DeviceIdentity>] {
        self.device_identities
            .get_or_init(|| self.read_identities().unwrap_or_default())
    }

    fn read_identities(&self) -> Option<Vec<Option<DeviceIdentity>>> {
        let dll = PathBuf::from(std::env::var_os("SystemRoot")?).join("System32/vulkan-1.dll");
        // ncnn owns these handles. Query through its instance with the system
        // loader and never destroy the instance or retain a borrowed handle.
        unsafe {
            let count = (self.get_gpu_count)();
            if count <= 0 {
                return None;
            }
            let entry = ash::Entry::load_from(dll).ok()?;
            let instance = self
                ._library
                .get::<unsafe extern "C" fn() -> vk::Instance>(
                    b"?get_gpu_instance@ncnn@@YAPEAUVkInstance_T@@XZ\0",
                )
                .ok()?();
            if instance == vk::Instance::null() {
                return None;
            }
            let address = entry
                .get_instance_proc_addr(instance, c"vkGetPhysicalDeviceProperties2".as_ptr())
                .or_else(|| {
                    entry.get_instance_proc_addr(
                        instance,
                        c"vkGetPhysicalDeviceProperties2KHR".as_ptr(),
                    )
                })?;
            let properties = std::mem::transmute::<
                unsafe extern "system" fn(),
                vk::PFN_vkGetPhysicalDeviceProperties2,
            >(address);
            let get_device = self
                ._library
                .get::<unsafe extern "C" fn(Handle) -> vk::PhysicalDevice>(
                    b"?physicalDevice@GpuInfo@ncnn@@QEBAPEAUVkPhysicalDevice_T@@XZ\0",
                )
                .ok()?;
            Some(
                (0..count)
                    .map(|index| {
                        let info = (self.get_gpu_info)(index);
                        if info.is_null() {
                            return None;
                        }
                        let device = get_device(info);
                        if device == vk::PhysicalDevice::null() {
                            return None;
                        }
                        let mut identity = vk::PhysicalDeviceIDProperties::default();
                        let mut query =
                            vk::PhysicalDeviceProperties2::default().push_next(&mut identity);
                        properties(device, &mut query);
                        Some(DeviceIdentity {
                            luid: (identity.device_luid_valid == vk::TRUE
                                && identity.device_luid != [0; 8])
                                .then_some(identity.device_luid),
                            uuid: identity.device_uuid,
                        })
                    })
                    .collect(),
            )
        }
    }

    pub(crate) fn default_adapter(&self) -> i32 {
        // The pinned library owns the device ordering and is kept loaded.
        unsafe { (self.get_default_gpu_index)() }
    }

    pub fn validate_adapter(&self, index: i32) -> Result<i32, String> {
        // The DLL remains loaded and the function takes no pointers or arguments.
        let count = unsafe { (self.get_gpu_count)() };
        if !(0..count).contains(&index) {
            return Err(format!(
                "ncnn Vulkan adapter {index} is unavailable ({count} devices)"
            ));
        }
        Ok(count)
    }

    fn pipeline_cache(&self, gpu: i32) -> Option<Handle> {
        let mut caches = self.pipeline_caches.lock().ok()?;
        if let Some(cache) = caches.get(&gpu) {
            return Some(cache.handle.ptr());
        }
        self.validate_adapter(gpu).ok()?;
        // The checked device index refers to the pinned SDK's process-wide device.
        let cache = unsafe {
            let info = (self.get_gpu_info)(gpu);
            if info.is_null() || (self.broken_pipeline_cache)(info) {
                return None;
            }
            OwnedHandle::new(
                (self.ncnn_pipelinecache_create)(gpu),
                self.ncnn_pipelinecache_destroy,
            )
            .ok()?
        };
        let handle = cache.ptr();
        let mut name = self.library_path.file_stem()?.to_owned();
        name.push(format!("-pipelines-{gpu}.dpapi"));
        let path = self.library_path.with_file_name(name);
        let mut fingerprint = None;
        let started = Instant::now();
        match cache_file::read(&path) {
            Ok(Some(data)) => {
                // The bounded, authenticated blob lives through the SDK's internal copy.
                if unsafe {
                    (self.ncnn_pipelinecache_load_memory)(handle, data.as_ptr(), data.len())
                } == 0
                {
                    fingerprint = Some(cache_file::fingerprint(&data));
                    eprintln!(
                        "[ls-infer] restored Vulkan pipelines on GPU {gpu}: {} bytes in {} ms",
                        data.len(),
                        started.elapsed().as_millis()
                    );
                } else {
                    eprintln!("[ls-infer] Vulkan pipeline cache incompatible on GPU {gpu}");
                }
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("[ls-infer] ignored Vulkan pipeline cache on GPU {gpu}: {error}")
            }
        }
        caches.insert(
            gpu,
            CachedPipelines {
                handle: cache,
                path,
                fingerprint,
            },
        );
        Some(handle)
    }

    fn save_pipeline_cache(&self, gpu: i32) {
        let Ok(mut caches) = self.pipeline_caches.lock() else {
            return;
        };
        let Some(cache) = caches.get_mut(&gpu) else {
            return;
        };
        let handle = cache.handle.ptr();
        let started = Instant::now();
        let mut size = 0;
        // The pinned API reports the buffer size when called with no output buffer.
        unsafe { (self.ncnn_pipelinecache_save_memory)(handle, ptr::null_mut(), &mut size) };
        if !(1..=cache_file::LIMIT).contains(&size) {
            return;
        }
        let mut data = vec![0; size];
        let status =
            unsafe { (self.ncnn_pipelinecache_save_memory)(handle, data.as_mut_ptr(), &mut size) };
        if status != 0 || size > data.len() {
            return;
        }
        data.truncate(size);
        let fingerprint = cache_file::fingerprint(&data);
        if cache.fingerprint == Some(fingerprint) {
            return;
        }
        match cache_file::write(&cache.path, &data, fingerprint) {
            Ok(()) => {
                cache.fingerprint = Some(fingerprint);
                eprintln!(
                    "[ls-infer] saved Vulkan pipelines on GPU {gpu}: {size} bytes in {} ms",
                    started.elapsed().as_millis()
                );
            }
            Err(error) => {
                eprintln!("[ls-infer] could not save Vulkan pipelines on GPU {gpu}: {error}")
            }
        }
    }
}

struct OwnedHandle(NonNull<c_void>, Destroy);

struct CachedPipelines {
    handle: OwnedHandle,
    path: PathBuf,
    fingerprint: Option<[u8; 32]>,
}

// The API owns the cache until all borrowing models release their Arc<Api>.
// ncnn locks its pipeline creation and lookup, and no Rust caller clears the cache.
unsafe impl Send for CachedPipelines {}

impl OwnedHandle {
    fn new(handle: Handle, destroy: Destroy) -> Result<Self, String> {
        NonNull::new(handle)
            .map(|ptr| Self(ptr, destroy))
            .ok_or_else(|| "ncnn returned a null handle".into())
    }

    fn ptr(&self) -> Handle {
        self.0.as_ptr()
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // Each handle is owned exactly once and dropped before the library.
        unsafe { (self.1)(self.ptr()) }
    }
}

pub struct Model {
    net: OwnedHandle,
    api: Arc<Api>,
    has_run: bool,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub enum Precision {
    Fp32,
    Fp16Storage,
    Fp16,
}

impl Precision {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fp32 => "ncnn/Vulkan FP32",
            Self::Fp16Storage => "ncnn/Vulkan FP16 storage, FP32 arithmetic",
            Self::Fp16 => "ncnn/Vulkan FP16",
        }
    }
}

// The net can move between threads. Its native calls require exclusive access.
unsafe impl Send for Model {}

impl Model {
    pub fn load(
        api: Arc<Api>,
        base: &Path,
        gpu: i32,
        precision: Precision,
    ) -> Result<Self, String> {
        Self::load_inner(api, base, gpu, precision, false)
    }

    fn load_inner(
        api: Arc<Api>,
        base: &Path,
        gpu: i32,
        precision: Precision,
        shared: bool,
    ) -> Result<Self, String> {
        let setup_started = Instant::now();
        let cache = shared.then(|| api.pipeline_cache(gpu)).flatten();
        let wide = |suffix| {
            let mut filename = base.as_os_str().to_owned();
            filename.push(suffix);
            filename.encode_wide().chain([0]).collect::<Vec<_>>()
        };
        // Net options are borrowed from the corresponding live net. The checked UTF-16
        // paths outlive their loading calls and work in a checkout with Chinese characters.
        unsafe {
            let net = OwnedHandle::new((api.ncnn_net_create)(), api.ncnn_net_destroy)?;
            let opt = (api.ncnn_net_get_option)(net.ptr());
            if opt.is_null() {
                return Err("ncnn returned no net options".into());
            }
            (api.ncnn_option_set_use_vulkan_compute)(opt, 1);
            let storage = i32::from(!matches!(precision, Precision::Fp32));
            (api.ncnn_option_set_use_fp16_packed)(opt, storage);
            (api.ncnn_option_set_use_fp16_storage)(opt, storage);
            (api.ncnn_option_set_use_fp16_arithmetic)(
                opt,
                i32::from(matches!(precision, Precision::Fp16)),
            );
            (api.ncnn_option_set_num_threads)(opt, 2);
            (api.ncnn_net_set_vulkan_device)(net.ptr(), gpu);
            if let Some(cache) = cache {
                (api.ncnn_option_set_pipeline_cache)(opt, cache);
            }
            let entries_before = cache.map(|cache| (api.ncnn_pipelinecache_get_size)(cache));
            let setup = setup_started.elapsed();
            let mut stages = [Duration::ZERO; 2];
            for (index, (load, suffix)) in [
                (api.ncnn_net_load_param_w, ".param"),
                (api.ncnn_net_load_model_w, ".bin"),
            ]
            .into_iter()
            .enumerate()
            {
                let started = Instant::now();
                let status = load(net.ptr(), wide(suffix).as_ptr());
                stages[index] = started.elapsed();
                if status != 0 {
                    return Err(format!("failed loading {base:?}{suffix}: {status}"));
                }
            }
            if (api.ncnn_option_get_use_vulkan_compute)(opt) == 0 {
                return Err("ncnn disabled Vulkan during model initialization".into());
            }
            if shared {
                let entries_after = cache.map(|cache| (api.ncnn_pipelinecache_get_size)(cache));
                eprintln!("[ls-infer] Vulkan load stages {} on GPU {gpu}: setup={} ms param={} ms model={} ms pipelines={entries_before:?}->{entries_after:?}",
                          base.file_name().unwrap_or(base.as_os_str()).to_string_lossy(),
                          setup.as_millis(), stages[0].as_millis(), stages[1].as_millis());
            }
            Ok(Self {
                net,
                api,
                has_run: false,
            })
        }
    }
    fn run_after_wait(
        &mut self,
        mut data: Vec<f32>,
        width: u32,
        height: u32,
        wait: Duration,
    ) -> Result<(Vec<usize>, Vec<f32>), String> {
        if width != 640 || height != 640 || data.len() != 3 * 640 * 640 {
            return Err("ncnn requires one 640×640 NCHW tensor".into());
        }
        let run_started = Instant::now();
        let api = &self.api;
        // The external matrix borrows data until both it and the extractor are freed.
        // The checked FP32 2D output is copied before ncnn can release it.
        let result = unsafe {
            let input = OwnedHandle::new(
                (api.ncnn_mat_create_external_3d)(
                    640,
                    640,
                    3,
                    data.as_mut_ptr().cast(),
                    ptr::null_mut(),
                ),
                api.ncnn_mat_destroy,
            )?;
            let extractor = OwnedHandle::new(
                (api.ncnn_extractor_create)(self.net.ptr()),
                api.ncnn_extractor_destroy,
            )?;
            let status = (api.ncnn_extractor_input)(extractor.ptr(), c"in0".as_ptr(), input.ptr());
            if status != 0 {
                return Err(format!("ncnn rejected input in0: {status}"));
            }
            let mut result = ptr::null_mut();
            let status =
                (api.ncnn_extractor_extract)(extractor.ptr(), c"out0".as_ptr(), &mut result);
            let output = OwnedHandle::new(result, api.ncnn_mat_destroy)?;
            let p = output.ptr();
            let width = (api.ncnn_mat_get_w)(p);
            let height = (api.ncnn_mat_get_h)(p);
            let values = (api.ncnn_mat_get_data)(p).cast::<f32>();
            if status != 0
                || (api.ncnn_mat_get_dims)(p) != 2
                || width != 8_400
                || !(5..=84).contains(&height)
                || (api.ncnn_mat_get_elempack)(p) != 1
                || (api.ncnn_mat_get_elemsize)(p) != 4
                || values.is_null()
            {
                return Err(format!(
                    "unsupported ncnn output: {width}×{height}, status {status}"
                ));
            }
            let (width, height) = (width as usize, height as usize);
            let values = std::slice::from_raw_parts(values, width * height).to_vec();
            (vec![1, height, width], values)
        };
        record_vulkan_inference(run_started.elapsed(), wait, !self.has_run);
        self.has_run = true;
        Ok(result)
    }
}

impl InferBackend for Model {
    fn run(
        &mut self,
        data: Vec<f32>,
        width: u32,
        height: u32,
    ) -> Result<(Vec<usize>, Vec<f32>), String> {
        self.run_after_wait(data, width, height, Duration::ZERO)
    }
}

pub(crate) struct SharedModel(Arc<Mutex<Model>>);

type ModelKey = (PathBuf, i32, Precision);
type ModelCache = Mutex<HashMap<ModelKey, Weak<Mutex<Model>>>>;
static MODELS: OnceLock<ModelCache> = OnceLock::new();

impl SharedModel {
    pub(crate) fn load(
        api: Arc<Api>,
        base: &Path,
        gpu: i32,
        precision: Precision,
    ) -> Result<Self, String> {
        // The verified package provides a canonical base and one pinned SDK per process.
        let key = (base.to_owned(), gpu, precision);
        let mut models = MODELS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .map_err(|error| error.to_string())?;
        if let Some(model) = models.get(&key).and_then(Weak::upgrade) {
            return Ok(Self(model));
        }
        models.retain(|_, model| model.strong_count() != 0);
        let started = Instant::now();
        let model = Arc::new(Mutex::new(Model::load_inner(
            Arc::clone(&api),
            base,
            gpu,
            precision,
            true,
        )?));
        eprintln!(
            "[ls-infer] initialized Vulkan model {} on GPU {gpu} in {} ms",
            base.file_name()
                .unwrap_or(base.as_os_str())
                .to_string_lossy(),
            started.elapsed().as_millis()
        );
        api.save_pipeline_cache(gpu);
        models.insert(key, Arc::downgrade(&model));
        Ok(Self(model))
    }
}

impl InferBackend for SharedModel {
    fn run(
        &mut self,
        data: Vec<f32>,
        width: u32,
        height: u32,
    ) -> Result<(Vec<usize>, Vec<f32>), String> {
        // ncnn's buffers include mutable Vulkan access state alongside their contents.
        // Cover the full native call, including extractor destruction.
        let waiting_since = Instant::now();
        let mut model = self.0.lock().map_err(|error| error.to_string())?;
        model.run_after_wait(data, width, height, waiting_since.elapsed())
    }
}

pub(crate) struct CoatHead(pub SharedModel);

impl InferBackend for CoatHead {
    fn run(
        &mut self,
        data: Vec<f32>,
        width: u32,
        height: u32,
    ) -> Result<(Vec<usize>, Vec<f32>), String> {
        let (shape, raw) = self.0.run(data, width, height)?;
        if shape != [1, 8, 8400] {
            return Err(format!("白大褂复核预测头维度异常: {shape:?}"));
        }
        Ok((vec![1, 300, 6], coat_topk(&raw)?))
    }
}

/// Reproduce the packaged ONNX postprocessing over the four-class raw head.
/// First rank anchors by their best class, then rank anchor/class pairs.
fn coat_topk(raw: &[f32]) -> Result<Vec<f32>, String> {
    const ANCHORS: usize = 8400;
    const CLASSES: usize = 4;
    const LIMIT: usize = 300;
    let scores = &raw[4 * ANCHORS..];
    if scores.iter().any(|score| !score.is_finite()) {
        return Err("白大褂复核预测分数异常".into());
    }
    let rank = |a: &(usize, f32), b: &(usize, f32)| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0));
    let mut anchors = (0..ANCHORS)
        .map(|index| {
            let best = (0..CLASSES)
                .map(|class| scores[class * ANCHORS + index])
                .fold(f32::NEG_INFINITY, f32::max);
            (index, best)
        })
        .collect::<Vec<_>>();
    anchors.select_nth_unstable_by(LIMIT, rank);
    anchors.truncate(LIMIT);
    anchors.sort_unstable_by(rank);

    let mut pairs = anchors
        .iter()
        .enumerate()
        .flat_map(|(ranked, &(anchor, _))| {
            (0..CLASSES)
                .map(move |class| (ranked * CLASSES + class, scores[class * ANCHORS + anchor]))
        })
        .collect::<Vec<_>>();
    pairs.select_nth_unstable_by(LIMIT, rank);
    pairs.truncate(LIMIT);
    pairs.sort_unstable_by(rank);
    let mut result = Vec::with_capacity(LIMIT * 6);
    for (pair, score) in pairs {
        let anchor = anchors[pair / CLASSES].0;
        result.extend((0..4).map(|coord| raw[coord * ANCHORS + anchor]));
        result.extend([score, (pair % CLASSES) as f32]);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a locked package, coat fixtures and physical Windows Vulkan GPU"]
    fn coat_head_preserves_packaged_detections() {
        use crate::yolo::{Backend, VulkanCv, YoloModel};

        let root = PathBuf::from(std::env::var_os("LS_TEST_VULKAN_PACKAGE").unwrap());
        let fixtures = PathBuf::from(std::env::var_os("LS_TEST_COAT_FIXTURES").unwrap());
        let cache = tempfile::tempdir().unwrap();
        let onnx = root.join("models/labcoat-confirm.onnx");
        let mut reference =
            YoloModel::load(&onnx, 640, 0.55, 0.5, 0.0, Some(Backend::Ort), false).unwrap();
        let mut actual = YoloModel::load_vulkan(
            &onnx,
            &root,
            cache.path(),
            VulkanAdapter::Auto,
            0.55,
            0.5,
            0.0,
        )
        .unwrap();
        assert_eq!(actual.device().label(), "Vulkan(GPU)");
        let specs = (0..4).map(|class| (class, 0.55, 0.0)).collect::<Vec<_>>();
        let mut images = 0;
        let mut checked = 0;
        for frame in std::fs::read_dir(fixtures).unwrap() {
            let path = frame.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "bgr") {
                continue;
            }
            let bgr = std::fs::read(&path).unwrap();
            assert_eq!(bgr.len(), 640 * 640 * 3);
            let expected = reference.detect_classes(&bgr, 640, 640, &specs).unwrap();
            let result = actual.detect_classes(&bgr, 640, 640, &specs).unwrap();
            for (class, (expected, mut observed)) in expected.into_iter().zip(result).enumerate() {
                assert_eq!(expected.len(), observed.len(), "{path:?}, class {class}");
                for expected in expected {
                    let index = observed
                        .iter()
                        .position(|det| {
                            crate::post::iou(&expected.box_, &det.box_) >= 0.95
                                && (expected.score - det.score).abs() <= 0.02
                        })
                        .unwrap_or_else(|| {
                            panic!("{path:?}, class {class}: no match for {expected:?}")
                        });
                    observed.swap_remove(index);
                    checked += 1;
                }
            }
            images += 1;
        }
        assert!(images >= 8 && checked >= 4);
        eprintln!("coat Vulkan parity: {checked} matched detections across {images} frames");

        drop(actual);
        let weights = std::fs::read_dir(cache.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "bin"))
            .expect("reconstructed coat weights");
        std::fs::write(weights, b"corrupt cache").unwrap();
        let config = VulkanCv {
            install_root: root,
            cache: cache.path().to_owned(),
            adapter: VulkanAdapter::Auto,
        };
        let fallback = YoloModel::load_cv(&onnx, 640, 0.55, 0.5, 0.0, true, Some(&config)).unwrap();
        assert!(matches!(fallback.device().label(), "DirectML(GPU)" | "CPU"));
    }

    #[test]
    #[ignore = "requires a verified Windows Vulkan package and physical GPU"]
    fn cameras_share_a_net_until_the_last_camera_stops() {
        let root = PathBuf::from(std::env::var_os("LS_TEST_VULKAN_PACKAGE").unwrap());
        let cache = tempfile::tempdir().unwrap();
        let onnx = root.join("models/yolo11n.onnx");
        let files = crate::vulkan_assets::prepare(&root, cache.path(), &onnx).unwrap();
        let api = Api::load(&files.dll).unwrap();
        let gpu = api.resolve_adapter(VulkanAdapter::Auto).unwrap();
        let precision = Precision::Fp16Storage;
        let mut independent =
            Model::load(Arc::clone(&api), &files.model_base, gpu, precision).unwrap();
        let load =
            || SharedModel::load(Arc::clone(&api), &files.model_base, gpu, precision).unwrap();
        let mut first = load();
        let mut second = load();
        assert!(Arc::ptr_eq(&first.0, &second.0));
        let net = Arc::downgrade(&first.0);

        let make_input = |fill| vec![fill; 3 * 640 * 640];
        let expected_a = independent.run(make_input(0.0), 640, 640).unwrap();
        let expected_b = independent.run(make_input(0.75), 640, 640).unwrap();
        assert_ne!(expected_a.1, expected_b.1);
        let barrier = std::sync::Barrier::new(2);
        let (a, b) = std::thread::scope(|scope| {
            let other = &barrier;
            let first_call = scope.spawn(move || {
                other.wait();
                first.run(make_input(0.0), 640, 640).unwrap()
            });
            barrier.wait();
            let b = second.run(make_input(0.75), 640, 640).unwrap();
            (first_call.join().unwrap(), b)
        });
        assert_eq!(a, expected_a);
        assert_eq!(b, expected_b);
        assert!(net.upgrade().is_some());
        drop(second);
        assert!(net.upgrade().is_none());
        let pipelines = api.pipeline_cache(gpu).map(|cache| {
            let size = unsafe { (api.ncnn_pipelinecache_get_size)(cache) };
            assert!(size > 0);
            (cache, size)
        });
        let mut reloaded = load();
        assert_eq!(reloaded.run(make_input(0.0), 640, 640).unwrap(), expected_a);
        if let Some((cache, size)) = pipelines {
            assert_eq!(api.pipeline_cache(gpu), Some(cache));
            assert_eq!(unsafe { (api.ncnn_pipelinecache_get_size)(cache) }, size);
        }
    }
}
