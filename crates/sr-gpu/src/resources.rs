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

/// The bind group layout of a sampled layer source: one filterable 2D texture, bound twice.
/// Binding 0 is sampled clamped and binding 1 repeating (pattern paints); OpenGL allows one
/// sampler per texture binding, so the repeating sampler needs a binding of its own.
pub fn source_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let tex = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("source"),
        entries: &[tex(0), tex(1)],
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
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
        ],
    });
    Tex { tex, view, size, bind }
}

/// Frames a size may go unrequested, or a free texture untaken, before it is released.
const KEEP_FRAMES: u64 = 8;

/// Reuses offscreen textures by size between frames.
#[derive(Default)]
pub struct Pool {
    /// Free textures by size, each with the frame it was returned in, oldest first.
    free: HashMap<[u32; 2], Vec<(Arc<Tex>, u64)>>,
    /// The frame (count of `trim` calls) each size was last requested in.
    wanted: HashMap<[u32; 2], u64>,
    /// `trim` calls so far.
    frame: u64,
    /// Textures created since the renderer started.
    pub created: usize,
    /// Textures destroyed by `trim` since the renderer started.
    pub released: usize,
}

impl Pool {
    /// A texture of `size`, reused when one is free.
    pub fn get(&mut self, device: &wgpu::Device, layout: &wgpu::BindGroupLayout, size: [u32; 2]) -> Arc<Tex> {
        self.wanted.insert(size, self.frame);
        if let Some((t, _)) = self.free.get_mut(&size).and_then(Vec::pop) {
            return t;
        }
        self.created += 1;
        Arc::new(create(device, layout, size, 1, "offscreen"))
    }

    /// Ends a frame: frees the textures of every size not requested in the last `KEEP_FRAMES`
    /// frames. Sizes that follow moving content (motion-blur and effect bounds, rounded to
    /// steps so they repeat) come and go, and would otherwise accumulate over a long render;
    /// keeping them a few frames lets a size that returns find its textures.
    ///
    /// Of a size still requested, frees the textures returned that long ago and not taken since.
    /// More can come in than go out: a cached target that is evicted is returned here whether or not
    /// it was taken from here, and a size in steady use would otherwise collect those for ever. The
    /// newest are taken first, so the ones a frame reuses never age.
    pub fn trim(&mut self) {
        let now = self.frame;
        self.wanted.retain(|_, at| now - *at < KEEP_FRAMES);
        let wanted = &self.wanted;
        let mut released = 0;
        self.free.retain(|size, list| {
            let before = list.len();
            if wanted.contains_key(size) {
                list.retain(|(_, at)| now - *at < KEEP_FRAMES);
            } else {
                list.clear();
            }
            released += before - list.len();
            !list.is_empty()
        });
        self.released += released;
        self.frame += 1;
    }

    /// Textures held for reuse.
    pub fn held(&self) -> usize {
        self.free.values().map(Vec::len).sum()
    }

    /// Texels of the textures held for reuse.
    pub fn held_texels(&self) -> u64 {
        self.free.iter().map(|(s, v)| s[0] as u64 * s[1] as u64 * v.len() as u64).sum()
    }

    /// Returns a texture to the pool.
    pub fn put(&mut self, t: Arc<Tex>) {
        if Arc::strong_count(&t) == 1 {
            self.free.entry(t.size).or_default().push((t, self.frame));
        }
    }
}

/// Decoded, working-space, premultiplied RGBA with its mip chain.
pub struct Decoded {
    /// Levels, largest first: (width, height, RGBA f32 premultiplied).
    pub levels: Vec<(u32, u32, Vec<[f32; 4]>)>,
    /// Why an embedded colour profile could not be honoured, when it could not.
    pub note: Option<String>,
}

/// How the values of a file become light: the colour space and transfer the document declares,
/// or the file's own profile.
#[derive(Clone, Debug, PartialEq)]
pub enum Coding {
    /// A named space and transfer (declared, or an embedded description that matches one).
    Named(ColorSpace, Transfer),
    /// A matrix/TRC ICC profile that matches no named space: linear RGB → XYZ (D65) and tone curves.
    Profile(color::M3, Box<[sr_media::icc::Curve; 3]>),
}

/// The H.273 colour primaries as a named space.
fn cicp_space(p: u16) -> Option<ColorSpace> {
    Some(match p {
        1 => ColorSpace::Srgb,
        9 => ColorSpace::Rec2020,
        11 => ColorSpace::DciP3,
        12 => ColorSpace::DisplayP3,
        _ => return None,
    })
}

/// The H.273 transfer characteristics as a transfer.
fn cicp_transfer(t: u16) -> Option<Transfer> {
    Some(match t {
        1 | 6 | 14 | 15 => Transfer::Bt1886,
        4 => Transfer::Gamma22,
        8 => Transfer::Linear,
        13 => Transfer::Srgb,
        16 => Transfer::Pq,
        18 => Transfer::Hlg,
        _ => return None,
    })
}

