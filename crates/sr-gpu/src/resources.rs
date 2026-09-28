//! GPU textures: offscreen targets from a pool, and source images decoded
//! into the working space with a full mip chain.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use sr_model::model::{AlphaMode, ColorSpace, Transfer};

use crate::color::{self, Working};

/// Frame and offscreen format.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// A texture with its view and source bind group.
#[derive(Debug)]
pub struct Tex {
    /// Texture.
    pub tex: wgpu::Texture,
    /// Full view.
    pub view: wgpu::TextureView,
    /// Size in pixels.
    pub size: [u32; 2],
    /// Bind group for group(1) (sampled as a layer source).
    pub bind: wgpu::BindGroup,
}

/// The bind group layout of a sampled layer source (one filterable 2D texture).
pub fn source_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("source"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    })
}

/// Creates a texture usable as render target, source and copy endpoint.
pub fn create(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, size: [u32; 2], mips: u32, label: &str) -> Tex {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
        mip_level_count: mips.max(1),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
    });
    Tex { tex, view, size, bind }
}

/// Reuses offscreen textures by size between frames.
#[derive(Default)]
pub struct Pool {
    free: HashMap<[u32; 2], Vec<Arc<Tex>>>,
    /// Textures created since the renderer started.
    pub created: usize,
}

impl Pool {
    /// A texture of `size`, reused when one is free.
    pub fn get(&mut self, device: &wgpu::Device, layout: &wgpu::BindGroupLayout, size: [u32; 2]) -> Arc<Tex> {
        if let Some(t) = self.free.get_mut(&size).and_then(Vec::pop) {
            return t;
        }
        self.created += 1;
        Arc::new(create(device, layout, size, 1, "offscreen"))
    }

    /// Returns a texture to the pool.
    pub fn put(&mut self, t: Arc<Tex>) {
        if Arc::strong_count(&t) == 1 {
            self.free.entry(t.size).or_default().push(t);
        }
    }
}

/// Decoded, working-space, premultiplied RGBA with its mip chain.
pub struct Decoded {
    /// Levels, largest first: (width, height, RGBA f32 premultiplied).
    pub levels: Vec<(u32, u32, Vec<[f32; 4]>)>,
}

/// Decodes an image file into the working space.
pub fn decode_image(
    path: &Path,
    space: ColorSpace,
    transfer: Transfer,
    alpha: AlphaMode,
    working: &Working,
    max_dim: u32,
) -> Result<Decoded, String> {
    let img = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    decode_dynamic(img, space, transfer, alpha, working, max_dim)
}

/// Decodes an encoded image held in memory (bitmap glyphs).
pub fn decode_image_bytes(bytes: &[u8], working: &Working, max_dim: u32) -> Result<Decoded, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    decode_dynamic(img, ColorSpace::Srgb, Transfer::Auto, AlphaMode::Auto, working, max_dim)
}

fn decode_dynamic(
    img: image::DynamicImage,
    space: ColorSpace,
    transfer: Transfer,
    alpha: AlphaMode,
    working: &Working,
    max_dim: u32,
) -> Result<Decoded, String> {
    let mut rgba = img.to_rgba32f();
    let (w, h) = rgba.dimensions();
    if w > max_dim || h > max_dim {
        let s = max_dim as f64 / w.max(h) as f64;
        rgba = image::imageops::resize(
            &rgba,
            ((w as f64 * s) as u32).max(1),
            ((h as f64 * s) as u32).max(1),
            image::imageops::FilterType::Triangle,
        );
    }
    let (w, h) = rgba.dimensions();
    let t = color::resolve(space, transfer);
    let m = color::convert(space, working.space);
    let mut lut = [0f64; 65536];
    let use_lut = !matches!(t, Transfer::Linear);
    if use_lut {
        for (i, v) in lut.iter_mut().enumerate() {
            *v = color::decode(t, i as f64 / 65535.0);
        }
    }
    let premultiplied_in = alpha == AlphaMode::Premultiplied;
    let ignore_alpha = alpha == AlphaMode::None;
    let px: Vec<[f32; 4]> = rgba
        .pixels()
        .map(|p| {
            let a = if ignore_alpha { 1.0 } else { p[3] as f64 };
            let mut c = [p[0] as f64, p[1] as f64, p[2] as f64];
            if premultiplied_in && a > 0.0 {
                c = c.map(|v| v / a);
            }
            let lin = if use_lut { c.map(|v| lut[(v.clamp(0.0, 1.0) * 65535.0).round() as usize]) } else { c };
            let wv = color::apply(&m, lin);
            let stored = working.store([wv[0], wv[1], wv[2], a]);
            [(stored[0] * a) as f32, (stored[1] * a) as f32, (stored[2] * a) as f32, a as f32]
        })
        .collect();
    Ok(Decoded { levels: mips(w, h, px) })
}

/// Box-filtered mip chain.
pub fn mips(w: u32, h: u32, base: Vec<[f32; 4]>) -> Vec<(u32, u32, Vec<[f32; 4]>)> {
    let mut levels = vec![(w, h, base)];
    loop {
        let (pw, ph, prev) = levels.last().unwrap();
        let (pw, ph) = (*pw, *ph);
        if pw == 1 && ph == 1 {
            break;
        }
        let (nw, nh) = ((pw / 2).max(1), (ph / 2).max(1));
        let mut next = Vec::with_capacity((nw * nh) as usize);
        for y in 0..nh {
            for x in 0..nw {
                let mut acc = [0f32; 4];
                let mut n = 0.0;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let (sx, sy) = ((x * 2 + dx).min(pw - 1), (y * 2 + dy).min(ph - 1));
                        let p = prev[(sy * pw + sx) as usize];
                        for k in 0..4 {
                            acc[k] += p[k];
                        }
                        n += 1.0;
                    }
                }
                next.push(acc.map(|v| v / n));
            }
        }
        levels.push((nw, nh, next));
    }
    levels
}

/// Uploads decoded levels into a new texture.
pub fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    d: &Decoded,
    label: &str,
) -> Tex {
    let (w, h, _) = d.levels[0];
    let t = create(device, layout, [w, h], d.levels.len() as u32, label);
    for (level, (lw, lh, px)) in d.levels.iter().enumerate() {
        let bytes: Vec<u8> =
            px.iter().flat_map(|p| p.iter().flat_map(|v| half::f16::from_f32(*v).to_le_bytes())).collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &t.tex,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(lw * 8), rows_per_image: Some(*lh) },
            wgpu::Extent3d { width: *lw, height: *lh, depth_or_array_layers: 1 },
        );
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_averages() {
        let base = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0]];
        let l = mips(2, 2, base);
        assert_eq!(l.len(), 2);
        assert_eq!(l[1].2[0], [0.5, 0.5, 0.5, 1.0]);
        assert_eq!(
            mips(5, 3, vec![[0.0; 4]; 15]).iter().map(|l| (l.0, l.1)).collect::<Vec<_>>(),
            vec![(5, 3), (2, 1), (1, 1)]
        );
    }
}
