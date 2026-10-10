//! The subsurface lobe of SREP 71 (Semantics 1): the normalized diffusion profile of Christensen and Burley,
//!
//! ```text
//! R(r) = A · (e^(−r/d) + e^(−r/(3d))) / (8π · d · r),   ∫₀^∞ R(r) · 2πr dr = A,
//! d = ℓ / s(A),   s(A) = 1.85 − A + 7·|A − 0.8|³,
//! ```
//!
//! evaluated by pre-integration over the local curvature of the surface. The irradiance of a point lit at cos θ = n·l
//! is spread by the profile over a sphere of the surface's radius of curvature ρ. On a sphere the area within chord
//! distance r of a point is π r², the area of the disc of radius r in the plane (Archimedes' hat-box theorem: a cap
//! of height h has area 2πρh and chord² = 2ρh), so the profile's planar radial distribution applies to chord distances
//! unchanged. Every point at chord distance r lies on the ring at angle α = 2·asin(r / 2ρ) from the shaded point, and
//! the mean clamped cosine over that ring has a closed form ([`ring_irradiance`]). The lobe is then the mean of the
//! ring irradiance over the profile's radial distribution, by a fixed quadrature at its quantiles ([`QUANTILES`]).
//! Mass beyond the antipode (r > 2ρ) is placed at the antipode, so the lobe keeps all the energy the profile carries
//! (∫R = A) on a small object.
//!
//! The five properties the SREP makes normative, and how this method meets them (tests below):
//! 1. weight 0 leaves the picture unchanged: the renderer skips the lobe;
//! 2. as ℓ → 0 every quantile goes to r = 0 and the lobe is max(cos θ, 0), Lambert with albedo A;
//! 3. past the terminator (cos θ < 0) the rings reach lit surface, so the lobe is positive and grows with ℓ;
//! 4. the lobe is A times a mean of irradiances, so it reflects at most A ≤ 1 of what arrives;
//! 5. a channel with a larger ℓ has larger quantile distances and spreads farther.
//!
//! A flat surface (ρ = ∞) has no curvature for light to wrap around, and the lobe is Lambertian there: this is a
//! curvature (pre-integrated) method, not a screen-space diffusion, and it does not blur shadow edges on flat ground.

use std::f64::consts::PI;

/// The shape-parameter fit of Christensen and Burley for the mean free path (their equation 5): d = ℓ / s(A).
pub fn shape(albedo: f64) -> f64 {
    1.85 - albedo + 7.0 * (albedo - 0.8).abs().powi(3)
}

/// The profile's shape parameter d for a mean free path ℓ and albedo A.
pub fn shape_parameter(mean_free_path: f64, albedo: f64) -> f64 {
    mean_free_path / shape(albedo.clamp(0.0, 1.0))
}

/// The per-channel shape parameters of a material: ℓ_c = `radius` · `scale`_c, d_c = ℓ_c / s(A_c).
pub fn shape_parameters(radius: f32, scale: [f32; 3], color: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|c| shape_parameter(radius as f64 * scale[c] as f64, color[c] as f64) as f32)
}

/// The profile R(r) of albedo `a` and shape parameter `d` at distance r > 0.
pub fn profile(r: f64, a: f64, d: f64) -> f64 {
    a * ((-r / d).exp() + (-r / (3.0 * d)).exp()) / (8.0 * PI * d * r)
}

/// The share of the profile's energy within distance x · d: F(x) = 1 − ¼e^(−x) − ¾e^(−x/3).
pub fn cdf(x: f64) -> f64 {
    1.0 - 0.25 * (-x).exp() - 0.75 * (-x / 3.0).exp()
}

