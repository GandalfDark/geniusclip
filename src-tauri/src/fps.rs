//! A game's frame rate from Windows' own graphics events (ETW), the way Xbox
//! Game Bar and PresentMon read it: nothing is injected into the game or
//! read from its memory. Every frame a game shows passes through a Present
//! call, which the DirectX runtimes (DXGI, D3D9) and the graphics kernel
//! (Vulkan, OpenGL) report.
//!
//! Windows lets only the "Performance Log Users" group (or an elevated
//! process) start such a session: Settings asks once to add the user to it
//! (`grant`), which takes effect at the next sign-in.

use parking_lot::Mutex;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows::core::{w, BOOL, GUID, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_CANCELLED, ERROR_SUCCESS, HANDLE, WAIT_OBJECT_0, WIN32_ERROR};
use windows::Win32::NetworkManagement::NetManagement::{NetApiBufferFree, NetUserGetLocalGroups, LG_INCLUDE_INDIRECT, LOCALGROUP_USERS_INFO_0, MAX_PREFERRED_LENGTH};
use windows::Win32::Security::Authentication::Identity::{GetUserNameExW, NameSamCompatible};
use windows::Win32::Security::{CheckTokenMembership, CreateWellKnownSid, LookupAccountSidW, WinBuiltinAdministratorsSid, WinBuiltinPerfLoggingUsersSid, PSID, SID_NAME_USE, WELL_KNOWN_SID_TYPE};
use windows::Win32::System::Diagnostics::Etw::*;
use windows::Win32::System::Performance::QueryPerformanceFrequency;
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

const SESSION: PCWSTR = w!("GeniusClip frame rate");
const DXGI: GUID = GUID::from_u128(0xca11c036_0102_4a2d_a6ad_f03cfed5d3c9);
const D3D9: GUID = GUID::from_u128(0x783aca0a_790e_4d7f_8451_aa850511c6b9);
const DXGKRNL: GUID = GUID::from_u128(0x802ec45a_1e99_4b83_9920_87c98277ba9d);
/// Present_Start of DXGI (Direct3D 10-12) and D3D9.
const DXGI_PRESENT: u16 = 42;
const D3D9_PRESENT: u16 = 1;
/// The kernel's present events (Present, PresentHistory_Start,
/// PresentHistoryDetailed_Start): Vulkan and OpenGL games, whose frames
/// skip the DirectX runtimes. Counted per kind, so one present that logs
/// two of them is not counted twice.
const KERNEL_PRESENTS: [u16; 3] = [184, 171, 215];
/// Frame times kept for the 1% low.
const HISTORY: Duration = Duration::from_secs(10);

/// What the overlay shows.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rate {
    pub fps: f32,
    /// Average frame time over the last second, ms.
    pub frame_ms: f32,
    /// The frame rate of the slowest 1% of frames over the last 10 s.
    pub low_1: f32,
}

/// Present times (QPC) of the followed process, by source.
struct Frames {
    pid: u32,
    api: VecDeque<i64>,
    kernel: [VecDeque<i64>; 3],
}

static FRAMES: Mutex<Frames> = Mutex::new(Frames { pid: 0, api: VecDeque::new(), kernel: [VecDeque::new(), VecDeque::new(), VecDeque::new()] });

struct Session {
    control: CONTROLTRACE_HANDLE,
    thread: std::thread::JoinHandle<()>,
}

static SESSION_STATE: Mutex<Option<Session>> = Mutex::new(None);
/// The last start failed for lack of access (shown in Settings).
static DENIED: AtomicBool = AtomicBool::new(false);

fn qpc_freq() -> i64 {
    let mut f = 0i64;
    let _ = unsafe { QueryPerformanceFrequency(&mut f) };
    f.max(1)
}

unsafe extern "system" fn on_event(rec: *mut EVENT_RECORD) {
    let h = &(*rec).EventHeader;
    let id = h.EventDescriptor.Id;
    let mut f = FRAMES.lock();
    if h.ProcessId != f.pid || f.pid == 0 {
        return;
    }
    let t = h.TimeStamp;
    if (h.ProviderId == DXGI && id == DXGI_PRESENT) || (h.ProviderId == D3D9 && id == D3D9_PRESENT) {
        f.api.push_back(t);
    } else if h.ProviderId == DXGKRNL {
        if let Some(k) = KERNEL_PRESENTS.iter().position(|&e| e == id) {
            f.kernel[k].push_back(t);
        }
    }
}

