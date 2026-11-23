use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Adapter {
    name: String,
    pub dxgi_index: Option<i32>,
    vulkan_index: Option<usize>,
    vendor_id: u32,
    device_id: u32,
    device_type: Option<&'static str>,
    local_memory_bytes: u64,
    budget_bytes: Option<u64>,
    usage_bytes: Option<u64>,
    #[cfg(windows)]
    #[serde(skip)]
    luid: Option<[u8; 8]>,
}

pub(crate) fn adapters() -> Vec<Adapter> {
    #[cfg(windows)]
    {
        use std::sync::{Mutex, OnceLock};
        use std::time::{Duration, Instant};
        type AdapterSnapshot = Option<(Instant, Vec<Adapter>)>;
        static CACHE: OnceLock<Mutex<AdapterSnapshot>> = OnceLock::new();
        let mut cache = CACHE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((read_at, adapters)) = cache.as_ref() {
            if read_at.elapsed() < Duration::from_secs(15) {
                return adapters.clone();
            }
        }
        let adapters = windows_gpu::enumerate();
        *cache = Some((Instant::now(), adapters.clone()));
        adapters
    }
    #[cfg(not(windows))]
    Vec::new()
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_gpu {
    use super::Adapter;
    use ash::vk;
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter3, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
        DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
    };

    pub(super) fn enumerate() -> Vec<Adapter> {
        let mut dxgi = dxgi_adapters();
        let vulkan = vulkan_adapters().unwrap_or_default();
        let mut vulkan_only = Vec::new();
        for adapter in vulkan {
            let matched = adapter
                .luid
                .and_then(|luid| dxgi.iter_mut().find(|other| other.luid == Some(luid)));
            if let Some(matched) = matched {
                matched.vulkan_index = adapter.vulkan_index;
                matched.device_type = adapter.device_type;
                matched.budget_bytes = adapter.budget_bytes.or(matched.budget_bytes);
                matched.usage_bytes = adapter.usage_bytes.or(matched.usage_bytes);
                matched.local_memory_bytes =
                    matched.local_memory_bytes.max(adapter.local_memory_bytes);
            } else {
                vulkan_only.push(adapter);
            }
        }
        dxgi.extend(vulkan_only);
        dxgi
    }

    fn dxgi_adapters() -> Vec<Adapter> {
        // DXGI owns the adapter ordering expected by the DirectML device ID.
        let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        for index in 0..32 {
            let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
                break;
            };
            let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
                continue;
            };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let memory = adapter.cast::<IDXGIAdapter3>().ok().and_then(|adapter| {
                let mut info = Default::default();
                // QueryVideoMemoryInfo describes the process's local-segment budget on node zero.
                unsafe {
                    adapter.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info)
                }
                .ok()
                .map(|_| info)
            });
            let mut luid = [0u8; 8];
            luid[..4].copy_from_slice(&desc.AdapterLuid.LowPart.to_le_bytes());
            luid[4..].copy_from_slice(&desc.AdapterLuid.HighPart.to_le_bytes());
            let length = desc
                .Description
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(desc.Description.len());
            result.push(Adapter {
                name: String::from_utf16_lossy(&desc.Description[..length]),
                dxgi_index: Some(index as i32),
                vulkan_index: None,
                vendor_id: desc.VendorId,
                device_id: desc.DeviceId,
                device_type: None,
                local_memory_bytes: desc.DedicatedVideoMemory as u64,
                budget_bytes: memory.map(|info| info.Budget),
                usage_bytes: memory.map(|info| info.CurrentUsage),
                luid: Some(luid),
            });
        }
        result
    }

    fn vulkan_adapters() -> Option<Vec<Adapter>> {
        // Load the driver loader from Windows, never from a working directory or a model path.
        let root = std::env::var_os("SystemRoot")?;
        let path = std::path::PathBuf::from(root)
            .join("System32")
            .join("vulkan-1.dll");
        // The entry owns the module and stays alive until after the instance is destroyed.
        let entry = unsafe { ash::Entry::load_from(path) }.ok()?;
        let highest = unsafe { entry.try_enumerate_instance_version() }
            .ok()?
            .unwrap_or(vk::API_VERSION_1_0);
        let api = highest.min(vk::API_VERSION_1_1);
        let app = vk::ApplicationInfo::default()
            .application_name(c"LabSentinel")
            .api_version(api);
        let create = vk::InstanceCreateInfo::default().application_info(&app);
        let instance = unsafe { entry.create_instance(&create, None) }.ok()?;
        let result = unsafe { instance.enumerate_physical_devices() }
            .ok()
            .map(|devices| {
                devices
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, device)| read_vulkan_device(&instance, api, index, device))
                    .collect()
            });
        unsafe { instance.destroy_instance(None) };
        result
    }

    fn read_vulkan_device(
        instance: &ash::Instance,
        api: u32,
        index: usize,
        device: vk::PhysicalDevice,
    ) -> Option<Adapter> {
        // The device is borrowed from the live instance for every property query.
        let props = unsafe { instance.get_physical_device_properties(device) };
        let device_type = match props.device_type {
            vk::PhysicalDeviceType::DISCRETE_GPU => "discrete",
            vk::PhysicalDeviceType::INTEGRATED_GPU => "integrated",
            vk::PhysicalDeviceType::VIRTUAL_GPU => "virtual",
            vk::PhysicalDeviceType::CPU => return None,
            _ => "other",
        };
        let mut id = vk::PhysicalDeviceIDProperties::default();
        let mut budget = vk::PhysicalDeviceMemoryBudgetPropertiesEXT::default();
        let mut memory = unsafe { instance.get_physical_device_memory_properties(device) };
        let modern = api >= vk::API_VERSION_1_1 && props.api_version >= vk::API_VERSION_1_1;
        let with_budget = modern
            && unsafe { instance.enumerate_device_extension_properties(device) }
                .ok()
                .is_some_and(|extensions| {
                    extensions.iter().any(|ext| {
                        ext.extension_name_as_c_str()
                            .is_ok_and(|name| name == ash::ext::memory_budget::NAME)
                    })
                });
        if modern {
            let mut identity = vk::PhysicalDeviceProperties2::default().push_next(&mut id);
            unsafe { instance.get_physical_device_properties2(device, &mut identity) };
        }
        if with_budget {
            let mut memory_query =
                vk::PhysicalDeviceMemoryProperties2::default().push_next(&mut budget);
            unsafe { instance.get_physical_device_memory_properties2(device, &mut memory_query) };
            memory = memory_query.memory_properties;
        }
        let mut total = 0u64;
        let mut headroom = 0u64;
        let mut usage = 0u64;
        let heap_count = (memory.memory_heap_count as usize).min(memory.memory_heaps.len());
        for (i, heap) in memory.memory_heaps[..heap_count].iter().enumerate() {
            if heap.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL) {
                total = total.saturating_add(heap.size);
                headroom = headroom.saturating_add(budget.heap_budget[i]);
                usage = usage.saturating_add(budget.heap_usage[i]);
            }
        }
        Some(Adapter {
            name: props
                .device_name_as_c_str()
                .ok()?
                .to_string_lossy()
                .into_owned(),
            dxgi_index: None,
            vulkan_index: Some(index),
            vendor_id: props.vendor_id,
            device_id: props.device_id,
            device_type: Some(device_type),
            local_memory_bytes: total,
            budget_bytes: with_budget.then_some(headroom),
            usage_bytes: with_budget.then_some(usage),
            luid: (id.device_luid_valid == vk::TRUE).then_some(id.device_luid),
        })
    }
}
