//! The procedural sky of a dome light (SREP 71, Semantics 2): a zenith, horizon and ground gradient with an optional
//! sun disc, defined for every direction in closed form so that it needs no image file.
//!
//! Directions are in the dome's own space (the space of an environment image): +x right, +y down, +z forward, so the
//! sine of the elevation of a unit direction d is −d.y.

use glam::Vec3;

use crate::env::{dir_of, Equirect};

/// A gradient sky in linear working values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sky {
    /// Radiance straight up (`skyZenith`).
    pub zenith: [f32; 3],
    /// Radiance at the horizon (`skyHorizon`).
    pub horizon: [f32; 3],
    /// Radiance straight down (`skyGround`).
    pub ground: [f32; 3],
    /// `skyExponent` (> 0).
    pub exponent: f32,
    /// Unit direction toward the sun's centre.
    pub sun_dir: [f32; 3],
    /// Cosine of half the sun's angular diameter: d is in the disc when d · sun_dir ≥ this.
    pub sun_cos: f32,
    /// What the sun adds inside its disc: `sunColor` · `sunIntensity`. Zero for no sun.
    pub sun: [f32; 3],
}

impl Sky {
    /// The sky of a dome; `sun_size`, `azimuth` and `elevation` in degrees.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        zenith: [f32; 3],
        horizon: [f32; 3],
        ground: [f32; 3],
        exponent: f32,
        azimuth: f64,
        elevation: f64,
        sun_size: f64,
        sun: [f32; 3],
    ) -> Sky {
        Sky {
            zenith,
            horizon,
            ground,
            exponent,
            sun_dir: sun_direction(azimuth, elevation),
            sun_cos: (sun_size.to_radians() * 0.5).cos() as f32,
            sun: if sun_size > 0.0 { sun } else { [0.0; 3] },
        }
    }

    /// Whether the sky has a sun disc.
    pub fn has_sun(&self) -> bool {
        self.sun.iter().any(|c| *c > 0.0)
    }

    /// The gradient alone at a unit direction d: H + (Z − H)·s^k above the horizon, H + (G − H)·(−s)^k below it.
    pub fn gradient(&self, d: Vec3) -> Vec3 {
        let s = -d.y;
        let (far, t) = if s >= 0.0 { (self.zenith, s) } else { (self.ground, -s) };
        let w = t.min(1.0).powf(self.exponent);
        let h = Vec3::from(self.horizon);
        h + (Vec3::from(far) - h) * w
    }

    /// The radiance at a unit direction: the gradient, plus the sun inside its disc.
    pub fn radiance(&self, d: Vec3) -> Vec3 {
        let mut c = self.gradient(d);
        if self.has_sun() && d.dot(Vec3::from(self.sun_dir)) >= self.sun_cos {
            c += Vec3::from(self.sun);
        }
        c
    }

    /// The solid angle of the sun disc in steradians.
    pub fn sun_solid_angle(&self) -> f32 {
        2.0 * std::f32::consts::PI * (1.0 - self.sun_cos)
    }

    /// The sky as an equirectangular image `width` × `width / 2` for image-based lighting.
    ///
    /// The gradient is sampled at texel centres. The sun is spread over the texels its disc covers (4 × 4 samples each)
    /// and scaled so that the image holds exactly its power (radiance × solid angle): a sun smaller than a texel
    /// lights as much as a large one of the same power.
    pub fn equirect(&self, width: u32) -> Equirect {
        let (w, h) = (width.max(4), (width / 2).max(2));
        let mut rgb = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let d = dir_of((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                rgb.push(self.gradient(d).to_array());
            }
        }
        if self.has_sun() {
            let texel_sa = |y: u32| {
                let v = (y as f32 + 0.5) / h as f32;
                (2.0 * std::f32::consts::PI / w as f32)
                    * (std::f32::consts::PI / h as f32)
                    * (v * std::f32::consts::PI).sin()
            };
            let sun = Vec3::from(self.sun_dir);
            // a texel's centre lies within the disc's radius plus the texel's diagonal of every texel the disc touches
            let reach = (1.0 - self.sun_cos.clamp(-1.0, 1.0)).max(0.0);
            let radius = (2.0 * reach).sqrt() + 2.0 * std::f32::consts::PI / w as f32 * 1.5;
            let mut cover: Vec<(usize, f32)> = Vec::new();
            for y in 0..h {
                for x in 0..w {
                    let d = dir_of((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                    if d.distance(sun) > radius {
                        continue;
                    }
                    const N: u32 = 4;
                    let mut inside = 0;
                    for j in 0..N {
                        for i in 0..N {
                            let u = (x as f32 + (i as f32 + 0.5) / N as f32) / w as f32;
                            let v = (y as f32 + (j as f32 + 0.5) / N as f32) / h as f32;
                            if dir_of(u, v).dot(sun) >= self.sun_cos {
                                inside += 1;
                            }
                        }
                    }
                    if inside > 0 {
                        cover.push(((y * w + x) as usize, inside as f32 / (N * N) as f32 * texel_sa(y)));
                    }
                }
            }
            if cover.is_empty() {
                // smaller than a sample: all of it in the texel of the centre
                let uv = crate::env::uv_of(sun);
                let x = ((uv.x * w as f32) as u32).min(w - 1);
                let y = ((uv.y * h as f32) as u32).min(h - 1);
                cover.push(((y * w + x) as usize, texel_sa(y)));
            }
            let total: f32 = cover.iter().map(|c| c.1).sum();
            let power = Vec3::from(self.sun) * self.sun_solid_angle();
            for (i, sa) in cover {
                let add = power * (sa / total) / texel_sa((i / w as usize) as u32);
                rgb[i] = (Vec3::from(rgb[i]) + add).to_array();
            }
        }
        Equirect { width: w, height: h, rgb }
    }
}

/// The sky's parameters as five vec4 for a shader (`sky_radiance` of sr-gpu's `sampling.wgsl`): zenith (rgb, exponent), horizon (rgb, 1), ground,
/// the sun's direction (xyz, cosine of half its size) and the sun's radiance.
impl Sky {
    pub fn uniforms(&self) -> [[f32; 4]; 5] {
        let v = |c: [f32; 3], w: f32| [c[0], c[1], c[2], w];
        [
            v(self.zenith, self.exponent),
            v(self.horizon, 1.0),
            v(self.ground, 0.0),
            v(self.sun_dir, self.sun_cos),
            v(self.sun, 0.0),
        ]
    }
}

/// The sun's direction for an azimuth and elevation in degrees: (cos e·sin a, −sin e, cos e·cos a). Azimuth 0 is +z,
/// positive azimuth turns toward +x, and positive elevation is up (−y).
pub fn sun_direction(azimuth: f64, elevation: f64) -> [f32; 3] {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    [(e.cos() * a.sin()) as f32, (-e.sin()) as f32, (e.cos() * a.cos()) as f32]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(c: f64) -> f32 {
        let c = c / 255.0;
        (if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }) as f32
    }

    fn enc(v: f32) -> f32 {
        let v = v.clamp(0.0, 1.0) as f64;
        (255.0 * if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }) as f32
    }

    fn lin(hex: u32) -> [f32; 3] {
        [dec(((hex >> 16) & 255) as f64), dec(((hex >> 8) & 255) as f64), dec((hex & 255) as f64)]
    }

    /// The kit's sky: zenith #2050A0, horizon #C0D0E0, ground #403020, exponent 1, no sun.
    fn kit_sky() -> Sky {
        Sky::new(lin(0x2050A0), lin(0xC0D0E0), lin(0x403020), 1.0, 0.0, 45.0, 0.53, [0.0; 3])
    }

    /// The ray of a pixel centre of the kit's implicit camera: 640 × 360, horizontal field of view 60°.
    fn ray(px: f32, py: f32) -> Vec3 {
        let f = 320.0 / 30f32.to_radians().tan();
        Vec3::new(px - 320.0, py - 180.0, f).normalize()
    }

    #[test]
    fn srep_0071_sky_gradient() {
        // the kit's five pixels and their 8-bit values (tolerance 3; this is the formula itself)
        let sky = kit_sky();
        for ((x, y), want) in [
            ((320, 0), [163.6, 180.8, 207.0]),
            ((320, 90), [178.0, 194.6, 215.4]),
            ((320, 270), [178.9, 193.1, 207.5]),
            ((320, 359), [165.8, 178.1, 190.9]),
            ((0, 0), [167.4, 184.5, 209.2]),
        ] {
            let c = sky.radiance(ray(x as f32 + 0.5, y as f32 + 0.5));
            for k in 0..3 {
                assert!((enc(c[k]) - want[k]).abs() < 0.1, "({x}, {y}) channel {k}: {} vs {}", enc(c[k]), want[k]);
            }
        }
    }

    #[test]
    fn srep_0071_sky_horizon() {
        let sky = kit_sky();
        let c = sky.radiance(Vec3::Z);
        assert_eq!([enc(c[0]).round(), enc(c[1]).round(), enc(c[2]).round()], [192.0, 208.0, 224.0]);
        // the rows on either side of the horizon are within a code value of it
        for y in [179.5f32, 180.5] {
            let c = sky.radiance(ray(320.0, y));
            assert!((enc(c[0]) - 192.0).abs() < 1.0 && (enc(c[2]) - 224.0).abs() < 1.0, "{y}: {c}");
        }
        // straight up and down are the zenith and ground, whatever the exponent
        let mut k = sky;
        k.exponent = 3.0;
        assert!(k.radiance(Vec3::NEG_Y).distance(Vec3::from(sky.zenith)) < 1e-6);
        assert!(k.radiance(Vec3::Y).distance(Vec3::from(sky.ground)) < 1e-6);
    }

    fn sun_sky(azimuth: f64, elevation: f64) -> Sky {
        Sky::new([0.0; 3], [0.0; 3], [0.0; 3], 0.5, azimuth, elevation, 5.0, [1.0, 0.0, 0.0])
    }

    /// The red pixels of the kit's frame: their count and centroid.
    fn red_disc(sky: &Sky) -> (u32, f32, f32, [f32; 4]) {
        let (mut n, mut sx, mut sy) = (0, 0.0, 0.0);
        let mut bbox = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for y in 0..360 {
            for x in 0..640 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                if sky.radiance(ray(px, py))[0] > 0.5 {
                    n += 1;
                    sx += px;
                    sy += py;
                    bbox = [bbox[0].min(px), bbox[1].min(py), bbox[2].max(px), bbox[3].max(py)];
                }
            }
        }
        (n, sx / n as f32, sy / n as f32, bbox)
    }

    #[test]
    fn srep_0071_sky_sun() {
        // a 5° red sun straight ahead: a disc of diameter 48.4 px at the centre (tolerance 2 px)
        let (n, cx, cy, b) = red_disc(&sun_sky(0.0, 0.0));
        assert!((cx - 320.0).abs() < 0.5 && (cy - 180.0).abs() < 0.5, "{cx} {cy}");
        let (w, h) = (b[2] - b[0] + 1.0, b[3] - b[1] + 1.0);
        assert!((w - 48.4).abs() < 2.0 && (h - 48.4).abs() < 2.0, "{w} × {h}");
        let r = 24.2f32;
        assert!((n as f32 - std::f32::consts::PI * r * r).abs() < 0.03 * std::f32::consts::PI * r * r, "{n}");
    }

    #[test]
    fn srep_0071_sky_sun_azimuth() {
        // azimuth 10°, elevation 5°: right of and above the centre at (417.7, 130.8) (tolerance 2 px)
        let (_, cx, cy, _) = red_disc(&sun_sky(10.0, 5.0));
        assert!((cx - 417.7).abs() < 2.0 && (cy - 130.8).abs() < 2.0, "{cx} {cy}");
    }

    #[test]
    fn the_sun_direction_follows_the_conventions() {
        let d = |a, e| Vec3::from(sun_direction(a, e));
        assert!(d(0.0, 0.0).distance(Vec3::Z) < 1e-6);
        assert!(d(90.0, 0.0).distance(Vec3::X) < 1e-6);
        assert!(d(0.0, 90.0).distance(Vec3::NEG_Y) < 1e-6);
        // no sun at intensity 0 or size 0
        assert!(!Sky::new([0.0; 3], [0.0; 3], [0.0; 3], 1.0, 0.0, 0.0, 5.0, [0.0; 3]).has_sun());
        assert!(!Sky::new([0.0; 3], [0.0; 3], [0.0; 3], 1.0, 0.0, 0.0, 0.0, [1.0; 3]).has_sun());
    }

    #[test]
    fn the_baked_sky_keeps_the_gradient_and_the_sun_power() {
        let mut sky = kit_sky();
        sky.sun = [50.0, 40.0, 30.0];
        sky.sun_dir = sun_direction(30.0, 20.0);
        sky.sun_cos = (0.53f32.to_radians() * 0.5).cos();
        for width in [64, 256, 1024] {
            let eq = sky.equirect(width);
            let (w, h) = (eq.width, eq.height);
            let mut power = Vec3::ZERO;
            let mut gradient = Vec3::ZERO;
            for y in 0..h {
                let v = (y as f32 + 0.5) / h as f32;
                let sa = (2.0 * std::f32::consts::PI / w as f32)
                    * (std::f32::consts::PI / h as f32)
                    * (v * std::f32::consts::PI).sin();
                for x in 0..w {
                    let d = dir_of((x as f32 + 0.5) / w as f32, v);
                    power += Vec3::from(eq.rgb[(y * w + x) as usize]) * sa;
                    gradient += sky.gradient(d) * sa;
                }
            }
            let sun = (power - gradient) / sky.sun_solid_angle();
            assert!(sun.distance(Vec3::from(sky.sun)) < 1e-2 * 50.0, "{width}: {sun}");
        }
    }
}
