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

#[cfg(test)]
thread_local! { static COLOR_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

fn lin(v: f64) -> f32 {
    #[cfg(test)]
    COLOR_DECODES.with(|n| n.set(n.get() + 1));
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
    if w == 0 || h == 0 {
        return Vec::new();
    }
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
    // Solid paint is independent of position. Decode each color once, before
    // the fine rasterizer visits every covered pixel (and each overlapping fill).
    let colors: Vec<[f32; 4]> = e
        .paints
        .iter()
        .map(|paint| match paint {
            Paint::Solid { rgba, .. } => [lin(rgba[0]), lin(rgba[1]), lin(rgba[2]), rgba[3] as f32],
            _ => [0.2, 0.2, 0.2, 1.0],
        })
        .collect();
    let paint = |i: u32, _x: f32, _y: f32| colors.get(i as usize).copied().unwrap_or([0.2, 0.2, 0.2, 1.0]);
    // Each worker owns just one band. Composite and encode straight into the
    // final disjoint byte slices, without collecting another full float frame.
    let mut out = vec![0u8; w as usize * h as usize * 4];
    let band_pixels = w as usize * sr_vector::tile::TILE as usize;
    out.par_chunks_mut(band_pixels * 4).zip(base.par_chunks(band_pixels)).enumerate().for_each(|(ty, (dst, below))| {
        let over = sr_vector::tile::render_cpu_rows(&e, ty as u32..ty as u32 + 1, &paint);
        for ((dst, b), v) in dst.chunks_mut(4).zip(below).zip(over) {
            let p: [f32; 4] = std::array::from_fn(|k| v[k] + b[k] * (1.0 - v[3]));
            let a = p[3];
            let un = |c: f32| if a > 0.0 { c / a } else { 0.0 };
            dst.copy_from_slice(&[
                enc(un(p[0])),
                enc(un(p[1])),
                enc(un(p[2])),
                (a.clamp(0.0, 1.0) * 255.0).round() as u8,
            ]);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sr_vector::{shapes, Cmd, FillRule, MaskOp, MatteMode, Scene};

    #[test]
    fn solid_paints_decode_once_and_masked_matted_pixels_match_reference() {
        let fill = || Cmd::Fill {
            polys: shapes::rect(0., 0., 65., 33., [0.; 4]).flatten(0.1),
            rule: FillRule::NonZero,
            paint: Paint::Solid { rgba: [0.2, 0.4, 0.6, 0.5], srgb: true },
            opacity: 0.8,
        };
        let d = Drawing {
            scene: Scene {
                cmds: vec![
                    Cmd::Push { mask_init: 0. },
                    fill(),
                    fill(),
                    Cmd::Mask {
                        polys: shapes::rect(3.25, 2.5, 50., 20., [0.; 4]).flatten(0.1),
                        rule: FillRule::NonZero,
                        op: MaskOp::Add,
                        opacity: 0.6,
                        invert: false,
                    },
                    Cmd::PushMatte,
                    fill(),
                    Cmd::PopMatte { mode: MatteMode::Luma, opacity: 0.7 },
                ],
            },
            ..Default::default()
        };
        let e = sr_vector::tile::encode(&d.scene, [65, 33]);
        let reference = sr_vector::tile::render_cpu(&e, &|i, _, _| match &e.paints[i as usize] {
            Paint::Solid { rgba, .. } => [
                sr_model_decode(rgba[0]) as f32,
                sr_model_decode(rgba[1]) as f32,
                sr_model_decode(rgba[2]) as f32,
                rgba[3] as f32,
            ],
            _ => [0.2, 0.2, 0.2, 1.],
        });
        let expected: Vec<u8> = reference
            .iter()
            .flat_map(|p| {
                let un = |c: f32| if p[3] > 0. { c / p[3] } else { 0. };
                [enc(un(p[0])), enc(un(p[1])), enc(un(p[2])), (p[3].clamp(0., 1.) * 255.).round() as u8]
            })
            .collect();
        COLOR_DECODES.with(|n| n.set(0));
        let actual = rasterize(&d, [65, 33]);
        assert_eq!(actual, expected);
        assert_eq!(
            COLOR_DECODES.with(|n| n.get()),
            3 * e.paints.len(),
            "decode solid colors once per paint, not per pixel"
        );
    }
    #[test]
    fn fused_bands_preserve_overlapping_bitmap_and_vector_compositing() {
        let mut d = Drawing::default();
        for (key, rgba, rect, xf) in [
            (1, [40, 100, 220, 160], [0., 0., 65., 33.], sr_vector::geom::Xf::IDENTITY),
            (2, [220, 80, 40, 190], [7.25, 3.5, 30.5, 20.75], sr_vector::geom::Xf([0.9, 0.1, -0.15, 1., 4., -2.])),
        ] {
            let img = image::RgbaImage::from_pixel(2, 2, image::Rgba(rgba));
            let mut png = std::io::Cursor::new(Vec::new());
            img.write_to(&mut png, image::ImageFormat::Png).unwrap();
            d.bitmaps.push(sr_text::Bitmap {
                png: std::sync::Arc::new(png.into_inner()),
                key,
                rect,
                xf,
                opacity: 0.7,
                uv: [0., 0., 1., 1.],
                below: true,
            });
        }
        d.scene.cmds.push(Cmd::Fill {
            polys: shapes::rect(3.25, 2.5, 50., 20., [0.; 4]).flatten(0.1),
            rule: FillRule::NonZero,
            paint: Paint::Solid { rgba: [0.2, 0.5, 0.7, 0.6], srgb: true },
            opacity: 0.8,
        });
        let e = sr_vector::tile::encode(&d.scene, [65, 33]);
        let over = sr_vector::tile::render_cpu(&e, &|_, _, _| [lin(0.2), lin(0.5), lin(0.7), 0.6]);
        let mut base = vec![[0.0f32; 4]; 65 * 33];
        // Original separate bitmap, composition and encoding passes.
        for b in &d.bitmaps {
            let img = Image::decode(&b.png).unwrap();
            let [x, y, w, h] = b.rect;
            let inv = b.xf.mul(&sr_vector::geom::Xf([w, 0., 0., h, x, y])).inverse().unwrap();
            for y in 0..33 {
                for x in 0..65 {
                    let q = inv.apply(sr_vector::geom::p(x as f64 + 0.5, y as f64 + 0.5));
                    if !(0.0..=1.0).contains(&q.x) || !(0.0..=1.0).contains(&q.y) {
                        continue;
                    }
                    let s = img.sample(q.x, q.y);
                    let o = b.opacity as f32;
                    let px = &mut base[y * 65 + x];
                    for k in 0..4 {
                        px[k] = s[k] * o + px[k] * (1. - s[3] * o);
                    }
                }
            }
        }
        for (b, v) in base.iter_mut().zip(over) {
            for k in 0..4 {
                b[k] = v[k] + b[k] * (1. - v[3]);
            }
        }
        let expected: Vec<u8> = base
            .iter()
            .flat_map(|p| {
                let un = |c: f32| if p[3] > 0. { c / p[3] } else { 0. };
                [enc(un(p[0])), enc(un(p[1])), enc(un(p[2])), (p[3].clamp(0., 1.) * 255.).round() as u8]
            })
            .collect();
        assert_eq!(rasterize(&d, [65, 33]), expected);
    }
}
