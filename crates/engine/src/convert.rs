//! GPU colour conversion + scaling (BGRA desktop → NV12 encoder surface)
//! using the D3D11 video processor, which exists on NVIDIA, AMD and Intel.
//! It also turns the image of a rotated (portrait) display upright.

use crate::dup::Rotation;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use windows::core::Interface;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;

pub struct Converter {
    vdev: ID3D11VideoDevice,
    vctx: ID3D11VideoContext,
    venum: ID3D11VideoProcessorEnumerator,
    vp: ID3D11VideoProcessor,
    /// Input size, as duplicated (before `rotation`).
    pub in_w: u32,
    pub in_h: u32,
    /// Rotation applied to the input (None if the GPU cannot rotate).
    pub rotation: Rotation,
    requested: Rotation,
    pub out_w: u32,
    pub out_h: u32,
    inputs: HashMap<usize, ID3D11VideoProcessorInputView>,
    outputs: HashMap<(usize, u32), ID3D11VideoProcessorOutputView>,
}

impl Converter {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
        in_w: u32,
        in_h: u32,
        rotation: Rotation,
        out_w: u32,
        out_h: u32,
        fps: u32,
    ) -> Result<Self> {
        let vdev: ID3D11VideoDevice = device.cast().context("ID3D11VideoDevice")?;
        let vctx: ID3D11VideoContext = ctx.cast().context("ID3D11VideoContext")?;
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: DXGI_RATIONAL { Numerator: fps, Denominator: 1 },
            InputWidth: in_w,
            InputHeight: in_h,
            OutputFrameRate: DXGI_RATIONAL { Numerator: fps, Denominator: 1 },
            OutputWidth: out_w,
            OutputHeight: out_h,
            Usage: D3D11_VIDEO_USAGE_OPTIMAL_QUALITY,
        };
        unsafe {
            let venum = vdev.CreateVideoProcessorEnumerator(&desc).context("CreateVideoProcessorEnumerator")?;
            let fin = venum.CheckVideoProcessorFormat(DXGI_FORMAT_B8G8R8A8_UNORM)?;
            let fout = venum.CheckVideoProcessorFormat(DXGI_FORMAT_NV12)?;
            if fin & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT.0 as u32 == 0
                || fout & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT.0 as u32 == 0
            {
                anyhow::bail!("GPU video processor cannot convert BGRA to NV12");
            }
            let vp = vdev.CreateVideoProcessor(&venum, 0).context("CreateVideoProcessor")?;
            let mut caps = D3D11_VIDEO_PROCESSOR_CAPS::default();
            let can_rotate = venum.GetVideoProcessorCaps(&mut caps).is_ok()
                && caps.FeatureCaps & D3D11_VIDEO_PROCESSOR_FEATURE_CAPS_ROTATION.0 as u32 != 0;
            let applied = if rotation != Rotation::None && !can_rotate {
                log::warn!("GPU video processor cannot rotate: the rotated display is recorded sideways");
                Rotation::None
            } else {
                rotation
            };
            let me = Converter {
                vdev,
                vctx,
                venum,
                vp,
                in_w,
                in_h,
                rotation: applied,
                requested: rotation,
                out_w,
                out_h,
                inputs: HashMap::new(),
                outputs: HashMap::new(),
            };
            me.configure();
            Ok(me)
        }
    }

    /// Whether this converter was made for input of this size and rotation.
    pub fn fits(&self, in_w: u32, in_h: u32, rotation: Rotation) -> bool {
        (self.in_w, self.in_h, self.requested) == (in_w, in_h, rotation)
    }

    fn configure(&self) {
        let (vctx, vp) = (&self.vctx, &self.vp);
        // Letterbox when the (upright) desktop aspect differs from the output aspect.
        let (uw, uh) = self.rotation.apply(self.in_w, self.in_h);
        let scale = (self.out_w as f64 / uw as f64).min(self.out_h as f64 / uh as f64);
        let dw = ((uw as f64 * scale).round() as i32).min(self.out_w as i32);
        let dh = ((uh as f64 * scale).round() as i32).min(self.out_h as i32);
        let dx = (self.out_w as i32 - dw) / 2;
        let dy = (self.out_h as i32 - dh) / 2;
        let rotation = match self.rotation {
            Rotation::None => D3D11_VIDEO_PROCESSOR_ROTATION_IDENTITY,
            Rotation::Cw90 => D3D11_VIDEO_PROCESSOR_ROTATION_90,
            Rotation::Cw180 => D3D11_VIDEO_PROCESSOR_ROTATION_180,
            Rotation::Cw270 => D3D11_VIDEO_PROCESSOR_ROTATION_270,
        };
        unsafe {
            vctx.VideoProcessorSetStreamFrameFormat(vp, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
            vctx.VideoProcessorSetStreamAutoProcessingMode(vp, 0, false);
            // Source rect is in input (unrotated) pixels, dest rect in output pixels.
            vctx.VideoProcessorSetStreamRotation(vp, 0, self.rotation != Rotation::None, rotation);
            vctx.VideoProcessorSetStreamSourceRect(vp, 0, true, Some(&RECT { left: 0, top: 0, right: self.in_w as i32, bottom: self.in_h as i32 }));
            vctx.VideoProcessorSetStreamDestRect(vp, 0, true, Some(&RECT { left: dx, top: dy, right: dx + dw, bottom: dy + dh }));
            vctx.VideoProcessorSetOutputTargetRect(vp, true, Some(&RECT { left: 0, top: 0, right: self.out_w as i32, bottom: self.out_h as i32 }));
            let black = D3D11_VIDEO_COLOR {
                Anonymous: D3D11_VIDEO_COLOR_0 { YCbCr: D3D11_VIDEO_COLOR_YCbCrA { Y: 0.0625, Cb: 0.5, Cr: 0.5, A: 1.0 } },
            };
            vctx.VideoProcessorSetOutputBackgroundColor(vp, true, &black);
            if let Ok(vctx1) = vctx.cast::<ID3D11VideoContext1>() {
                vctx1.VideoProcessorSetStreamColorSpace1(vp, 0, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709);
                vctx1.VideoProcessorSetOutputColorSpace1(vp, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709);
            } else {
                // Usage=0, RGB_Range=0 (full) in; YCbCr_Matrix=1 (BT.709), Nominal_Range=1 (16-235) out.
                vctx.VideoProcessorSetStreamColorSpace(vp, 0, &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 });
                vctx.VideoProcessorSetOutputColorSpace(vp, &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: (1 << 2) | (1 << 4) });
            }
        }
    }

    /// Forget cached views (call when input textures are recreated).
    pub fn reset_inputs(&mut self) {
        self.inputs.clear();
    }

    fn input_view(&mut self, tex: &ID3D11Texture2D) -> Result<ID3D11VideoProcessorInputView> {
        let key = tex.as_raw() as usize;
        if let Some(v) = self.inputs.get(&key) {
            return Ok(v.clone());
        }
        let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: 0 } },
        };
        let mut view = None;
        unsafe { self.vdev.CreateVideoProcessorInputView(tex, &self.venum, &desc, Some(&mut view)) }
            .context("CreateVideoProcessorInputView")?;
        let view = view.unwrap();
        self.inputs.insert(key, view.clone());
        Ok(view)
    }

    fn output_view(&mut self, tex: &ID3D11Texture2D, slice: u32) -> Result<ID3D11VideoProcessorOutputView> {
        let key = (tex.as_raw() as usize, slice);
        if let Some(v) = self.outputs.get(&key) {
            return Ok(v.clone());
        }
        let mut td = D3D11_TEXTURE2D_DESC::default();
        unsafe { tex.GetDesc(&mut td) };
        let desc = if td.ArraySize > 1 {
            D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2DARRAY,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2DArray: D3D11_TEX2D_ARRAY_VPOV { MipSlice: 0, FirstArraySlice: slice, ArraySize: 1 },
                },
            }
        } else {
            D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 } },
            }
        };
        let mut view = None;
        unsafe { self.vdev.CreateVideoProcessorOutputView(tex, &self.venum, &desc, Some(&mut view)) }
            .context("CreateVideoProcessorOutputView")?;
        let view = view.unwrap();
        self.outputs.insert(key, view.clone());
        Ok(view)
    }

    pub fn convert(&mut self, input: &ID3D11Texture2D, output: &ID3D11Texture2D, slice: u32) -> Result<()> {
        let iv = self.input_view(input)?;
        let ov = self.output_view(output, slice)?;
        let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: ManuallyDrop::new(Some(iv)),
            ..Default::default()
        };
        let r = unsafe { self.vctx.VideoProcessorBlt(&self.vp, &ov, 0, std::slice::from_ref(&stream)) };
        unsafe { ManuallyDrop::drop(&mut stream.pInputSurface) };
        r.context("VideoProcessorBlt")
    }
}