/// The named space and transfer an ICC profile equals, if any, within the rounding of ICC's
/// fixed-point colorants and 16-bit curves.
fn named_profile(p: &sr_media::icc::Profile, m: &color::M3) -> Option<(ColorSpace, Transfer)> {
    let space = if p.gray {
        ColorSpace::Srgb
    } else {
        *[ColorSpace::Srgb, ColorSpace::DisplayP3, ColorSpace::Rec2020, ColorSpace::DciP3].iter().find(|s| {
            let n = color::to_xyz_d65(**s);
            (0..3).all(|i| (0..3).all(|j| (n[i][j] - m[i][j]).abs() < 3e-3))
        })?
    };
    let transfer = *[Transfer::Srgb, Transfer::Linear, Transfer::Gamma22, Transfer::Bt1886, Transfer::Gamma26]
        .iter()
        .find(|t| {
        p.curves
            .iter()
            .all(|c| (0..=32).all(|i| (c.eval(i as f64 / 32.0) - color::decode(**t, i as f64 / 32.0)).abs() < 2e-3))
    })?;
    Some((space, transfer))
}

/// Chooses the coding of a decoded still: its embedded description when `embedded` and usable,
/// else the declared one. A linear-light format (EXR, HDR) with `transfer="auto"` is linear.
pub fn coding(still: &sr_media::still::Still, space: ColorSpace, transfer: Transfer, embedded: bool) -> Coding {
    use sr_media::still::Colour;
    let declared = || {
        let t = if transfer == Transfer::Auto && still.linear { Transfer::Linear } else { transfer };
        Coding::Named(space, t)
    };
    if !embedded || space == ColorSpace::Raw {
        return declared();
    }
    match &still.colour {
        Colour::Cicp(p, t) => match (cicp_space(*p), cicp_transfer(*t)) {
            (Some(s), Some(t)) => Coding::Named(s, t),
            (Some(s), None) => Coding::Named(s, transfer),
            (None, Some(t)) => Coding::Named(space, t),
            (None, None) => declared(),
        },
        Colour::Icc(p) => {
            let m = color::mul(&color::icc_d50_to_d65(), &p.to_xyz_d50);
            match named_profile(p, &m) {
                Some((s, t)) => Coding::Named(s, t),
                None => Coding::Profile(m, Box::new(p.curves.clone())),
            }
        }
        Colour::Unknown | Colour::Unsupported(_) => declared(),
    }
}

/// Decodes an image file into the working space. `embedded` lets the file's own colour profile
/// (ICC, or H.273 code points) take precedence over `space` and `transfer`.
pub fn decode_image(
    path: &Path,
    space: ColorSpace,
    transfer: Transfer,
    alpha: AlphaMode,
    embedded: bool,
    working: &Working,
    max_dim: u32,
) -> Result<Decoded, String> {
    let still = sr_media::still::open(path)?;
    let coding = coding(&still, space, transfer, embedded);
    let mut d = decode_dynamic(still.image, &coding, alpha, working, max_dim)?;
    if let (sr_media::still::Colour::Unsupported(why), true) = (&still.colour, embedded) {
        d.note = Some(format!("embedded colour profile not used ({why}); colorSpace applies"));
    }
    Ok(d)
}

/// Decodes an encoded image held in memory (bitmap glyphs).
pub fn decode_image_bytes(bytes: &[u8], working: &Working, max_dim: u32) -> Result<Decoded, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    decode_dynamic(img, &Coding::Named(ColorSpace::Srgb, Transfer::Auto), AlphaMode::Auto, working, max_dim)
}

/// `img` reduced, in its own sample format, to at most `max_dim` a side. Reducing before the
/// conversion to float keeps a very large image from being held as 16 bytes a pixel.
pub fn fit(img: image::DynamicImage, max_dim: u32) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());
    if w <= max_dim && h <= max_dim {
        return img;
    }
    let s = max_dim as f64 / w.max(h) as f64;
    img.thumbnail_exact(((w as f64 * s) as u32).max(1), ((h as f64 * s) as u32).max(1))
}

fn decode_dynamic(
    img: image::DynamicImage,
    coding: &Coding,
    alpha: AlphaMode,
    working: &Working,
    max_dim: u32,
) -> Result<Decoded, String> {
    let rgba = fit(img, max_dim).to_rgba32f();
    let (w, h) = rgba.dimensions();
    let table = |f: &dyn Fn(f64) -> f64| -> Vec<f64> { (0..65536).map(|i| f(i as f64 / 65535.0)).collect() };
    // Per-channel decoding tables (none when the values are already linear) and the matrix to the working space.
    let (luts, m): (Option<[std::sync::Arc<Vec<f64>>; 3]>, color::M3) = match coding {
        Coding::Named(space, transfer) => {
            let t = color::resolve(*space, *transfer);
            let lut = (t != Transfer::Linear).then(|| std::sync::Arc::new(table(&|v| color::decode(t, v))));
            (lut.map(|l| [l.clone(), l.clone(), l]), color::convert(*space, working.space))
        }
        Coding::Profile(to_xyz, curves) => {
            let l = (**curves).clone().map(|c| std::sync::Arc::new(table(&|v| c.eval(v))));
            (Some(l), color::mul(&color::inv(&color::to_xyz_d65(working.space)), to_xyz))
        }
    };
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
            let lin = match &luts {
                Some(l) => [0, 1, 2].map(|k| l[k][(c[k].clamp(0.0, 1.0) * 65535.0).round() as usize]),
                None => c,
            };
            let wv = color::apply(&m, lin);
            let stored = working.store([wv[0], wv[1], wv[2], a]);
            [(stored[0] * a) as f32, (stored[1] * a) as f32, (stored[2] * a) as f32, a as f32]
        })
        .collect();
    Ok(Decoded { levels: mips(w, h, px), note: None })
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
