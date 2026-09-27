//! Mouse cursor tracking (from Desktop Duplication metadata) and GPU drawing
//! of the cursor on top of the captured desktop.

use crate::dup::{rotate_bgra, Rotation};
use anyhow::{anyhow, Context, Result};
use std::ffi::c_void;
use windows::core::{s, Interface, PCSTR};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    DXGI_OUTDUPL_POINTER_SHAPE_INFO, DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR, DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR,
    DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME,
};

pub struct CursorShape {
    pub width: u32,
    pub height: u32,
    /// Straight-alpha BGRA pixels.
    pub pixels: Vec<u8>,
}

#[derive(Default)]
pub struct CursorState {
    pub visible: bool,
    /// Top-left of the shape on the upright desktop (also on rotated displays).
    pub x: i32,
    pub y: i32,
    pub shape: Option<CursorShape>,
    /// Incremented whenever `shape` changes.
    pub shape_gen: u64,
}

/// Converts a DXGI pointer shape into straight-alpha BGRA. XOR-inverted
/// pixels cannot be reproduced without reading the destination, so they are
/// approximated with opaque colours.
pub fn convert_shape(info: &DXGI_OUTDUPL_POINTER_SHAPE_INFO, buf: &[u8]) -> Option<CursorShape> {
    let pitch = info.Pitch as usize;
    let t = info.Type as i32;
    if t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME.0 {
        let (w, h) = (info.Width as usize, info.Height as usize / 2);
        let mut px = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let mask = 0x80u8 >> (x % 8);
                let and = buf.get(y * pitch + x / 8)? & mask != 0;
                let xor = buf.get((y + h) * pitch + x / 8)? & mask != 0;
                let c: [u8; 4] = match (and, xor) {
                    (false, false) => [0, 0, 0, 255],
                    (false, true) => [255, 255, 255, 255],
                    (true, false) => [0, 0, 0, 0],
                    (true, true) => [0, 0, 0, 255],
                };
                px[(y * w + x) * 4..][..4].copy_from_slice(&c);
            }
        }
        Some(CursorShape { width: w as u32, height: h as u32, pixels: px })
    } else if t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR.0 || t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR.0 {
        let masked = t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR.0;
        let (w, h) = (info.Width as usize, info.Height as usize);
        let mut px = vec![0u8; w * h * 4];
        for y in 0..h {
            let row = buf.get(y * pitch..y * pitch + w * 4)?;
            let dst = &mut px[y * w * 4..(y + 1) * w * 4];
            dst.copy_from_slice(row);
            if masked {
                for p in dst.chunks_exact_mut(4) {
                    if p[3] == 0 {
                        p[3] = 255;
                    } else {
                        p[3] = if p[0] | p[1] | p[2] == 0 { 0 } else { 255 };
                    }
                }
            }
        }
        Some(CursorShape { width: w as u32, height: h as u32, pixels: px })
    } else {
        None
    }
}

const SHADER: &str = r#"
cbuffer C : register(b0) { float4 rect; };
struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
VSOut vs(uint id : SV_VertexID) {
    float2 uv = float2(id & 1, id >> 1);
    VSOut o;
    o.pos = float4(lerp(rect.x, rect.z, uv.x), lerp(rect.y, rect.w, uv.y), 0, 1);
    o.uv = uv;
    return o;
}
Texture2D tex : register(t0);
SamplerState smp : register(s0);
float4 ps(VSOut i) : SV_Target { return tex.Sample(smp, i.uv); }
"#;

