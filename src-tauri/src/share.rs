//! Sharing: put clip files on the clipboard the same way Explorer's "Copy"
//! does, so Ctrl+V in Telegram (or Discord, a folder, …) attaches them.

use anyhow::{anyhow, Context, Result};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::Duration;
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HWND, POINT};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{CF_HDROP, DROPEFFECT_COPY};
use windows::Win32::UI::Shell::DROPFILES;

/// Allocates a movable global memory block holding `bytes`.
unsafe fn global(bytes: &[u8]) -> Result<HANDLE> {
    let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
    let p = GlobalLock(h) as *mut u8;
    if p.is_null() {
        return Err(anyhow!("GlobalLock failed"));
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    let _ = GlobalUnlock(h);
    Ok(HANDLE(h.0))
}

/// Copies files to the clipboard (CF_HDROP + "Preferred DropEffect: copy").
/// `owner` must be a window of this process: with a NULL owner Windows
/// rejects SetClipboardData after EmptyClipboard.
pub fn copy_files(owner: HWND, paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        return Err(anyhow!("nothing to copy"));
    }
    let header = std::mem::size_of::<DROPFILES>();
    let mut wide: Vec<u16> = Vec::new();
    for p in paths {
        wide.extend(p.as_os_str().encode_wide());
        wide.push(0);
    }
    wide.push(0);
    let df = DROPFILES { pFiles: header as u32, pt: POINT::default(), fNC: false.into(), fWide: true.into() };
    let mut buf = vec![0u8; header + wide.len() * 2];
    unsafe {
        std::ptr::write_unaligned(buf.as_mut_ptr() as *mut DROPFILES, df);
        std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, buf.as_mut_ptr().add(header), wide.len() * 2);
    }

    unsafe {
        // The clipboard can be briefly held by another app; retry a little.
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(Some(owner)).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        if !opened {
            return Err(anyhow!("clipboard is busy"));
        }
        let res = (|| -> Result<()> {
            EmptyClipboard()?;
            SetClipboardData(CF_HDROP.0 as u32, Some(global(&buf)?)).context("SetClipboardData(CF_HDROP)")?;
            let effect = RegisterClipboardFormatW(w!("Preferred DropEffect"));
            let _ = SetClipboardData(effect, Some(global(&DROPEFFECT_COPY.0.to_le_bytes())?));
            Ok(())
        })();
        let _ = CloseClipboard();
        res
    }
}