/// The distance, in units of d, within which a share u ∈ [0, 1) of the energy lies (the inverse of [`cdf`]).
pub fn quantile(u: f64) -> f64 {
    let (mut lo, mut hi) = (0.0, 1.0);
    while cdf(hi) < u {
        hi *= 2.0;
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if cdf(mid) < u {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// Number of quadrature points of the lobe.
pub const SAMPLES: usize = 16;

/// The profile's quantiles at the midpoints (k + ½) / [`SAMPLES`], in units of d.
pub const QUANTILES: [f32; SAMPLES] = [
    0.06383456, 0.20014349, 0.34958202, 0.51450014, 0.69787014, 0.90351087, 1.1364199, 1.4032811, 1.7132746, 2.0794415,
    2.5211546, 3.0690293, 3.776038, 4.7477603, 6.25375, 9.535895,
];

/// The mean of max(n·l, 0) over the ring of points at angle α from a point whose normal makes cos θ = `c` with the
/// light: with a = c·cos α and b = sin θ·sin α ≥ 0 it is a when a ≥ b, 0 when a ≤ −b, and
/// (a·acos(−a/b) + √(b² − a²)) / π between.
pub fn ring_irradiance(c: f64, alpha: f64) -> f64 {
    let c = c.clamp(-1.0, 1.0);
    let a = c * alpha.cos();
    let b = (1.0 - c * c).max(0.0).sqrt() * alpha.sin().abs();
    if a >= b {
        a
    } else if a <= -b {
        0.0
    } else {
        (a * (-a / b).clamp(-1.0, 1.0).acos() + (b * b - a * a).max(0.0).sqrt()) / PI
    }
}

/// The lobe's irradiance factor (multiplied by A it is the reflected share) at cos θ = `c` on a surface of curvature
/// `curvature` (1/ρ) for shape parameter `d`: the mean of [`ring_irradiance`] at the quantiles of the profile.
pub fn lobe(c: f64, curvature: f64, d: f64) -> f64 {
    QUANTILES
        .iter()
        .map(|&x| {
            let half = (x as f64 * d * curvature * 0.5).min(1.0);
            ring_irradiance(c, 2.0 * half.asin())
        })
        .sum::<f64>()
        / SAMPLES as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_is_normalized_and_its_cdf_matches() {
        // ∫ R(r) 2πr dr = A, numerically on a log grid, and F is its integral
        for (a, d) in [(0.8, 1.0), (0.3, 7.5), (1.0, 0.2)] {
            let (mut sum, mut r) = (0.0, 1e-9);
            let f = 1.0005f64;
            while r < 200.0 * d {
                let dr = r * (f - 1.0);
                sum += profile(r + 0.5 * dr, a, d) * 2.0 * PI * (r + 0.5 * dr) * dr;
                r *= f;
            }
            assert!((sum - a).abs() < 1e-3 * a, "{a} {d}: {sum}");
        }
        assert_eq!(cdf(0.0), 0.0);
        assert!((cdf(60.0) - 1.0).abs() < 1e-8);
    }

    #[test]
    fn the_quantile_table_is_the_profile_s() {
        for (k, q) in QUANTILES.iter().enumerate() {
            let want = quantile((k as f64 + 0.5) / SAMPLES as f64);
            assert!((*q as f64 - want).abs() < 1e-6, "{k}: {q} vs {want}");
        }
    }

    #[test]
    fn the_shape_fit_is_christensen_burley_s() {
        // s(A) at a few points of the fit (equation 5 of Pixar memo 15-04, as appleseed implements it)
        assert!((shape(0.8) - 1.05).abs() < 1e-12);
        assert!((shape(0.0) - (1.85 + 7.0 * 0.512)).abs() < 1e-12);
        assert!((shape(1.0) - (0.85 + 7.0 * 0.008)).abs() < 1e-12);
        let d = shape_parameters(10.0, [1.0, 0.5, 0.25], [0.8, 0.8, 0.8]);
        assert!((d[0] - 10.0 / 1.05).abs() < 1e-5 && (d[1] * 2.0 - d[0]).abs() < 1e-5);
    }

    #[test]
    fn the_ring_irradiance_is_the_mean_clamped_cosine() {
        for c in [-0.9, -0.3, 0.0, 0.2, 0.7, 1.0] {
            for alpha in [0.0, 0.3, 1.0, 2.0, 3.0, PI] {
                let n = 20_000;
                let (st, ct) = ((1.0f64 - c * c).sqrt(), c);
                let mut sum = 0.0;
                for i in 0..n {
                    let phi = (i as f64 + 0.5) / n as f64 * 2.0 * PI;
                    // the ring point's normal: rotate n by α toward azimuth φ; its cosine with l
                    let cos = ct * alpha.cos() + st * alpha.sin() * phi.cos();
                    sum += cos.max(0.0);
                }
                let mean = sum / n as f64;
                assert!(
                    (ring_irradiance(c, alpha) - mean).abs() < 1e-4,
                    "{c} {alpha}: {} vs {mean}",
                    ring_irradiance(c, alpha)
                );
            }
        }
    }

    /// Brute force: the profile spread over a sphere of radius ρ lit from +z, ∫ E(p) R(|p − q|) dA / A at q.
    fn sphere_reference(c: f64, rho: f64, d: f64) -> f64 {
        let theta_q = c.acos();
        let q = [theta_q.sin(), 0.0, theta_q.cos()];
        let (nt, np) = (1500, 600);
        let mut sum = 0.0;
        for i in 0..nt {
            let t = (i as f64 + 0.5) / nt as f64 * PI;
            for j in 0..np {
                let p_ = (j as f64 + 0.5) / np as f64 * 2.0 * PI;
                let p = [t.sin() * p_.cos(), t.sin() * p_.sin(), t.cos()];
                let e = p[2].max(0.0);
                if e == 0.0 {
                    continue;
                }
                let r = rho * ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
                let da = rho * rho * t.sin() * (PI / nt as f64) * (2.0 * PI / np as f64);
                sum += e * profile(r.max(1e-9), 1.0, d) * da;
            }
        }
        sum
    }

    #[test]
    fn the_lobe_approximates_the_profile_on_a_sphere() {
        // where the profile fits on the sphere (d small against ρ), the quadrature is within a few per cent
        // of the brute-force integral, on both sides of the terminator
        let rho = 80.0;
        for d in [2.0, 6.0] {
            for c in [0.8, 0.3, 0.05, -0.1] {
                let reference = sphere_reference(c, rho, d);
                let ours = lobe(c, 1.0 / rho, d);
                assert!((ours - reference).abs() < 0.02 + 0.08 * reference, "d {d} cos {c}: {ours} vs {reference}");
            }
        }
    }

    #[test]
    fn property_2_zero_radius_is_lambert() {
        for c in [-0.5, 0.0, 0.3, 1.0] {
            assert!((lobe(c, 1.0 / 80.0, 0.0) - c.max(0.0)).abs() < 1e-12);
            // and a flat surface
            assert!((lobe(c, 0.0, 5.0) - c.max(0.0)).abs() < 1e-12);
        }
    }

    #[test]
    fn property_3_wrap_grows_with_the_radius() {
        // the kit's sphere: radius 80, subsurfaceRadius 20, past the terminator
        for c in [-0.05, -0.2, -0.4] {
            let mut last = 0.0;
            for l in [1.0, 5.0, 10.0, 20.0, 40.0] {
                let e = lobe(c, 1.0 / 80.0, shape_parameter(l, 0.8));
                // never below Lambert (0 here); strictly growing once the profile's quadrature reaches lit surface
                assert!(e >= last && (l < 10.0 || e > last), "cos {c}, ℓ {l}: {e} after {last}");
                last = e;
            }
        }
    }

    #[test]
    fn property_4_energy() {
        // the lobe never exceeds the brightest irradiance (1), so A · lobe ≤ A ≤ 1; the mean over the sphere
        // under one light is what Lambert gives (¼ of the sphere's cross-section per area), no more
        for d in [0.5, 10.0, 80.0, 1000.0] {
            let n = 2000;
            let mut mean = 0.0;
            for i in 0..n {
                // uniform in cos θ over the sphere
                let c = -1.0 + (i as f64 + 0.5) / n as f64 * 2.0;
                let e = lobe(c, 1.0 / 80.0, d);
                assert!((0.0..=1.0).contains(&e));
                mean += e / n as f64;
            }
            assert!(mean <= 0.25 + 1e-3, "{d}: {mean}");
        }
    }

    #[test]
    fn property_5_red_spreads_farthest() {
        // the default scale (1, 0.5, 0.25): past the terminator red > green > blue; on the lit side the reverse
        let d = shape_parameters(20.0, [1.0, 0.5, 0.25], [0.8, 0.8, 0.8]);
        let at = |c: f64| d.map(|dc| lobe(c, 1.0 / 80.0, dc as f64));
        let past = at(-0.15);
        assert!(past[0] > past[1] && past[1] > past[2], "{past:?}");
        let lit = at(0.9);
        assert!(lit[0] <= lit[1] && lit[1] <= lit[2], "{lit:?}");
    }
}
