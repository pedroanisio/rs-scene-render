//! Environment maps: equirectangular HDRIs prefiltered for image-based
//! lighting (GGX-convolved specular mips, SH9 irradiance) and the split-sum
//! BRDF table.

use glam::{Vec2, Vec3};
use std::f32::consts::PI;

/// An equirectangular image, linear RGB, row 0 at the top (up = −y in scene space).
#[derive(Clone, Debug)]
pub struct Equirect {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<[f32; 3]>,
}

/// Loads `.hdr` (Radiance), `.exr` or an integer image (PNG, JPEG, AVIF, …) into linear RGB.
/// Float images are linear; 8- and 16-bit images are sRGB-decoded (conventions 5.5).
pub fn load(path: &std::path::Path) -> Result<Equirect, String> {
    let still = sr_media::still::open(path)?;
    let float = still.linear;
    let img = still.image.to_rgb32f();
    let decode = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    let rgb = img.pixels().map(|p| if float { p.0 } else { p.0.map(decode) }).collect();
    Ok(Equirect { width: img.width(), height: img.height(), rgb })
}

/// Unit direction of equirect coordinates (u, v ∈ [0, 1]); v = 0 is straight up (−y).
pub fn dir_of(u: f32, v: f32) -> Vec3 {
    let (phi, th) = (u * 2.0 * PI - PI, v * PI);
    Vec3::new(th.sin() * phi.sin(), -th.cos(), th.sin() * phi.cos())
}

/// Equirect coordinates of a direction.
pub fn uv_of(d: Vec3) -> Vec2 {
    let d = d.normalize_or_zero();
    Vec2::new((d.x.atan2(d.z) + PI) / (2.0 * PI), (-d.y).clamp(-1.0, 1.0).acos() / PI)
}

impl Equirect {
    fn at(&self, x: i64, y: i64) -> Vec3 {
        let xx = x.rem_euclid(self.width as i64) as u32;
        let yy = y.clamp(0, self.height as i64 - 1) as u32;
        Vec3::from(self.rgb[(yy * self.width + xx) as usize])
    }

    /// Bilinear sample at uv.
    pub fn sample(&self, uv: Vec2) -> Vec3 {
        let (x, y) = (uv.x * self.width as f32 - 0.5, uv.y * self.height as f32 - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let a = self.at(x0, y0).lerp(self.at(x0 + 1, y0), fx);
        let b = self.at(x0, y0 + 1).lerp(self.at(x0 + 1, y0 + 1), fx);
        a.lerp(b, fy)
    }

    /// Box-downsampled to `w`×`h`.
    pub fn resized(&self, w: u32, h: u32) -> Equirect {
        let (sx, sy) = (self.width as f32 / w as f32, self.height as f32 / h as f32);
        let mut rgb = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let (x0, x1) =
                    ((x as f32 * sx) as i64, (((x + 1) as f32 * sx).ceil() as i64).max(x as i64 * sx as i64 + 1));
                let (y0, y1) =
                    ((y as f32 * sy) as i64, (((y + 1) as f32 * sy).ceil() as i64).max(y as i64 * sy as i64 + 1));
                if x1 - x0 <= 1 && y1 - y0 <= 1 {
                    rgb.push(self.sample(Vec2::new((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32)).into());
                    continue;
                }
                let mut acc = Vec3::ZERO;
                let mut n = 0.0;
                for yy in y0..y1 {
                    for xx in x0..x1 {
                        acc += self.at(xx, yy);
                        n += 1.0;
                    }
                }
                rgb.push((acc / n).into());
            }
        }
        Equirect { width: w, height: h, rgb }
    }
}

fn hammersley(i: u32, n: u32) -> Vec2 {
    Vec2::new(i as f32 / n as f32, i.reverse_bits() as f32 * 2.328_306_4e-10)
}

fn ggx_sample(xi: Vec2, n: Vec3, a: f32) -> Vec3 {
    let phi = 2.0 * PI * xi.x;
    let cos_t = ((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y)).sqrt();
    let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
    let h = Vec3::new(sin_t * phi.cos(), sin_t * phi.sin(), cos_t);
    let up = if n.y.abs() < 0.999 { Vec3::Y } else { Vec3::X };
    let tx = up.cross(n).normalize();
    let ty = n.cross(tx);
    (tx * h.x + ty * h.y + n * h.z).normalize()
}

/// Prefiltered environment: specular mips (level k has roughness k/(levels−1)) and SH9 irradiance.
#[derive(Clone, Debug)]
pub struct Prefiltered {
    /// (width, height, RGBA32F) per mip, largest first.
    pub mips: Vec<(u32, u32, Vec<[f32; 4]>)>,
    /// Irradiance SH coefficients (already convolved with the cosine lobe, divided by π).
    pub sh: [[f32; 3]; 9],
}

fn sh_basis(d: Vec3) -> [f32; 9] {
    [
        0.282_095,
        0.488_603 * d.y,
        0.488_603 * d.z,
        0.488_603 * d.x,
        1.092_548 * d.x * d.y,
        1.092_548 * d.y * d.z,
        0.315_392 * (3.0 * d.z * d.z - 1.0),
        1.092_548 * d.x * d.z,
        0.546_274 * (d.x * d.x - d.y * d.y),
    ]
}

