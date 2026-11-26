use ls_infer::yolo::VulkanAdapter;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Adapter {
    name: String,
    pub dxgi_index: Option<i32>,
    vulkan_index: Option<usize>,
    gpu_id: Option<String>,
    #[serde(skip)]
    vulkan_name: Option<String>,
    vendor_id: u32,
    device_id: u32,
    device_type: Option<&'static str>,
    local_memory_bytes: u64,
    budget_bytes: Option<u64>,
    usage_bytes: Option<u64>,
    #[serde(skip)]
    luid: Option<[u8; 8]>,
    #[serde(skip)]
    uuid: Option<[u8; 16]>,
}

pub(crate) fn by_luid(adapters: &[Adapter], luid: [u8; 8]) -> Option<&Adapter> {
    let mut matches = adapters.iter().filter(|adapter| adapter.luid == Some(luid));
    let found = matches.next()?;
    matches.next().is_none().then_some(found)
}

pub(crate) fn by_physical_id(adapters: &[Adapter], id: VulkanAdapter) -> Option<&Adapter> {
    match id {
        VulkanAdapter::Luid(luid) => by_luid(adapters, luid),
        VulkanAdapter::Uuid(uuid) => {
            let mut matches = adapters.iter().filter(|adapter| adapter.uuid == Some(uuid));
            let found = matches.next()?;
            matches.next().is_none().then_some(found)
        }
        _ => None,
    }
}

pub(crate) fn persistable_id(adapters: &[Adapter], selection: VulkanAdapter) -> VulkanAdapter {
    if let VulkanAdapter::Luid(_) = selection {
        if let Some(uuid) = by_physical_id(adapters, selection).and_then(|adapter| adapter.uuid) {
            let id = VulkanAdapter::Uuid(uuid);
            if by_physical_id(adapters, id).is_some() {
                return id;
            }
        }
    }
    selection
}

/// Map device descriptions only when exactly one physical adapter matches.
pub(crate) fn unique_vulkan_adapter<'a>(
    adapters: &'a [Adapter],
    name: &str,
) -> Option<&'a Adapter> {
    let mut matches = adapters.iter().filter(|adapter| {
        adapter
            .vulkan_name
            .as_deref()
            .is_some_and(|candidate| candidate.trim().eq_ignore_ascii_case(name.trim()))
    });
    let found = matches.next()?;
    matches.next().is_none().then_some(found)
}

