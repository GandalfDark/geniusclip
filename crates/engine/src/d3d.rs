//! Monitor enumeration and D3D11 device creation.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use windows::core::Interface;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::{HMODULE, ERROR_SUCCESS};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

pub const VENDOR_NVIDIA: u32 = 0x10DE;
pub const VENDOR_AMD: u32 = 0x1002;
pub const VENDOR_INTEL: u32 = 0x8086;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    /// GDI device name, e.g. `\\.\DISPLAY1` — stable identifier used in settings.
    pub id: String,
    /// Human readable monitor name from EDID, e.g. "DELL S2721DGF".
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub primary: bool,
    pub hdr: bool,
    pub adapter: String,
    pub vendor_id: u32,
}

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// GDI device name → EDID friendly name.
fn friendly_names() -> HashMap<String, String> {
    let mut map = HashMap::new();
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) != ERROR_SUCCESS {
            return map;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None)
            != ERROR_SUCCESS
        {
            return map;
        }
        for p in &paths[..np as usize] {
            let mut src = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            src.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            src.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            src.header.adapterId = p.sourceInfo.adapterId;
            src.header.id = p.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut src.header) != 0 {
                continue;
            }
            let mut tgt = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            tgt.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            tgt.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            tgt.header.adapterId = p.targetInfo.adapterId;
            tgt.header.id = p.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut tgt.header) != 0 {
                continue;
            }
            let name = wide_to_string(&tgt.monitorFriendlyDeviceName);
            if !name.is_empty() {
                map.insert(wide_to_string(&src.viewGdiDeviceName), name);
            }
        }
    }
    map
}

pub(crate) struct OutputHandle {
    pub adapter: IDXGIAdapter1,
    pub output: IDXGIOutput,
    pub info: MonitorInfo,
}

fn enumerate() -> Result<Vec<OutputHandle>> {
    let names = friendly_names();
    let mut out = Vec::new();
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().context("CreateDXGIFactory1")?;
        let mut ai = 0;
        while let Ok(adapter) = factory.EnumAdapters1(ai) {
            ai += 1;
            let ad = adapter.GetDesc1()?;
            if ad.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let adapter_name = wide_to_string(&ad.Description);
            let mut oi = 0;
            while let Ok(output) = adapter.EnumOutputs(oi) {
                oi += 1;
                let od = output.GetDesc()?;
                if !od.AttachedToDesktop.as_bool() {
                    continue;
                }
                let id = wide_to_string(&od.DeviceName);
                let r = od.DesktopCoordinates;
                let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                let primary = GetMonitorInfoW(od.Monitor, &mut mi).as_bool() && mi.dwFlags & MONITORINFOF_PRIMARY != 0;
                let hdr = output
                    .cast::<IDXGIOutput6>()
                    .and_then(|o6| o6.GetDesc1())
                    .map(|d| d.ColorSpace == windows::Win32::Graphics::Dxgi::Common::DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020)
                    .unwrap_or(false);
                let name = names.get(&id).cloned().unwrap_or_else(|| id.trim_start_matches("\\\\.\\").to_string());
                out.push(OutputHandle {
                    adapter: adapter.clone(),
                    output,
                    info: MonitorInfo {
                        id,
                        name,
                        width: (r.right - r.left) as u32,
                        height: (r.bottom - r.top) as u32,
                        x: r.left,
                        y: r.top,
                        primary,
                        hdr,
                        adapter: adapter_name.clone(),
                        vendor_id: ad.VendorId,
                    },
                });
            }
        }
    }
    Ok(out)
}

/// A fingerprint of the monitors and the adapters driving them. It changes
/// when a monitor comes or goes, or the GPU is back after a driver reset
/// (meanwhile Windows drives the screens with its Basic Render Driver).
pub(crate) fn display_setup() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for o in enumerate().unwrap_or_default() {
        let i = &o.info;
        (&i.id, i.x, i.y, i.width, i.height, &i.adapter, i.vendor_id).hash(&mut h);
    }
    h.finish()
}

pub fn list_monitors() -> Result<Vec<MonitorInfo>> {
    Ok(enumerate()?.into_iter().map(|o| o.info).collect())
}

/// Finds the requested monitor, falling back to the primary one.
pub(crate) fn find_output(id: Option<&str>) -> Result<OutputHandle> {
    let mut all = enumerate()?;
    if all.is_empty() {
        return Err(anyhow!("no active monitors found"));
    }
    let idx = id
        .and_then(|id| all.iter().position(|o| o.info.id == id))
        .or_else(|| all.iter().position(|o| o.info.primary))
        .unwrap_or(0);
    Ok(all.swap_remove(idx))
}

pub(crate) fn create_device(adapter: &IDXGIAdapter1) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device = None;
    let mut ctx = None;
    unsafe {
        D3D11CreateDevice(
            adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut ctx),
        )
        .context("D3D11CreateDevice")?;
    }
    let device = device.ok_or_else(|| anyhow!("no device"))?;
    let ctx = ctx.ok_or_else(|| anyhow!("no context"))?;
    unsafe {
        // FFmpeg's encoders touch the immediate context from their own code paths.
        if let Ok(mt) = device.cast::<ID3D11Multithread>() {
            let _ = mt.SetMultithreadProtected(true);
        }
        // Keep capture/encode responsive while a game saturates the GPU.
        if let Ok(dxgi) = device.cast::<IDXGIDevice>() {
            let _ = dxgi.SetGPUThreadPriority(7);
        }
    }
    Ok((device, ctx))
}

pub(crate) fn create_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
    bind: D3D11_BIND_FLAG,
) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: bind.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut tex = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut tex)).context("CreateTexture2D")? };
    tex.ok_or_else(|| anyhow!("CreateTexture2D returned null"))
}
