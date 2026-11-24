//! Windows-only experimental adapter of the ncnn C ABI shipped in the official x64 SDK.
#![allow(unsafe_code)]

use libloading::Library;
use ls_infer::yolo::InferBackend;
use std::ffi::{c_char, c_void};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{self, NonNull};
use std::sync::{Arc, OnceLock};

type Handle = *mut c_void;
type Destroy = unsafe extern "C" fn(Handle);

macro_rules! api {
    ($($name:ident: $ty:ty,)*) => {
        pub struct Api {
            $($name: $ty,)*
            _library: Library,
        }
        impl Api {
            pub fn load(path: &Path) -> Result<Arc<Self>, String> {
                // ncnn retains process-wide Vulkan resources after dropping the nets.
                // Keep its code mapped through Windows process teardown.
                static API: OnceLock<Result<Arc<Api>, String>> = OnceLock::new();
                API.get_or_init(|| unsafe {
                    let library = Library::new(path).map_err(|error| error.to_string())?;
                    Ok(Arc::new(Self {
                        $($name: *library.get::<$ty>(stringify!($name).as_bytes())
                            .map_err(|error| format!("{}: {error}", stringify!($name)))?,)*
                        _library: library,
                    }))
                }).clone()
            }
        }
    };
}

api! {
    ncnn_net_create: unsafe extern "C" fn() -> Handle,
    ncnn_net_destroy: Destroy,
    ncnn_net_get_option: unsafe extern "C" fn(Handle) -> Handle,
    ncnn_option_set_use_vulkan_compute: unsafe extern "C" fn(Handle, i32),
    ncnn_option_set_use_fp16_packed: unsafe extern "C" fn(Handle, i32),
    ncnn_option_set_use_fp16_storage: unsafe extern "C" fn(Handle, i32),
    ncnn_option_set_use_fp16_arithmetic: unsafe extern "C" fn(Handle, i32),
    ncnn_option_set_num_threads: unsafe extern "C" fn(Handle, i32),
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

struct OwnedHandle(NonNull<c_void>, Destroy);

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
}

// A net moves to a single benchmark worker. It never shares extractors or matrices.
unsafe impl Send for Model {}

impl Model {
    pub fn load(api: Arc<Api>, base: &Path, gpu: i32, fp16: bool) -> Result<Self, String> {
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
            (api.ncnn_option_set_use_fp16_packed)(opt, i32::from(fp16));
            (api.ncnn_option_set_use_fp16_storage)(opt, i32::from(fp16));
            (api.ncnn_option_set_use_fp16_arithmetic)(opt, i32::from(fp16));
            (api.ncnn_option_set_num_threads)(opt, 2);
            (api.ncnn_net_set_vulkan_device)(net.ptr(), gpu);
            for (load, suffix) in [
                (api.ncnn_net_load_param_w, ".param"),
                (api.ncnn_net_load_model_w, ".bin"),
            ] {
                let status = load(net.ptr(), wide(suffix).as_ptr());
                if status != 0 {
                    return Err(format!("failed loading {base:?}{suffix}: {status}"));
                }
            }
            Ok(Self { net, api })
        }
    }
}

impl InferBackend for Model {
    fn run(
        &mut self,
        mut data: Vec<f32>,
        width: u32,
        height: u32,
    ) -> Result<(Vec<usize>, Vec<f32>), String> {
        if width != 640 || height != 640 || data.len() != 3 * 640 * 640 {
            return Err("ncnn benchmark requires one 640×640 NCHW tensor".into());
        }
        let api = &self.api;
        // The external matrix borrows data until both it and the extractor are freed.
        // The checked FP32 2D output is copied before ncnn can release it.
        unsafe {
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
            Ok((vec![1, height, width], values))
        }
    }
}
