//! DXGI Desktop Duplication capture of a single monitor.
//!
//! Chosen over Windows.Graphics.Capture because it never draws the yellow
//! "being captured" border on Windows 10 and works for borderless and
//! (non-exclusive) fullscreen games.

use crate::cursor::{convert_shape, CursorState};
use anyhow::{Context, Result};
use std::time::{Duration, Instant};
use windows::core::{Interface, HRESULT};
use windows::Win32::Foundation::{E_ACCESSDENIED, LUID};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

/// Duplication failing for this long means the output itself is gone
/// (unplugged, Win+P): `poll` then fails so capture starts over. Errors that
/// only mean "not now" (see `is_waiting`) are waited out instead, as long as
/// the output is still there; it is checked again this often.
const GIVE_UP_AFTER: Duration = Duration::from_secs(5);

/// States that last as long as something else does: the secure desktop
/// (UAC, lock screen), a mode change or a mode duplication cannot handle,
/// too many duplication clients, a remote or switched-away session. Ending
/// capture for them would only wipe the replay buffer and end a recording
/// every few seconds.
fn is_waiting(code: HRESULT) -> bool {
    [
        E_ACCESSDENIED,
        DXGI_ERROR_ACCESS_LOST,
        DXGI_ERROR_UNSUPPORTED,
        DXGI_ERROR_NOT_CURRENTLY_AVAILABLE,
        DXGI_ERROR_SESSION_DISCONNECTED,
        DXGI_ERROR_MODE_CHANGE_IN_PROGRESS,
    ]
    .contains(&code)
}

/// The D3D device is gone (driver update, TDR): nothing recovers without a new one.
fn is_device_lost(code: HRESULT) -> bool {
    [DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, DXGI_ERROR_DEVICE_HUNG, DXGI_ERROR_DRIVER_INTERNAL_ERROR].contains(&code)
}

fn hresult(e: &anyhow::Error) -> Option<HRESULT> {
    e.downcast_ref::<windows::core::Error>().map(|w| w.code())
}

/// Clockwise rotation that turns the duplicated image upright. On a
/// portrait or flipped display Desktop Duplication hands out the unrotated
/// scan-out image, while pointer position and shape are upright.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rotation {
    #[default]
    None,
    Cw90,
    Cw180,
    Cw270,
}

impl Rotation {
    fn from_dxgi(r: DXGI_MODE_ROTATION) -> Self {
        match r {
            DXGI_MODE_ROTATION_ROTATE90 => Rotation::Cw90,
            DXGI_MODE_ROTATION_ROTATE180 => Rotation::Cw180,
            DXGI_MODE_ROTATION_ROTATE270 => Rotation::Cw270,
            _ => Rotation::None,
        }
    }

    /// Width and height trade places (portrait).
    pub fn swaps(self) -> bool {
        matches!(self, Rotation::Cw90 | Rotation::Cw270)
    }

    /// The size of a `w`×`h` image after rotation.
    pub fn apply(self, w: u32, h: u32) -> (u32, u32) {
        if self.swaps() {
            (h, w)
        } else {
            (w, h)
        }
    }

    pub fn inverse(self) -> Self {
        match self {
            Rotation::Cw90 => Rotation::Cw270,
            Rotation::Cw270 => Rotation::Cw90,
            r => r,
        }
    }

    /// Maps the upright rectangle (x, y, w, h) into a duplicated image of
    /// `tw`×`th` pixels (unrotated), as (x, y, w, h).
    pub fn to_image(self, x: i32, y: i32, w: i32, h: i32, tw: i32, th: i32) -> (i32, i32, i32, i32) {
        match self {
            Rotation::None => (x, y, w, h),
            Rotation::Cw90 => (y, th - x - w, h, w),
            Rotation::Cw180 => (tw - x - w, th - y - h, w, h),
            Rotation::Cw270 => (tw - y - h, x, h, w),
        }
    }
}

/// Rotates a BGRA image (rows `pitch` bytes apart) clockwise; returns the
/// tightly packed pixels and their size.
pub fn rotate_bgra(src: &[u8], w: u32, h: u32, pitch: usize, rot: Rotation) -> (Vec<u8>, u32, u32) {
    let (w, h) = (w as usize, h as usize);
    let dw = if rot.swaps() { h } else { w };
    let mut dst = vec![0u8; w * h * 4];
    for sy in 0..h {
        for (sx, px) in src[sy * pitch..][..w * 4].chunks_exact(4).enumerate() {
            let (dx, dy) = match rot {
                Rotation::None => (sx, sy),
                Rotation::Cw90 => (h - 1 - sy, sx),
                Rotation::Cw180 => (w - 1 - sx, h - 1 - sy),
                Rotation::Cw270 => (sy, w - 1 - sx),
            };
            dst[(dy * dw + dx) * 4..][..4].copy_from_slice(px);
        }
    }
    let (dw, dh) = rot.apply(w as u32, h as u32);
    (dst, dw, dh)
}