fn compile(entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
    let mut code = None;
    let mut errors = None;
    let r = unsafe {
        D3DCompile(
            SHADER.as_ptr() as *const c_void,
            SHADER.len(),
            s!("cursor"),
            None,
            None,
            entry,
            target,
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(e) = r {
        let msg = errors
            .map(|b: ID3DBlob| unsafe {
                String::from_utf8_lossy(std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()))
                    .into_owned()
            })
            .unwrap_or_default();
        return Err(anyhow!("shader compile failed: {e} {msg}"));
    }
    code.ok_or_else(|| anyhow!("no shader code"))
}

fn blob_bytes(b: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
}

pub struct CursorRenderer {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    blend: ID3D11BlendState,
    sampler: ID3D11SamplerState,
    cbuf: ID3D11Buffer,
    tex: Option<(ID3D11ShaderResourceView, u32, u32)>,
    /// Shape generation and rotation `tex` was made for.
    tex_key: (u64, Rotation),
    rtv: Option<(usize, ID3D11RenderTargetView)>,
}

impl CursorRenderer {
    pub fn new(device: &ID3D11Device) -> Result<Self> {
        unsafe {
            let vsb = compile(s!("vs"), s!("vs_4_0"))?;
            let psb = compile(s!("ps"), s!("ps_4_0"))?;
            let mut vs = None;
            device.CreateVertexShader(blob_bytes(&vsb), None, Some(&mut vs))?;
            let mut ps = None;
            device.CreatePixelShader(blob_bytes(&psb), None, Some(&mut ps))?;

            let mut bd = D3D11_BLEND_DESC::default();
            bd.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
                BlendEnable: true.into(),
                SrcBlend: D3D11_BLEND_SRC_ALPHA,
                DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOp: D3D11_BLEND_OP_ADD,
                SrcBlendAlpha: D3D11_BLEND_ONE,
                DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOpAlpha: D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            let mut blend = None;
            device.CreateBlendState(&bd, Some(&mut blend))?;

            let sd = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_POINT,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MaxLOD: f32::MAX,
                ..Default::default()
            };
            let mut sampler = None;
            device.CreateSamplerState(&sd, Some(&mut sampler))?;

            let cd = D3D11_BUFFER_DESC {
                ByteWidth: 16,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            };
            let mut cbuf = None;
            device.CreateBuffer(&cd, None, Some(&mut cbuf))?;

            Ok(CursorRenderer {
                vs: vs.unwrap(),
                ps: ps.unwrap(),
                blend: blend.unwrap(),
                sampler: sampler.unwrap(),
                cbuf: cbuf.unwrap(),
                tex: None,
                tex_key: (u64::MAX, Rotation::None),
                rtv: None,
            })
        }
    }

    /// Uploads the shape turned the other way than `rotation`, so it comes
    /// out upright once the whole image is rotated.
    fn upload_shape(&mut self, device: &ID3D11Device, shape: &CursorShape, rotation: Rotation) -> Result<()> {
        let (pixels, width, height) = rotate_bgra(&shape.pixels, shape.width, shape.height, shape.width as usize * 4, rotation.inverse());
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr() as *const c_void,
            SysMemPitch: width * 4,
            SysMemSlicePitch: 0,
        };
        unsafe {
            let mut tex = None;
            device.CreateTexture2D(&desc, Some(&init), Some(&mut tex)).context("cursor texture")?;
            let tex = tex.unwrap();
            let mut srv = None;
            device.CreateShaderResourceView(&tex, None, Some(&mut srv))?;
            self.tex = Some((srv.unwrap(), shape.width, shape.height));
        }
        Ok(())
    }

    /// Alpha-blends the cursor onto `target`, a BGRA render target holding
    /// the duplicated image, which `rotation` turns upright.
    pub fn draw(
        &mut self,
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
        target: &ID3D11Texture2D,
        cursor: &CursorState,
        rotation: Rotation,
    ) -> Result<()> {
        let Some(shape) = cursor.shape.as_ref().filter(|_| cursor.visible) else { return Ok(()) };
        if self.tex_key != (cursor.shape_gen, rotation) {
            self.upload_shape(device, shape, rotation)?;
            self.tex_key = (cursor.shape_gen, rotation);
        }
        let Some((srv, w, h)) = self.tex.clone() else { return Ok(()) };
        let mut td = D3D11_TEXTURE2D_DESC::default();
        unsafe { target.GetDesc(&mut td) };
        let (tw, th) = (td.Width, td.Height);

        let key = target.as_raw() as usize;
        if self.rtv.as_ref().map(|r| r.0) != Some(key) {
            let mut rtv = None;
            unsafe { device.CreateRenderTargetView(target, None, Some(&mut rtv))? };
            self.rtv = Some((key, rtv.unwrap()));
        }
        let rtv = self.rtv.as_ref().unwrap().1.clone();

        let (fw, fh) = (tw as f32, th as f32);
        // `w`×`h` is the upright shape size; the uploaded texture is already turned.
        let (x, y, w, h) = rotation.to_image(cursor.x, cursor.y, w as i32, h as i32, tw as i32, th as i32);
        let (x0, y0) = (x as f32, y as f32);
        let (x1, y1) = (x0 + w as f32, y0 + h as f32);
        let rect = [x0 / fw * 2.0 - 1.0, 1.0 - y0 / fh * 2.0, x1 / fw * 2.0 - 1.0, 1.0 - y1 / fh * 2.0];

        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(&self.cbuf, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            std::ptr::copy_nonoverlapping(rect.as_ptr(), mapped.pData as *mut f32, 4);
            ctx.Unmap(&self.cbuf, 0);

            ctx.OMSetRenderTargets(Some(&[Some(rtv)]), None);
            ctx.OMSetBlendState(&self.blend, None, 0xffff_ffff);
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: fw,
                Height: fh,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            ctx.VSSetShader(&self.vs, None);
            ctx.VSSetConstantBuffers(0, Some(&[Some(self.cbuf.clone())]));
            ctx.PSSetShader(&self.ps, None);
            ctx.PSSetShaderResources(0, Some(&[Some(srv)]));
            ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            ctx.Draw(4, 0);
            // Unbind so the texture can be used as a video processor input.
            ctx.OMSetRenderTargets(None, None);
            ctx.PSSetShaderResources(0, Some(&[None]));
        }
        Ok(())
    }
}
