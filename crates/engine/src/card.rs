//! The "menu is open" card on the held frame (see `Engine::set_hold`), so a
//! clip's frozen stretch doesn't look like a hung video. Drawn once per hold
//! with Direct2D straight onto a copy of the last frame.

use anyhow::Result;
use windows::core::{w, Interface};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::IDXGISurface;

/// Text of the card, in the app's language.
#[derive(Clone, Debug)]
pub struct HoldCard {
    pub title: String,
    pub subtitle: String,
    /// Accent colour of the icon, RGB.
    pub accent: [u8; 3],
}

fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }
}

fn rounded(r: D2D_RECT_F, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }
}

/// Dims `tex` (a `w`×`h` BGRA render target) and draws the card centred on it.
pub(crate) fn draw(tex: &ID3D11Texture2D, w: u32, h: u32, card: &HoldCard) -> Result<()> {
    let (wf, hf) = (w as f32, h as f32);
    unsafe {
        let surface: IDXGISurface = tex.cast()?;
        let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        };
        let rt = d2d.CreateDxgiSurfaceRenderTarget(&surface, &props)?;
        let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

        // Sizes follow the frame height, so 1080p and 4K clips look the same.
        let title_size = (hf * 0.034).max(16.0);
        let sub_size = (hf * 0.019).max(11.0);
        let format = |size: f32, weight| dw.CreateTextFormat(w!("Segoe UI"), None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!(""));
        let title_fmt = format(title_size, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
        let sub_fmt = format(sub_size, DWRITE_FONT_WEIGHT_NORMAL)?;
        let title: Vec<u16> = card.title.encode_utf16().collect();
        let sub: Vec<u16> = card.subtitle.encode_utf16().collect();
        let measure = |text: &[u16], f: &IDWriteTextFormat| -> Result<DWRITE_TEXT_METRICS> {
            let mut m = DWRITE_TEXT_METRICS::default();
            dw.CreateTextLayout(text, f, wf, hf)?.GetMetrics(&mut m)?;
            Ok(m)
        };
        let tm = measure(&title, &title_fmt)?;
        let sm = measure(&sub, &sub_fmt)?;

        let icon = title_size * 0.95;
        let gap = title_size * 0.45;
        let row_w = icon + gap + tm.width;
        let spacing = sub_size * 0.7;
        let top = (hf - (tm.height + spacing + sm.height)) / 2.0;
        let left = (wf - row_w) / 2.0;

        rt.BeginDraw();
        let dim = rt.CreateSolidColorBrush(&color(0.024, 0.024, 0.03, 0.55), None)?;
        rt.FillRectangle(&rect(0.0, 0.0, wf, hf), &dim);

        // Accent square with pause bars.
        let [r, g, b] = card.accent.map(|c| c as f32 / 255.0);
        let accent = rt.CreateSolidColorBrush(&color(r, g, b, 1.0), None)?;
        // Dark bars on a light accent (the monochrome one).
        let light = 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.7;
        let bars = if light { color(0.08, 0.08, 0.1, 1.0) } else { color(1.0, 1.0, 1.0, 1.0) };
        let white = rt.CreateSolidColorBrush(&bars, None)?;
        let iy = top + (tm.height - icon) / 2.0;
        rt.FillRoundedRectangle(&rounded(rect(left, iy, icon, icon), icon * 0.28), &accent);
        let (bw, bh) = (icon * 0.13, icon * 0.44);
        for x in [0.33, 0.54] {
            rt.FillRoundedRectangle(&rounded(rect(left + icon * x, iy + (icon - bh) / 2.0, bw, bh), bw / 2.0), &white);
        }

        let text = rt.CreateSolidColorBrush(&color(0.945, 0.945, 0.96, 1.0), None)?;
        let muted = rt.CreateSolidColorBrush(&color(0.79, 0.79, 0.83, 1.0), None)?;
        let tx = left + icon + gap;
        rt.DrawText(&title, &title_fmt, &rect(tx, top, tm.width + 4.0, tm.height), &text, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        let sy = top + tm.height + spacing;
        rt.DrawText(&sub, &sub_fmt, &rect((wf - sm.width) / 2.0, sy, sm.width + 4.0, sm.height), &muted, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        rt.EndDraw(None, None)?;
    }
    Ok(())
}