/// Evaluates the irradiance SH in a direction (returns E/π, the Lambertian radiance for albedo 1).
pub fn sh_eval(sh: &[[f32; 3]; 9], d: Vec3) -> Vec3 {
    let b = sh_basis(d.normalize_or_zero());
    (0..9).fold(Vec3::ZERO, |acc, k| acc + Vec3::from(sh[k]) * b[k])
}

/// Prefilters `env` into `levels` mips starting at `base_w`×`base_w/2`.
pub fn prefilter(env: &Equirect, base_w: u32, levels: usize) -> Prefiltered {
    // source pyramid for filtered importance sampling
    let mut src = vec![env.resized(base_w, base_w / 2)];
    while src.last().map(|e| e.width > 8).unwrap_or(false) {
        let l = src.last().unwrap();
        src.push(l.resized(l.width / 2, l.height / 2));
    }
    let texel_sa = |e: &Equirect| 4.0 * PI / (e.width * e.height) as f32;
    let mut mips = Vec::new();
    let samples = 48u32;
    for k in 0..levels {
        let (w, h) = ((base_w >> k).max(8), ((base_w / 2) >> k).max(4));
        let rough = k as f32 / (levels - 1).max(1) as f32;
        let a = (rough * rough).max(1e-4);
        let mut px = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let n = dir_of((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                if k == 0 {
                    let c = src[0].sample(uv_of(n));
                    px.push([c.x, c.y, c.z, 1.0]);
                    continue;
                }
                let (mut acc, mut wsum) = (Vec3::ZERO, 0.0);
                for i in 0..samples {
                    let hv = ggx_sample(hammersley(i, samples), n, a);
                    let l = 2.0 * n.dot(hv) * hv - n;
                    let nl = n.dot(l);
                    if nl <= 0.0 {
                        continue;
                    }
                    // pdf of the reflected direction for V = N: D(h)/4
                    let nh = n.dot(hv).max(0.0);
                    let d = a * a / (PI * ((nh * nh) * (a * a - 1.0) + 1.0).powi(2));
                    let pdf = d / 4.0;
                    let sa_sample = 1.0 / (samples as f32 * pdf.max(1e-6));
                    let lvl = (0.5 * (sa_sample / texel_sa(&src[0])).log2()).clamp(0.0, (src.len() - 1) as f32);
                    let c = src[lvl.round() as usize].sample(uv_of(l));
                    acc += c * nl;
                    wsum += nl;
                }
                let c = acc / wsum.max(1e-6);
                px.push([c.x, c.y, c.z, 1.0]);
            }
        }
        mips.push((w, h, px));
    }
    // SH9 projection from a small copy
    let small = &src[src.len().saturating_sub(3).min(src.len() - 1)];
    let mut sh = [[0.0f32; 3]; 9];
    for y in 0..small.height {
        let v = (y as f32 + 0.5) / small.height as f32;
        let sa = (2.0 * PI / small.width as f32) * (PI / small.height as f32) * (v * PI).sin();
        for x in 0..small.width {
            let d = dir_of((x as f32 + 0.5) / small.width as f32, v);
            let c = small.rgb[(y * small.width + x) as usize];
            let b = sh_basis(d);
            for (coef, bk) in sh.iter_mut().zip(b) {
                for (v, ci) in coef.iter_mut().zip(c) {
                    *v += ci * bk * sa;
                }
            }
        }
    }
    // cosine-lobe convolution (band factors π, 2π/3, π/4), then /π for radiance
    let band = [1.0, 2.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0, 0.25, 0.25, 0.25, 0.25, 0.25];
    for (coef, f) in sh.iter_mut().zip(band) {
        coef.iter_mut().for_each(|v| *v *= f);
    }
    Prefiltered { mips, sh }
}

/// Split-sum environment BRDF (scale, bias) over (N·V, roughness), `n`×`n`.
pub fn brdf_lut(n: usize) -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(n * n);
    let samples = 64;
    for j in 0..n {
        let rough = (j as f32 + 0.5) / n as f32;
        let a = rough * rough;
        for i in 0..n {
            let nv = ((i as f32 + 0.5) / n as f32).max(1e-3);
            let v = Vec3::new((1.0 - nv * nv).sqrt(), 0.0, nv);
            let (mut s, mut b) = (0.0, 0.0);
            for k in 0..samples {
                let h = ggx_sample(hammersley(k, samples), Vec3::Z, a);
                let l = 2.0 * v.dot(h) * h - v;
                let (nl, nh, vh) = (l.z.max(0.0), h.z.max(0.0), v.dot(h).max(0.0));
                if nl > 0.0 {
                    let kk = a / 2.0;
                    let g = (nv / (nv * (1.0 - kk) + kk)) * (nl / (nl * (1.0 - kk) + kk));
                    let gv = g * vh / (nh * nv).max(1e-6);
                    let fc = (1.0 - vh).powi(5);
                    s += (1.0 - fc) * gv;
                    b += fc * gv;
                }
            }
            out.push([s / samples as f32, b / samples as f32]);
        }
    }
    out
}
