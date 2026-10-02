//! Hardware load for the in-game menu and the in-game overlay. CPU and
//! memory come from Windows. On NVIDIA the GPU's load, temperature and
//! memory come from NVML (cheap); otherwise load and memory come from the
//! "GPU Engine" / "GPU Adapter Memory" performance counters (any vendor,
//! like Task Manager, but costly to read: they list every process).

use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::ffi::c_void;
use windows::core::w;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Performance::*;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// 0..100
    pub cpu: f32,
    pub ram_used_gb: f64,
    pub ram_total_gb: f64,
    pub gpu: Option<f32>,
    pub gpu_temp: Option<u32>,
    pub vram_used_gb: Option<f64>,
}

/// Who asks for stats: they are let go of once neither does.
#[derive(Clone, Copy)]
pub enum User {
    Menu = 1,
    Overlay = 2,
}

#[repr(C)]
#[derive(Default)]
struct NvmlUtilization {
    gpu: u32,
    memory: u32,
}

#[repr(C)]
#[derive(Default)]
struct NvmlMemory {
    total: u64,
    free: u64,
    used: u64,
}

struct Nvml {
    _lib: libloading::Library,
    temperature: unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> i32,
    utilization: unsafe extern "C" fn(*mut c_void, *mut NvmlUtilization) -> i32,
    memory: unsafe extern "C" fn(*mut c_void, *mut NvmlMemory) -> i32,
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
            let utilization = *lib.get(b"nvmlDeviceGetUtilizationRates\0").ok()?;
            let memory = *lib.get(b"nvmlDeviceGetMemoryInfo\0").ok()?;
            let shutdown = *lib.get(b"nvmlShutdown\0").ok()?;
            Some(Nvml { _lib: lib, temperature, utilization, memory, shutdown, device })
        }
    }

    fn temp(&self) -> Option<u32> {
        let mut t = 0;
        // NVML_TEMPERATURE_GPU = 0
        (unsafe { (self.temperature)(self.device, 0, &mut t) } == 0).then_some(t)
    }

    fn load_pct(&self) -> Option<f32> {
        let mut u = NvmlUtilization::default();
        (unsafe { (self.utilization)(self.device, &mut u) } == 0).then_some(u.gpu as f32)
    }

    fn used_gb(&self) -> Option<f64> {
        let mut m = NvmlMemory::default();
        (unsafe { (self.memory)(self.device, &mut m) } == 0).then(|| m.used as f64 / GB)
    }
}

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

/// GPU load and memory counters (without NVML).
struct GpuCounters {
    query: PDH_HQUERY,
    load: PDH_HCOUNTER,
    memory: Option<PDH_HCOUNTER>,
}

struct State {
    cpu_prev: Option<(u64, u64)>,
    nvml: Option<Nvml>,
    counters: Option<GpuCounters>,
}

// PDH handles and the NVML device are only used under the mutex.
unsafe impl Send for State {}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(c) = self.counters.take() {
            unsafe {
                let _ = PdhCloseQuery(c.query);
            }
        }
    }
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
/// `User` bits of who has asked since they last let go.
static USERS: Mutex<u8> = Mutex::new(0);

/// Lets go of the counters and NVML (a driver library with threads of its
/// own) once nobody shows them any more.
pub fn release(who: User) {
    let mut users = USERS.lock();
    *users &= !(who as u8);
    if *users == 0 {
        STATE.lock().take();
    }
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

fn open_counters() -> Option<GpuCounters> {
    unsafe {
        let mut query = PDH_HQUERY::default();
        if PdhOpenQueryW(None, 0, &mut query) != 0 {
            return None;
        }
        let mut load = PDH_HCOUNTER::default();
        if PdhAddEnglishCounterW(query, w!("\\GPU Engine(*engtype_3D)\\Utilization Percentage"), 0, &mut load) != 0 {
            let _ = PdhCloseQuery(query);
            return None;
        }
        let mut memory = PDH_HCOUNTER::default();
        let memory = (PdhAddEnglishCounterW(query, w!("\\GPU Adapter Memory(*)\\Dedicated Usage"), 0, &mut memory) == 0).then_some(memory);
        let _ = PdhCollectQueryData(query);
        Some(GpuCounters { query, load, memory })
    }
}

/// A counter's values by instance name.
unsafe fn values(counter: PDH_HCOUNTER) -> Option<Vec<(String, f64)>> {
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
    Some(std::slice::from_raw_parts(items, count as usize).iter().map(|i| (i.szName.to_string().unwrap_or_default(), i.FmtValue.Anonymous.doubleValue)).collect())
}

/// The busiest adapter's 3D engine load (summed over processes), and the
/// dedicated memory in use on the adapter that uses the most.
fn read_counters(c: &GpuCounters) -> (Option<f32>, Option<f64>) {
    unsafe {
        if PdhCollectQueryData(c.query) != 0 {
            return (None, None);
        }
        let load = values(c.load).and_then(|v| {
            let mut per_adapter: HashMap<String, f64> = HashMap::new();
            for (name, value) in v {
                let luid = name.split("_phys").next().unwrap_or("").split("luid_").nth(1).unwrap_or("").to_string();
                *per_adapter.entry(luid).or_default() += value;
            }
            per_adapter.into_values().reduce(f64::max).map(|v| v.clamp(0.0, 100.0) as f32)
        });
        let memory = c.memory.and_then(|m| values(m)).and_then(|v| v.into_iter().map(|(_, b)| b).reduce(f64::max)).map(|b| b / GB);
        (load, memory)
    }
}

pub fn snapshot(who: User) -> Stats {
    *USERS.lock() |= who as u8;
    let mut guard = STATE.lock();
    let st = guard.get_or_insert_with(|| {
        let nvml = Nvml::load();
        // NVML has it all on NVIDIA; the counters are the slow way.
        let counters = if nvml.is_some() { None } else { open_counters() };
        State { cpu_prev: None, nvml, counters }
    });
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
        out.ram_total_gb = mem.ullTotalPhys as f64 / GB;
        out.ram_used_gb = (mem.ullTotalPhys - mem.ullAvailPhys) as f64 / GB;
    }

    if let Some(n) = &st.nvml {
        out.gpu = n.load_pct();
        out.gpu_temp = n.temp();
        out.vram_used_gb = n.used_gb();
    } else if let Some(c) = &st.counters {
        (out.gpu, out.vram_used_gb) = read_counters(c);
    }
    out
}
