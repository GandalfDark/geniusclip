//! DXGI Desktop Duplication capture of a single monitor.
//!
//! Chosen over Windows.Graphics.Capture because it never draws the yellow
//! "being captured" border on Windows 10 and works for borderless and
//! (non-exclusive) fullscreen games.

use crate::cursor::{convert_shape, CursorState};
use anyhow::{Context, Result};
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Foundation::E_ACCESSDENIED;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::*;

/// Duplication failing for this long means the output itself is gone
/// (unplugged, Win+P): `poll` then fails so capture starts over.
const GIVE_UP_AFTER: Duration = Duration::from_secs(5);

pub struct Duplicator {
    device: ID3D11Device,
    output: IDXGIOutput1,
    dup: Option<IDXGIOutputDuplication>,
    pub width: u32,
    pub height: u32,
    next_retry: Instant,
    /// When duplication started failing (None while it works).
    lost_since: Option<Instant>,
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
        let mut me = Duplicator {
            device: device.clone(),
            output,
            dup: None,
            width: (r.right - r.left) as u32,
            height: (r.bottom - r.top) as u32,
            next_retry: Instant::now(),
            lost_since: None,
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
        self.width = desc.ModeDesc.Width;
        self.height = desc.ModeDesc.Height;
        self.dup = Some(dup);
        log::info!("desktop duplication started {}x{}", self.width, self.height);
        Ok(())
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
                // The secure desktop (UAC prompt, lock screen) denies access
                // for as long as it is shown: that is not a lost output.
                if e.downcast_ref::<windows::core::Error>().is_some_and(|w| w.code() == E_ACCESSDENIED) {
                    self.lost_since = None;
                }
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
                self.lost_since = None;
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
                self.lost_since = None;
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

    /// Drops the duplication so the next poll after `retry` recreates it;
    /// fails once it has been failing for `GIVE_UP_AFTER`.
    fn lost(&mut self, e: anyhow::Error, retry: Duration) -> Result<Poll> {
        self.dup = None;
        self.next_retry = Instant::now() + retry;
        if self.lost_since.get_or_insert_with(Instant::now).elapsed() > GIVE_UP_AFTER {
            return Err(e.context("desktop duplication keeps failing"));
        }
        Ok(Poll::Lost)
    }
}
