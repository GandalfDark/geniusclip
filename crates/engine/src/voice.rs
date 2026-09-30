//! Discord voices on a track of their own: finding Discord's process tree
//! and capturing it, or everything but it, with WASAPI process loopback
//! (Windows 10 2004, build 19041, and later).

use anyhow::{bail, Context, Result};
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use std::mem::{size_of, ManuallyDrop};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::core::{implement, IUnknown, Interface, Ref, HRESULT};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0};
use windows::Win32::System::Com::BLOB;
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::Variant::VT_BLOB;

/// Executables of Discord's release channels.
const DISCORD: [&str; 3] = ["Discord.exe", "DiscordPTB.exe", "DiscordCanary.exe"];
/// How often the process list is scanned for Discord.
const RESCAN: Duration = Duration::from_secs(3);
/// Activation normally completes within milliseconds.
const ACTIVATE_TIMEOUT: Duration = Duration::from_secs(2);

/// Discord's process tree, shared by the game and voice capture threads so
/// that both switch together when Discord starts, restarts or closes.
#[derive(Default)]
pub(crate) struct Discord(Mutex<State>);

#[derive(Default)]
struct State {
    pid: u32,
    scanned: Option<Instant>,
    /// Process loopback failed for this pid; tried again once it changes.
    failed: u32,
}

impl Discord {
    /// Root pid of the process tree to split off: 0 while Discord isn't
    /// running or process loopback failed for it. Scans at most every
    /// `RESCAN`, however often it is asked.
    pub(crate) fn target(&self) -> u32 {
        let mut s = self.0.lock();
        if s.scanned.is_none_or(|t| t.elapsed() >= RESCAN) {
            s.scanned = Some(Instant::now());
            s.pid = find_root().unwrap_or(0);
        }
        if s.pid == s.failed {
            0
        } else {
            s.pid
        }
    }

    /// Process loopback failed for `pid` (Windows before 19041, most
    /// likely): Discord stays in the game audio and the voice track silent.
    /// Logged once per run: on such a system it fails every time.
    pub(crate) fn failed(&self, pid: u32, e: &anyhow::Error) {
        static WARNED: AtomicBool = AtomicBool::new(false);
        self.0.lock().failed = pid;
        if !WARNED.swap(true, Ordering::Relaxed) {
            log::warn!("Discord voices stay in the game audio, process loopback failed: {e:#}");
        }
    }
}

/// A Discord process whose parent is not one: the others (renderer, GPU,
/// voice helpers) are its descendants. With several release channels
/// running, the first one found is split off.
fn find_root() -> Option<u32> {
    let mut procs = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snap, &mut e).is_ok();
        while more {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            let name = String::from_utf16_lossy(&e.szExeFile[..len]);
            if DISCORD.iter().any(|d| d.eq_ignore_ascii_case(&name)) {
                procs.push((e.th32ProcessID, e.th32ParentProcessID));
            }
            more = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    tree_root(&procs)
}

/// The first `(pid, parent)` whose parent is not in the list.
fn tree_root(procs: &[(u32, u32)]) -> Option<u32> {
    procs.iter().find(|(_, parent)| !procs.iter().any(|(pid, _)| pid == parent)).map(|&(pid, _)| pid)
}

/// Signals the activation's end (called on a system thread; objects made
/// with `implement` are agile, as ActivateAudioInterfaceAsync requires).
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Completion(Sender<()>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
    fn ActivateCompleted(&self, _op: Ref<IActivateAudioInterfaceAsyncOperation>) -> windows::core::Result<()> {
        let _ = self.0.try_send(());
        Ok(())
    }
}

/// An audio client for the process tree of `pid` only (`include`) or for
/// everything but it. It has no mix format: Initialize it with the format
/// wanted and the loopback flag. Gives up early when `stop` is set.
pub(crate) unsafe fn process_loopback(pid: u32, include: bool, stop: &AtomicBool) -> Result<IAudioClient> {
    let mode = if include { PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE } else { PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE };
    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS { TargetProcessId: pid, ProcessLoopbackMode: mode },
        },
    };
    // Borrows `params`, which outlives the activation (waited for below).
    let prop = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB { cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32, pBlobData: &params as *const _ as *mut u8 },
                },
                ..Default::default()
            }),
        },
    };
    let (tx, rx) = crossbeam_channel::bounded(1);
    let handler: IActivateAudioInterfaceCompletionHandler = Completion(tx).into();
    let op = ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, Some(&prop as *const _), &handler)
        .context("ActivateAudioInterfaceAsync")?;
    let deadline = Instant::now() + ACTIVATE_TIMEOUT;
    while rx.recv_timeout(Duration::from_millis(50)).is_err() {
        if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
            bail!("process loopback activation did not complete");
        }
    }
    let (mut hr, mut client) = (HRESULT(0), None::<IUnknown>);
    op.GetActivateResult(&mut hr, &mut client).context("GetActivateResult")?;
    hr.ok().context("process loopback activation")?;
    Ok(client.context("no audio client")?.cast()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_of_process_tree() {
        // Update.exe (40) started the main process (10), which started its helpers.
        assert_eq!(tree_root(&[(11, 10), (10, 40), (12, 10), (13, 12)]), Some(10));
        assert_eq!(tree_root(&[]), None);
        // A parent that exited long ago keeps its pid in the child's entry.
        assert_eq!(tree_root(&[(7, 3)]), Some(7));
    }
}