pub struct Duplicator {
    device: ID3D11Device,
    output: IDXGIOutput1,
    /// GDI name and adapter of the output, to tell whether it still exists.
    device_name: [u16; 32],
    adapter_luid: Option<LUID>,
    dup: Option<IDXGIOutputDuplication>,
    /// Size of the duplicated image (unrotated, as the frame texture comes).
    /// Estimated from the display mode until a frame arrives, then taken
    /// from the frame itself.
    pub width: u32,
    pub height: u32,
    pub rotation: Rotation,
    next_retry: Instant,
    /// When duplication started failing (None while it works); restarted
    /// after each check that found the output still there.
    lost_since: Option<Instant>,
    /// A long wait was logged for the current failure streak.
    wait_logged: bool,
    shape_buf: Vec<u8>,
}

pub enum Poll {
    /// Nothing changed since the last poll.
    Idle,
    /// Desktop image and/or cursor changed. `desktop` is true when the image was copied.
    Changed { desktop: bool },
    /// Duplication is temporarily unavailable (secure desktop, mode change…).
    Lost,
}

impl Duplicator {
    pub fn new(device: &ID3D11Device, output: &IDXGIOutput) -> Result<Self> {
        let output: IDXGIOutput1 = output.cast().context("IDXGIOutput1")?;
        let desc = unsafe { output.GetDesc()? };
        let r = desc.DesktopCoordinates;
        // Desktop coordinates are upright; the duplicated image is not.
        let rotation = Rotation::from_dxgi(desc.Rotation);
        let (width, height) = rotation.apply((r.right - r.left) as u32, (r.bottom - r.top) as u32);
        let adapter_luid = unsafe { device.cast::<IDXGIDevice>().and_then(|d| d.GetAdapter()).and_then(|a| a.GetDesc()) }
            .ok()
            .map(|d| d.AdapterLuid);
        let mut me = Duplicator {
            device: device.clone(),
            output,
            device_name: desc.DeviceName,
            adapter_luid,
            dup: None,
            width,
            height,
            rotation,
            next_retry: Instant::now(),
            lost_since: None,
            wait_logged: false,
            shape_buf: Vec::new(),
        };
        if let Err(e) = me.start() {
            log::warn!("desktop duplication not available yet: {e:#}");
        }
        Ok(me)
    }

    fn start(&mut self) -> Result<()> {
        self.dup = None;
        let dup = unsafe {
            match self.output.cast::<IDXGIOutput5>() {
                Ok(o5) => o5.DuplicateOutput1(&self.device, 0, &[DXGI_FORMAT_B8G8R8A8_UNORM]),
                Err(_) => self.output.DuplicateOutput(&self.device),
            }
        }
        .context("DuplicateOutput")?;
        let desc = unsafe { dup.GetDesc() };
        // ModeDesc is the upright desktop size (like DesktopCoordinates),
        // while frames come in scan-out orientation, which `rotation` turns
        // upright. The first frame's texture has the final say (see `poll`).
        let (mw, mh) = (desc.ModeDesc.Width, desc.ModeDesc.Height);
        self.rotation = Rotation::from_dxgi(desc.Rotation);
        (self.width, self.height) = self.rotation.apply(mw, mh);
        self.dup = Some(dup);
        log::info!("desktop duplication started: desktop {mw}x{mh}, rotation {:?}", self.rotation);
        Ok(())
    }

    /// Size of the image once `rotation` turned it upright.
    pub fn upright_size(&self) -> (u32, u32) {
        self.rotation.apply(self.width, self.height)
    }

    /// Grabs the newest desktop frame (non-blocking) into `dst`, which must be
    /// a BGRA texture of `width`×`height`. The caller recreates `dst` when the
    /// size changes (check `width`/`height` after `Poll::Changed`).
    pub fn poll(
        &mut self,
        ctx: &ID3D11DeviceContext,
        dst: &mut Option<ID3D11Texture2D>,
        make_dst: &mut dyn FnMut(u32, u32) -> Result<ID3D11Texture2D>,
        cursor: &mut CursorState,
    ) -> Result<Poll> {
        if self.dup.is_none() {
            if Instant::now() < self.next_retry {
                return Ok(Poll::Lost);
            }
            if let Err(e) = self.start() {
                log::debug!("duplication retry failed: {e:#}");
                return self.lost(e, Duration::from_millis(500));
            }
            *dst = None;
        }
        let dup = self.dup.as_ref().unwrap().clone();

        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut res: Option<IDXGIResource> = None;
        match unsafe { dup.AcquireNextFrame(0, &mut info, &mut res) } {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                self.recovered();
                return Ok(Poll::Idle);
            }
            Err(e) => {
                // ACCESS_LOST (mode switch, fullscreen transition, UAC) or similar: recreate.
                log::info!("duplication lost: {e}");
                return self.lost(e.into(), Duration::from_millis(100));
            }
        }

