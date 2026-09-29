//! Drawings rasterised on the CPU into an sRGB image: the texture a 3D map or
//! globe is draped with.
//!
//! Bitmaps marked `below` (raster map tiles) are warped in first, sampled
//! bilinearly; the vector commands are rasterised over them by the same fine
//! stage the GPU runs ([`sr_vector::tile::render_cpu`]), tile rows in parallel.
//! Compositing is in linear light; the result is sRGB-encoded RGBA8 with
//! straight alpha, ready for an sRGB texture. Document paints
//! (`Paint::External`) have no CPU form and draw mid-grey.

use std::collections::HashMap;

use rayon::prelude::*;
use sr_text::Drawing;
use sr_vector::scene::Paint;

fn lin(v: f64) -> f32 {
    sr_model_decode(v) as f32
}

fn sr_model_decode(v: f64) -> f64 {
    crate::color::decode(sr_model::model::Transfer::Srgb, v.clamp(0.0, 1.0))
}

fn enc(v: f32) -> u8 {
    (crate::color::encode(sr_model::model::Transfer::Srgb, v.clamp(0.0, 1.0) as f64) * 255.0).round() as u8
}

/// A decoded bitmap in linear premultiplied RGBA.
struct Image {
    w: u32,
    h: u32,
    px: Vec<[f32; 4]>,
}

impl Image {
    fn decode(bytes: &[u8]) -> Option<Image> {
        let img = image::load_from_memory(bytes).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        let table: Vec<f32> = (0..256).map(|i| lin(i as f64 / 255.0)).collect();
        let px = img
            .pixels()
            .map(|p| {
                let a = p[3] as f32 / 255.0;
                [table[p[0] as usize] * a, table[p[1] as usize] * a, table[p[2] as usize] * a, a]
            })
            .collect();
        Some(Image { w, h, px })
    }

    /// Bilinear sample at texture coordinates (0‥1).
    fn sample(&self, u: f64, v: f64) -> [f32; 4] {
        let x = (u * self.w as f64 - 0.5).clamp(0.0, self.w as f64 - 1.0);
        let y = (v * self.h as f64 - 0.5).clamp(0.0, self.h as f64 - 1.0);
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let at = |x: u32, y: u32| self.px[(y * self.w + x) as usize];
        let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
        [0, 1, 2, 3].map(|k| (a[k] * (1.0 - fx) + b[k] * fx) * (1.0 - fy) + (c[k] * (1.0 - fx) + d[k] * fx) * fy)
    }
}

/// Rasterises `d` at `size` pixels (drawing units are pixels).
pub fn rasterize(d: &Drawing, size: [u32; 2]) -> Vec<u8> {
    let [w, h] = size;
    let mut base = vec![[0.0f32; 4]; (w * h) as usize];
    // raster tiles beneath
    let mut images: HashMap<u64, Option<Image>> = HashMap::new();
    for b in d.bitmaps.iter().filter(|b| b.below) {
        let img = images.entry(b.key).or_insert_with(|| Image::decode(&b.png));
        let Some(img) = img else { continue };
        // the bitmap's unit square → pixels: xf ∘ rect
        let [rx, ry, rw, rh] = b.rect;
        let m = b.xf.mul(&sr_vector::geom::Xf([rw, 0.0, 0.0, rh, rx, ry]));
        let Some(inv) = m.inverse() else { continue };
        let corners = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]].map(|c| m.apply(sr_vector::geom::p(c[0], c[1])));
        let x0 = corners.iter().map(|c| c.x).fold(f64::INFINITY, f64::min).floor().max(0.0) as u32;
        let x1 = corners.iter().map(|c| c.x).fold(f64::NEG_INFINITY, f64::max).ceil().min(w as f64) as u32;
        let y0 = corners.iter().map(|c| c.y).fold(f64::INFINITY, f64::min).floor().max(0.0) as u32;
        let y1 = corners.iter().map(|c| c.y).fold(f64::NEG_INFINITY, f64::max).ceil().min(h as f64) as u32;
        let [u0, v0, u1, v1] = b.uv;
        for y in y0..y1 {
            for x in x0..x1 {
                let q = inv.apply(sr_vector::geom::p(x as f64 + 0.5, y as f64 + 0.5));
                if !(0.0..=1.0).contains(&q.x) || !(0.0..=1.0).contains(&q.y) {
                    continue;
                }
                let s = img.sample(u0 + (u1 - u0) * q.x, v0 + (v1 - v0) * q.y);
                let o = (b.opacity as f32).clamp(0.0, 1.0);
                let px = &mut base[(y * w + x) as usize];
                for k in 0..4 {
                    px[k] = s[k] * o + px[k] * (1.0 - s[3] * o);
                }
            }
        }
    }
    // vectors over them, one band of tiles at a time
    let e = sr_vector::tile::encode(&d.scene, size);
    let paint = |i: u32, _x: f32, _y: f32| -> [f32; 4] {
        match e.paints.get(i as usize) {
            Some(Paint::Solid { rgba, .. }) => [lin(rgba[0]), lin(rgba[1]), lin(rgba[2]), rgba[3] as f32],
            _ => [0.2, 0.2, 0.2, 1.0],
        }
    };
    let over = render_parallel(&e, &paint);
    base.par_iter_mut().zip(over.par_iter()).for_each(|(b, v)| {
        for k in 0..4 {
            b[k] = v[k] + b[k] * (1.0 - v[3]);
        }
    });
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for p in base {
        let a = p[3];
        let un = |c: f32| if a > 0.0 { c / a } else { 0.0 };
        out.extend([enc(un(p[0])), enc(un(p[1])), enc(un(p[2])), (a.clamp(0.0, 1.0) * 255.0).round() as u8]);
    }
    out
}

/// [`sr_vector::tile::render_cpu`] in bands of tile rows rendered in parallel.
fn render_parallel(e: &sr_vector::tile::Encoded, paint: &(dyn Fn(u32, f32, f32) -> [f32; 4] + Sync)) -> Vec<[f32; 4]> {
    let rows = e.tiles[1];
    let bands: Vec<Vec<[f32; 4]>> =
        (0..rows).into_par_iter().map(|ty| sr_vector::tile::render_cpu_rows(e, ty..ty + 1, paint)).collect();
    bands.concat()
}
