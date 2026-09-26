//! GeniusClip installer: a small window in the app's own style around the
//! standard NSIS setup, which it runs silently. The app's updater keeps using
//! the NSIS setup directly.
#![windows_subsystem = "windows"]

mod i18n;
mod install;
mod ui;

use windows::core::{w, HSTRING};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

fn main() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    if let Err(e) = ui::run() {
        unsafe {
            MessageBoxW(None, &HSTRING::from(format!("{e}")), w!("GeniusClip Setup"), MB_OK | MB_ICONERROR);
        }
    }
}