        let result = (|| -> Result<Poll> {
            let mut desktop = false;
            let mut cur = false;
            if info.LastPresentTime != 0 {
                if let Some(res) = res.as_ref() {
                    let tex: ID3D11Texture2D = res.cast()?;
                    let mut d = D3D11_TEXTURE2D_DESC::default();
                    unsafe { tex.GetDesc(&mut d) };
                    if d.Width != self.width || d.Height != self.height || dst.is_none() {
                        // The texture is what gets converted: its size wins
                        // over the mode's (the upright size follows from it).
                        if (d.Width, d.Height) != (self.width, self.height) {
                            log::info!("desktop frame is {}x{}, expected {}x{} (rotation {:?})", d.Width, d.Height, self.width, self.height, self.rotation);
                        }
                        self.width = d.Width;
                        self.height = d.Height;
                        *dst = Some(make_dst(d.Width, d.Height)?);
                    }
                    unsafe { ctx.CopyResource(dst.as_ref().unwrap(), &tex) };
                    desktop = true;
                }
            }
            if info.LastMouseUpdateTime != 0 {
                let vis = info.PointerPosition.Visible.as_bool();
                let (x, y) = (info.PointerPosition.Position.x, info.PointerPosition.Position.y);
                if vis != cursor.visible || (vis && (x != cursor.x || y != cursor.y)) {
                    cur = true;
                }
                cursor.visible = vis;
                if vis {
                    cursor.x = x;
                    cursor.y = y;
                }
            }
            if info.PointerShapeBufferSize > 0 {
                self.shape_buf.resize(info.PointerShapeBufferSize as usize, 0);
                let mut needed = 0u32;
                let mut si = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
                unsafe {
                    dup.GetFramePointerShape(
                        self.shape_buf.len() as u32,
                        self.shape_buf.as_mut_ptr() as *mut _,
                        &mut needed,
                        &mut si,
                    )?
                };
                cursor.shape = convert_shape(&si, &self.shape_buf);
                cursor.shape_gen += 1;
                cur = true;
            }
            Ok(if desktop || cur { Poll::Changed { desktop } } else { Poll::Idle })
        })();

        unsafe {
            let _ = dup.ReleaseFrame();
        }
        match result {
            Ok(p) => {
                self.recovered();
                Ok(p)
            }
            // Pointer shape or frame copy failed: recover like a lost
            // duplication rather than ending capture (and the replay buffer).
            Err(e) => {
                log::warn!("desktop frame failed: {e:#}");
                self.lost(e, Duration::from_millis(100))
            }
        }
    }

    fn recovered(&mut self) {
        if self.lost_since.take().is_some() && std::mem::take(&mut self.wait_logged) {
            log::info!("desktop duplication works again");
        }
    }

    /// Drops the duplication so the next poll after `retry` recreates it.
    /// Fails when the device is lost, or once it has been failing for
    /// `GIVE_UP_AFTER` and the output is gone or the error is not one that
    /// is worth waiting out.
    fn lost(&mut self, e: anyhow::Error, retry: Duration) -> Result<Poll> {
        self.dup = None;
        self.next_retry = Instant::now() + retry;
        let code = hresult(&e);
        if code.is_some_and(is_device_lost) {
            return Err(e.context("graphics device lost"));
        }
        if self.lost_since.get_or_insert_with(Instant::now).elapsed() <= GIVE_UP_AFTER {
            return Ok(Poll::Lost);
        }
        if !self.output_present() {
            return Err(e.context("display output is gone"));
        }
        if !code.is_some_and(is_waiting) {
            // Possibly a stale output after a display change: a new
            // pipeline enumerates the outputs again.
            return Err(e.context("desktop duplication keeps failing"));
        }
        if !std::mem::replace(&mut self.wait_logged, true) {
            log::info!("desktop duplication unavailable ({e:#}); waiting for it");
        }
        // Next output check in another GIVE_UP_AFTER.
        self.lost_since = Some(Instant::now());
        Ok(Poll::Lost)
    }

    /// Whether the output is still part of the desktop, on this device's
    /// adapter. Asks a new factory: an old one can keep describing the
    /// display layout it was created with.
    fn output_present(&self) -> bool {
        unsafe {
            if self.device.GetDeviceRemovedReason().is_err() {
                return false;
            }
            let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
                // Cannot tell: keep waiting rather than restart for nothing.
                return true;
            };
            let mut ai = 0;
            while let Ok(adapter) = factory.EnumAdapters1(ai) {
                ai += 1;
                let luid = adapter.GetDesc1().map(|d| d.AdapterLuid).ok();
                if self.adapter_luid.is_some() && luid != self.adapter_luid {
                    continue;
                }
                let mut oi = 0;
                while let Ok(output) = adapter.EnumOutputs(oi) {
                    oi += 1;
                    if output.GetDesc().is_ok_and(|d| d.DeviceName == self.device_name && d.AttachedToDesktop.as_bool()) {
                        return true;
                    }
                }
            }
            false
        }
    }
}