pub(crate) fn distinct_dxgi(adapters: &[Adapter], first: &Adapter, second: Option<i32>) -> bool {
    first.dxgi_index.zip(second).is_some_and(|(first, second)| {
        first != second
            && adapters
                .iter()
                .any(|adapter| adapter.dxgi_index == Some(second))
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(luid: [u8; 8], vulkan_index: usize, dxgi_index: i32) -> Adapter {
        Adapter {
            name: "Same GPU name".into(),
            dxgi_index: Some(dxgi_index),
            vulkan_index: Some(vulkan_index),
            gpu_id: None,
            vulkan_name: Some("Same GPU name".into()),
            vendor_id: 1,
            device_id: 2,
            device_type: None,
            local_memory_bytes: 0,
            budget_bytes: None,
            usage_bytes: None,
            luid: Some(luid),
            uuid: None,
        }
    }

    #[test]
    fn physical_id_resolves_dxgi_when_vulkan_order_differs() {
        let mut adapters = vec![adapter([1; 8], 2, 0), adapter([2; 8], 0, 3)];
        assert_eq!(by_luid(&adapters, [2; 8]).unwrap().dxgi_index, Some(3));
        assert!(by_luid(&adapters, [3; 8]).is_none());
        assert!(unique_vulkan_adapter(&adapters, "Same GPU name").is_none());
        adapters.push(adapter([2; 8], 1, 4));
        assert!(by_luid(&adapters, [2; 8]).is_none());
    }

    #[test]
    fn stable_identity_survives_luid_and_index_changes_without_guessing() {
        let uuid = [9; 16];
        let original = Adapter {
            uuid: Some(uuid),
            ..adapter([1; 8], 0, 0)
        };
        let legacy = VulkanAdapter::Luid([1; 8]);
        let saved = persistable_id(&[original], legacy);
        assert_eq!(saved, VulkanAdapter::Uuid(uuid));
        assert_eq!(VulkanAdapter::parse(&saved.id()), Some(saved));

        let same_card = Adapter {
            uuid: Some(uuid),
            ..adapter([2; 8], 1, 3)
        };
        let other_card = Adapter {
            uuid: Some([8; 16]),
            ..adapter([3; 8], 0, 0)
        };
        let mut after_restart = vec![other_card, same_card];
        assert!(by_physical_id(&after_restart, legacy).is_none());
        assert_eq!(persistable_id(&after_restart, legacy), legacy);
        assert_eq!(
            by_physical_id(&after_restart, saved).unwrap().dxgi_index,
            Some(3)
        );
        assert!(by_physical_id(&after_restart[..1], saved).is_none());

        after_restart.push(Adapter {
            uuid: Some(uuid),
            ..adapter([4; 8], 2, 4)
        });
        assert!(by_physical_id(&after_restart, saved).is_none());
        let live_luid = VulkanAdapter::Luid([2; 8]);
        assert_eq!(persistable_id(&after_restart, live_luid), live_luid);
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_gpu {
    use super::{by_physical_id, Adapter};
    use ash::vk;
    use ls_infer::yolo::VulkanAdapter;
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter3, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
        DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
    };

    pub(super) fn enumerate() -> Vec<Adapter> {
        let mut dxgi = dxgi_adapters();
        let mut vulkan = vulkan_adapters().unwrap_or_default();
        let mut vulkan_only = Vec::new();
        while let Some(adapter) = vulkan.pop() {
            let matched = adapter.luid.and_then(|luid| {
                // Several Vulkan nodes may share the same linked DXGI adapter LUID.
                if vulkan
                    .iter()
                    .chain(vulkan_only.iter())
                    .any(|other: &Adapter| other.luid == Some(luid))
                {
                    return None;
                }
                let index = super::by_luid(&dxgi, luid)?.dxgi_index?;
                dxgi.iter_mut()
                    .find(|other| other.dxgi_index == Some(index) && other.vulkan_index.is_none())
            });
            if let Some(matched) = matched {
                matched.vulkan_index = adapter.vulkan_index;
                matched.vulkan_name = adapter.vulkan_name;
                matched.uuid = adapter.uuid;
                matched.device_type = adapter.device_type;
                matched.budget_bytes = adapter.budget_bytes.or(matched.budget_bytes);
                matched.usage_bytes = adapter.usage_bytes.or(matched.usage_bytes);
                matched.local_memory_bytes =
                    matched.local_memory_bytes.max(adapter.local_memory_bytes);
            } else {
                vulkan_only.push(adapter);
            }
        }
        vulkan_only.reverse();
        dxgi.extend(vulkan_only);
        let ids = dxgi
            .iter()
            .map(|adapter| {
                adapter.vulkan_index?;
                adapter
                    .uuid
                    .map(VulkanAdapter::Uuid)
                    .filter(|id| by_physical_id(&dxgi, *id).is_some())
                    .or_else(|| {
                        adapter
                            .luid
                            .map(VulkanAdapter::Luid)
                            .filter(|id| by_physical_id(&dxgi, *id).is_some())
                    })
                    .map(VulkanAdapter::id)
            })
            .collect::<Vec<_>>();
        for (adapter, id) in dxgi.iter_mut().zip(ids) {
            adapter.gpu_id = id;
        }
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
                gpu_id: None,
                vulkan_name: None,
                vendor_id: desc.VendorId,
                device_id: desc.DeviceId,
                device_type: None,
                local_memory_bytes: desc.DedicatedVideoMemory as u64,
                budget_bytes: memory.map(|info| info.Budget),
                usage_bytes: memory.map(|info| info.CurrentUsage),
                luid: (luid != [0; 8]).then_some(luid),
                uuid: None,
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
        let name = props
            .device_name_as_c_str()
            .ok()?
            .to_string_lossy()
            .into_owned();
        let luid = (id.device_luid_valid == vk::TRUE && id.device_luid != [0; 8])
            .then_some(id.device_luid);
        Some(Adapter {
            vulkan_name: Some(name.clone()),
            name,
            dxgi_index: None,
            vulkan_index: Some(index),
            gpu_id: None,
            vendor_id: props.vendor_id,
            device_id: props.device_id,
            device_type: Some(device_type),
            local_memory_bytes: total,
            budget_bytes: with_budget.then_some(headroom),
            usage_bytes: with_budget.then_some(usage),
            luid,
            uuid: (id.device_uuid != [0; 16]).then_some(id.device_uuid),
        })
    }
}