/// The properties block: EVENT_TRACE_PROPERTIES followed by room for the
/// session name, which Windows writes there.
fn properties() -> Vec<u64> {
    let size = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() + 1024;
    let mut buf = vec![0u64; size.div_ceil(8)];
    unsafe {
        let p = buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
        (*p).Wnode.BufferSize = size as u32;
        (*p).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        // Timestamps in QPC ticks.
        (*p).Wnode.ClientContext = 1;
        (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        (*p).BufferSize = 64;
        (*p).MinimumBuffers = 4;
        (*p).MaximumBuffers = 16;
        // Events reach us within a second at most.
        (*p).FlushTimer = 1;
        (*p).LoggerNameOffset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
    }
    buf
}

/// Enables a provider for only the listed events (and, for the user-mode
/// runtimes, only the followed process: filtered where they are logged).
unsafe fn enable(control: CONTROLTRACE_HANDLE, provider: &GUID, keywords: u64, ids: &[u16], pid: Option<u32>) -> WIN32_ERROR {
    // EVENT_FILTER_EVENT_ID with `ids.len()` entries.
    let mut ev = vec![0u8; 4 + 2 * ids.len()];
    ev[0] = 1; // FilterIn
    ev[2..4].copy_from_slice(&(ids.len() as u16).to_le_bytes());
    for (i, id) in ids.iter().enumerate() {
        ev[4 + 2 * i..6 + 2 * i].copy_from_slice(&id.to_le_bytes());
    }
    let pid_buf = pid.map(|p| p.to_le_bytes());
    let mut filters = vec![EVENT_FILTER_DESCRIPTOR { Ptr: ev.as_ptr() as u64, Size: ev.len() as u32, Type: EVENT_FILTER_TYPE_EVENT_ID }];
    if let Some(p) = &pid_buf {
        filters.push(EVENT_FILTER_DESCRIPTOR { Ptr: p.as_ptr() as u64, Size: 4, Type: EVENT_FILTER_TYPE_PID });
    }
    let params = ENABLE_TRACE_PARAMETERS {
        Version: ENABLE_TRACE_PARAMETERS_VERSION_2,
        EnableFilterDesc: filters.as_mut_ptr(),
        FilterDescCount: filters.len() as u32,
        ..Default::default()
    };
    EnableTraceEx2(control, provider, EVENT_CONTROL_CODE_ENABLE_PROVIDER.0, TRACE_LEVEL_INFORMATION as u8, keywords, 0, 0, Some(&params))
}

/// Stops a session by name (ours, left over by a crash).
unsafe fn stop_named() {
    let mut props = properties();
    let _ = ControlTraceW(CONTROLTRACE_HANDLE::default(), SESSION, props.as_mut_ptr() as *mut _, EVENT_TRACE_CONTROL_STOP);
}

/// Follows the frames of `pid`, starting the session if needed. Err when
/// Windows refuses it (no access) or it can't start.
pub fn follow(pid: u32) -> anyhow::Result<()> {
    {
        let mut f = FRAMES.lock();
        if f.pid != pid {
            f.pid = pid;
            f.api.clear();
            f.kernel.iter_mut().for_each(VecDeque::clear);
        }
    }
    let mut state = SESSION_STATE.lock();
    // The runtimes filter by process where they log: a new game needs the
    // session started again.
    if state.as_ref().is_some_and(|s| !s.thread.is_finished()) && FOLLOWED.load(Ordering::Relaxed) == pid {
        return Ok(());
    }
    if let Some(s) = state.take() {
        close(s);
    }
    let s = unsafe { start(pid) }?;
    FOLLOWED.store(pid, Ordering::Relaxed);
    *state = Some(s);
    Ok(())
}

static FOLLOWED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

unsafe fn start(pid: u32) -> anyhow::Result<Session> {
    let mut control = CONTROLTRACE_HANDLE::default();
    let mut props = properties();
    let mut err = StartTraceW(&mut control, SESSION, props.as_mut_ptr() as *mut _);
    if err == ERROR_ALREADY_EXISTS {
        stop_named();
        props = properties();
        err = StartTraceW(&mut control, SESSION, props.as_mut_ptr() as *mut _);
    }
    DENIED.store(err == ERROR_ACCESS_DENIED, Ordering::Relaxed);
    if err != ERROR_SUCCESS {
        anyhow::bail!("StartTrace: {err:?}");
    }
    // Keywords as PresentMon enables them: the runtimes' "Events" (+ their
    // analytic channel bit), the kernel's "Base".
    let user = 0x8000_0000_0000_0002;
    for (provider, keywords, ids, filter) in [
        (&DXGI, user, &[DXGI_PRESENT][..], Some(pid)),
        (&D3D9, user, &[D3D9_PRESENT][..], Some(pid)),
        (&DXGKRNL, 0x1, &KERNEL_PRESENTS[..], None),
    ] {
        let e = enable(control, provider, keywords, ids, filter);
        if e != ERROR_SUCCESS {
            log::warn!("fps: enabling a provider failed: {e:?}");
        }
    }
    let thread = std::thread::Builder::new().name("gc-fps".into()).spawn(|| unsafe {
        let mut name: Vec<u16> = "GeniusClip frame rate\0".encode_utf16().collect();
        let mut logfile = EVENT_TRACE_LOGFILEW { LoggerName: PWSTR(name.as_mut_ptr()), ..Default::default() };
        logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        logfile.Anonymous2.EventRecordCallback = Some(on_event);
        let h = OpenTraceW(&mut logfile);
        if h.Value == u64::MAX {
            log::warn!("fps: OpenTrace failed");
            return;
        }
        // Returns once the session is stopped.
        let _ = ProcessTrace(&[h], None, None);
        let _ = CloseTrace(h);
    })?;
    log::info!("fps: following process {pid}");
    Ok(Session { control, thread })
}

fn close(s: Session) {
    unsafe {
        let mut props = properties();
        let _ = ControlTraceW(s.control, PCWSTR::null(), props.as_mut_ptr() as *mut _, EVENT_TRACE_CONTROL_STOP);
    }
    let _ = s.thread.join();
}

/// Stops following (no game in front, overlay off).
pub fn stop() {
    if let Some(s) = SESSION_STATE.lock().take() {
        close(s);
        log::info!("fps: stopped");
    }
    let mut f = FRAMES.lock();
    f.pid = 0;
    f.api.clear();
    f.kernel.iter_mut().for_each(VecDeque::clear);
    FOLLOWED.store(0, Ordering::Relaxed);
}

/// The followed game's frame rate now (None before any frame).
pub fn rate() -> Option<Rate> {
    let freq = qpc_freq();
    let mut now = 0i64;
    let _ = unsafe { windows::Win32::System::Performance::QueryPerformanceCounter(&mut now) };
    let keep = HISTORY.as_secs() as i64 * freq;
    let mut guard = FRAMES.lock();
    let f = &mut *guard;
    for q in std::iter::once(&mut f.api).chain(f.kernel.iter_mut()) {
        while q.front().is_some_and(|&t| now - t > keep) {
            q.pop_front();
        }
    }
    // The runtimes' count when the game uses them, else the kernel's.
    let times = if !f.api.is_empty() { &f.api } else { f.kernel.iter().max_by_key(|q| q.len())? };
    rate_of(times, now, freq)
}

fn rate_of(times: &VecDeque<i64>, now: i64, freq: i64) -> Option<Rate> {
    let last = *times.back()?;
    // Events come in batches up to a second late (the session's flush
    // timer), so the second measured is the one before the newest frame.
    // None for longer: the game is paused or loading.
    if now - last > freq * 5 / 2 {
        return Some(Rate::default());
    }
    let second: Vec<i64> = times.iter().copied().filter(|&t| last - t <= freq).collect();
    if second.len() < 2 {
        return Some(Rate::default());
    }
    let span = (second[second.len() - 1] - second[0]) as f32 / freq as f32;
    let fps = (second.len() - 1) as f32 / span.max(1e-6);
    let mut gaps: Vec<i64> = times.iter().zip(times.iter().skip(1)).map(|(a, b)| b - a).collect();
    gaps.sort_unstable();
    let p99 = gaps[(gaps.len() * 99 / 100).min(gaps.len() - 1)] as f32 / freq as f32;
    Some(Rate { fps, frame_ms: 1000.0 / fps.max(1e-3), low_1: 1.0 / p99.max(1e-6) })
}

/// Frame times (ms) of the last `n` frames, oldest first: the panel's graph.
pub fn frame_times(n: usize) -> Vec<f32> {
    let freq = qpc_freq() as f32;
    let f = FRAMES.lock();
    let times = if !f.api.is_empty() { &f.api } else { f.kernel.iter().max_by_key(|q| q.len()).unwrap_or(&f.api) };
    let skip = times.len().saturating_sub(n + 1);
    times.iter().skip(skip).zip(times.iter().skip(skip + 1)).map(|(a, b)| (b - a) as f32 * 1000.0 / freq).collect()
}

// ---------------------------------------------------------------------------
// Access

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Access {
    /// Sessions can be started.
    Granted,
    /// In the group, but this sign-in's token predates it.
    SignInAgain,
    Needed,
}

fn sid(kind: WELL_KNOWN_SID_TYPE) -> Vec<u8> {
    let mut buf = vec![0u8; 68];
    let mut size = buf.len() as u32;
    let _ = unsafe { CreateWellKnownSid(kind, None, Some(PSID(buf.as_mut_ptr() as _)), &mut size) };
    buf
}

fn token_has(kind: WELL_KNOWN_SID_TYPE) -> bool {
    let mut s = sid(kind);
    let mut member = BOOL(0);
    unsafe { CheckTokenMembership(None, PSID(s.as_mut_ptr() as _), &mut member) }.is_ok() && member.as_bool()
}

/// The Performance Log Users group's name in this Windows' language.
fn group_name() -> Option<String> {
    let mut s = sid(WinBuiltinPerfLoggingUsersSid);
    let (mut name, mut domain) = ([0u16; 256], [0u16; 256]);
    let (mut n, mut d) = (name.len() as u32, domain.len() as u32);
    let mut kind = SID_NAME_USE::default();
    unsafe { LookupAccountSidW(PCWSTR::null(), PSID(s.as_mut_ptr() as _), Some(PWSTR(name.as_mut_ptr())), &mut n, Some(PWSTR(domain.as_mut_ptr())), &mut d, &mut kind) }.ok()?;
    Some(String::from_utf16_lossy(&name[..n as usize]))
}

/// "COMPUTER\user" (or "DOMAIN\user").
fn user_name() -> Option<String> {
    let mut buf = [0u16; 512];
    let mut n = buf.len() as u32;
    unsafe { GetUserNameExW(NameSamCompatible, Some(PWSTR(buf.as_mut_ptr())), &mut n) }.then(|| String::from_utf16_lossy(&buf[..n as usize]))
}

/// Whether the user's account is in the group (whatever this sign-in's token says).
fn account_in_group() -> bool {
    let (Some(group), Some(user)) = (group_name(), user_name()) else { return false };
    unsafe {
        let mut buf: *mut u8 = std::ptr::null_mut();
        let (mut read, mut total) = (0u32, 0u32);
        let user = HSTRING::from(user);
        if NetUserGetLocalGroups(PCWSTR::null(), &user, 0, LG_INCLUDE_INDIRECT, &mut buf, MAX_PREFERRED_LENGTH, &mut read, &mut total) != 0 || buf.is_null() {
            return false;
        }
        let groups = std::slice::from_raw_parts(buf as *const LOCALGROUP_USERS_INFO_0, read as usize);
        let found = groups.iter().any(|g| g.lgrui0_name.to_string().is_ok_and(|n| n.eq_ignore_ascii_case(&group)));
        let _ = NetApiBufferFree(Some(buf as _));
        found
    }
}

pub fn access() -> Access {
    if token_has(WinBuiltinPerfLoggingUsersSid) || token_has(WinBuiltinAdministratorsSid) {
        // A session start can still be refused (group policy).
        if DENIED.load(Ordering::Relaxed) {
            return Access::Needed;
        }
        Access::Granted
    } else if account_in_group() {
        Access::SignInAgain
    } else {
        Access::Needed
    }
}

/// Adds the user to Performance Log Users with `net localgroup`, run as
/// administrator (Windows asks first). Ok(false) when the user declined.
pub fn grant() -> anyhow::Result<bool> {
    let group = group_name().ok_or_else(|| anyhow::anyhow!("no Performance Log Users group"))?;
    let user = user_name().ok_or_else(|| anyhow::anyhow!("no user name"))?;
    let args = HSTRING::from(format!("localgroup \"{group}\" \"{user}\" /add"));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: w!("net.exe"),
        lpParameters: PCWSTR(args.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        if e.code() == ERROR_CANCELLED.to_hresult() {
            return Ok(false);
        }
        return Err(e.into());
    }
    let process: HANDLE = info.hProcess;
    if !process.is_invalid() {
        unsafe {
            if WaitForSingleObject(process, 30_000) == WAIT_OBJECT_0 {
                let mut code = 0u32;
                let _ = GetExitCodeProcess(process, &mut code);
                // 2 with "already a member" is fine: checked below.
                log::info!("fps: net localgroup exited with {code}");
            }
            let _ = CloseHandle(process);
        }
    }
    DENIED.store(false, Ordering::Relaxed);
    if account_in_group() {
        Ok(true)
    } else {
        anyhow::bail!("the account is not in the group")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_from_present_times() {
        let freq = 1000;
        // 100 fps for 2 s, then a single 50 ms hitch.
        let mut times: VecDeque<i64> = (0..200).map(|i| i * 10).collect();
        times.push_back(1990 + 50);
        let r = rate_of(&times, 2040, freq).unwrap();
        assert!((r.fps - 96.0).abs() < 2.0, "{r:?}");
        assert!(r.low_1 < 101.0, "{r:?}");
        // Still the same a second later (events arrive in batches)...
        assert!((rate_of(&times, 3000, freq).unwrap().fps - r.fps).abs() < 0.01);
        // ...but nothing for a few seconds is shown as 0.
        assert_eq!(rate_of(&times, 5000, freq).unwrap().fps, 0.0);
    }

    #[test]
    fn the_group_resolves() {
        assert!(group_name().is_some_and(|n| !n.is_empty()));
        assert!(user_name().is_some_and(|n| n.contains('\\')));
    }
}
