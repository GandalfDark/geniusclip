//! Hardware load for the in-game menu. CPU and memory come from Windows, GPU
//! load from the "GPU Engine" performance counters (any vendor, like Task
//! Manager), GPU temperature from NVML when an NVIDIA driver is installed.
//! A game's own FPS can't be read without hooking into it, so it isn't shown.

use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::ffi::c_void;
use windows::core::w;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Performance::*;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// 0..100
    cpu: f32,
    ram_used_gb: f64,
    ram_total_gb: f64,
    gpu: Option<f32>,
    gpu_temp: Option<u32>,
}

struct Nvml {
    _lib: libloading::Library,
    temperature: unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> i32,
    shutdown: unsafe extern "C" fn() -> i32,
    device: *mut c_void,
}

impl Drop for Nvml {
    fn drop(&mut self) {
        unsafe {
            (self.shutdown)();
        }
    }
}

impl Nvml {
    fn load() -> Option<Nvml> {
        unsafe {
            let lib = libloading::Library::new("nvml.dll").ok()?;
            let init: libloading::Symbol<unsafe extern "C" fn() -> i32> = lib.get(b"nvmlInit_v2\0").ok()?;
            if init() != 0 {
                return None;
            }
            let by_index: libloading::Symbol<unsafe extern "C" fn(u32, *mut *mut c_void) -> i32> = lib.get(b"nvmlDeviceGetHandleByIndex_v2\0").ok()?;
            let mut device = std::ptr::null_mut();
            if by_index(0, &mut device) != 0 {
                return None;
            }
            let temperature = *lib.get(b"nvmlDeviceGetTemperature\0").ok()?;
            let shutdown = *lib.get(b"nvmlShutdown\0").ok()?;
            Some(Nvml { _lib: lib, temperature, shutdown, device })
        }
    }

    fn temp(&self) -> Option<u32> {
        let mut t = 0;
        // NVML_TEMPERATURE_GPU = 0
        (unsafe { (self.temperature)(self.device, 0, &mut t) } == 0).then_some(t)
    }
}

struct State {
    cpu_prev: Option<(u64, u64)>,
    gpu: Option<(PDH_HQUERY, PDH_HCOUNTER)>,
    nvml: Option<Nvml>,
}

// PDH handles and the NVML device are only used under the mutex.
unsafe impl Send for State {}

impl Drop for State {
    fn drop(&mut self) {
        if let Some((query, _)) = self.gpu.take() {
            unsafe {
                let _ = PdhCloseQuery(query);
            }
        }
    }
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Lets go of the GPU counters and NVML (a driver library with threads of
/// its own) once the in-game menu, the only one asking, is hidden.
pub fn release() {
    STATE.lock().take();
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

fn open_gpu_query() -> Option<(PDH_HQUERY, PDH_HCOUNTER)> {
    unsafe {
        let mut query = PDH_HQUERY::default();
        if PdhOpenQueryW(None, 0, &mut query) != 0 {
            return None;
        }
        let mut counter = PDH_HCOUNTER::default();
        if PdhAddEnglishCounterW(query, w!("\\GPU Engine(*engtype_3D)\\Utilization Percentage"), 0, &mut counter) != 0 {
            let _ = PdhCloseQuery(query);
            return None;
        }
        let _ = PdhCollectQueryData(query);
        Some((query, counter))
    }
}

/// Busiest adapter's 3D engine load, summed over processes.
fn gpu_load(query: PDH_HQUERY, counter: PDH_HCOUNTER) -> Option<f32> {
    unsafe {
        if PdhCollectQueryData(query) != 0 {
            return None;
        }
        let (mut size, mut count) = (0u32, 0u32);
        let _ = PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
        if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) != 0 {
            return None;
        }
        let mut per_adapter: HashMap<String, f64> = HashMap::new();
        for item in std::slice::from_raw_parts(items, count as usize) {
            let name = item.szName.to_string().unwrap_or_default();
            let luid = name.split("_phys").next().unwrap_or("").split("luid_").nth(1).unwrap_or("").to_string();
            *per_adapter.entry(luid).or_default() += item.FmtValue.Anonymous.doubleValue;
        }
        per_adapter.values().cloned().fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v)))).map(|v| v.clamp(0.0, 100.0) as f32)
    }
}

pub fn snapshot() -> Stats {
    let mut guard = STATE.lock();
    let st = guard.get_or_insert_with(|| State { cpu_prev: None, gpu: open_gpu_query(), nvml: Nvml::load() });
    let mut out = Stats::default();

    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    if unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.is_ok() {
        // Kernel time includes idle time.
        let (i, total) = (ft(idle), ft(kernel) + ft(user));
        if let Some((pi, pt)) = st.cpu_prev {
            let dt = total.saturating_sub(pt);
            if dt > 0 {
                out.cpu = (100.0 * (1.0 - i.saturating_sub(pi) as f64 / dt as f64)).clamp(0.0, 100.0) as f32;
            }
        }
        st.cpu_prev = Some((i, total));
    }

    let mut mem = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    if unsafe { GlobalMemoryStatusEx(&mut mem) }.is_ok() {
        let gb = 1024.0 * 1024.0 * 1024.0;
        out.ram_total_gb = mem.ullTotalPhys as f64 / gb;
        out.ram_used_gb = (mem.ullTotalPhys - mem.ullAvailPhys) as f64 / gb;
    }

    out.gpu = st.gpu.and_then(|(q, c)| gpu_load(q, c));
    out.gpu_temp = st.nvml.as_ref().and_then(Nvml::temp);
    out
}
