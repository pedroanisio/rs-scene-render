//! Light around a black hole that does not rotate: the null geodesics of the Schwarzschild metric in the
//! plane of the orbit, and the formulas that a render of one is checked against.
//!
//! Units are geometric, `G = c = 1`, and lengths are the scene's: the mass `M` is a length, the horizon is at
//! `r = 2M`, the photon sphere at `3M` and the last stable circular orbit at `6M`. A ray stays in one plane
//! through the hole; in it, with `u = 1/r` and `phi` the angle swept, the path of light obeys Binet's equation
//!
//! ```text
//! d²u/dφ² + u = 3 M u²        with the first integral        (du/dφ)² = 1/b² − u² + 2 M u³,
//! ```
//!
//! where `b` is the ray's impact parameter (its angular momentum over its energy, the distance from the hole
//! of the straight line it would follow far away). Nothing here allocates per step, calls a thread or keeps
//! state: the same arguments give the same bits, and the integrator is written so that a shader can do the
//! same arithmetic in the same order (the order of the operations of every function is the one it is written
//! in, left to right).
//!
//! [`trace`] is the integrator the shader mirrors: fixed step, classical Runge-Kutta in `phi`.
//! [`trace_with`] takes the step and the budget, for the tests that refine it; [`f32`] is the same arithmetic
//! in single precision; [`oracle`] holds the closed forms and the quadratures that the integration is
//! compared with, and none of it is meant to be run per pixel.

/// The step in `phi` of [`trace`], in radians.
pub const STEP: f64 = 0.02;

/// The most steps [`trace`] takes: a ray that has neither been captured nor escaped by then is taken to be
/// captured (it is circling the photon sphere, which light does for ever).
pub const MAX_STEPS: usize = 4096;

/// How a ray ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It fell through the horizon, or it was still circling when the budget ran out.
    Captured,
    /// It went out to infinity.
    Escaped,
}

/// What a ray did: how it ended, the angle it swept, and where it crossed the plane of the disc.
#[derive(Clone, Debug, PartialEq)]
pub struct Trace<T = f64> {
    pub outcome: Outcome,
    /// The angle swept from the observer to infinity, radians; 0 for a ray that was captured.
    pub phi_inf: T,
    /// `(phi, r)` of the crossings of the plane of the disc, in the order of `phi`.
    pub crossings: Vec<(T, T)>,
}

/// The step and the budget of an integration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    /// Radians of `phi` per step.
    pub step: f64,
    /// Steps after which a ray is taken to be captured.
    pub max_steps: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config { step: STEP, max_steps: MAX_STEPS }
    }
}

/// The impact parameter of the ray that reaches a static observer at `r_obs` at the angle `alpha` (radians) to
/// the direction of the hole: `r_obs sin(alpha) / sqrt(1 - 2M / r_obs)`.
pub fn impact_parameter(mass: f64, r_obs: f64, alpha: f64) -> f64 {
    r_obs * alpha.sin() / (1.0 - 2.0 * mass / r_obs).sqrt()
}

/// The redshift `g = nu_observed / nu_emitted` of light from an emitter on a circular orbit of radius `r`, in
/// the plane of the disc, for the ray of impact parameter `b`: `sqrt(1 - 3M/r) / (1 + Omega b h)`,
/// `Omega = sqrt(M / r^3)`. `h` is the component of the disc's axis on the normal to the plane of the ray,
/// signed by the direction of the emitter's motion: `h = (e1 x e2) . axis`, with `e1` toward the observer and
/// `e2` along the angle `phi` as it grows on the path traced from the camera outward. `h > 0` is a receding
/// emitter (`g < sqrt(1 - 3M/r)`), `h < 0` an approaching one, and `h = 0` the gravitational shift alone. The
/// orbit exists for `r > 3M` only; the stable ones are those beyond `6M`.
pub fn redshift(mass: f64, r: f64, b: f64, h: f64) -> f64 {
    let omega = (mass / (r * r * r)).sqrt();
    (1.0 - 3.0 * mass / r).sqrt() / (1.0 + omega * b * h)
}

/// The integrator, written once for both precisions: the arithmetic of the two is the same, operation by
/// operation, in the order it is written.
macro_rules! integrator {
    ($t:ty) => {
        /// One step of `h` radians of the classical Runge-Kutta method on `(u, w)`, `w = du/dphi`, for
        /// `u'' = -u + 3 M u^2`. The acceleration is `-u + 3 * mass * u * u`, taken left to right; the
        /// stages are `k1..k4` in that order, the half step is `0.5 * h` taken once, and the result is
        /// `x + (h / 6) * (k1 + 2 * k2 + 2 * k3 + k4)` for each of `u` and `w`.
        pub fn rk4_step(mass: $t, u: $t, w: $t, h: $t) -> ($t, $t) {
            let acceleration = |u: $t| -u + 3.0 * mass * u * u;
            let half = 0.5 * h;
            let (k1u, k1w) = (w, acceleration(u));
            let (u2, w2) = (u + half * k1u, w + half * k1w);
            let (k2u, k2w) = (w2, acceleration(u2));
            let (u3, w3) = (u + half * k2u, w + half * k2w);
            let (k3u, k3w) = (w3, acceleration(u3));
            let (u4, w4) = (u + h * k3u, w + h * k3w);
            let (k4u, k4w) = (w4, acceleration(u4));
            let sixth = h / 6.0;
            (u + sixth * (k1u + 2.0 * k2u + 2.0 * k3u + k4u), w + sixth * (k1w + 2.0 * k2w + 2.0 * k3w + k4w))
        }

        /// Follows the ray of impact parameter `b` that is at `r_obs` and goes in (`ingoing`) or out, with
        /// the step [`STEP`] and the budget [`MAX_STEPS`]; see `trace_with`.
        pub fn trace(mass: $t, r_obs: $t, b: $t, ingoing: bool, phi0: $t, max_crossings: usize) -> Trace<$t> {
            trace_with(Config::default(), mass, r_obs, b, ingoing, phi0, max_crossings)
        }

        /// Follows a ray from `r_obs` (`phi = 0`) until it is captured (`u >= 1/(2M)`) or has escaped
        /// (`u <= 0`, and the angle it leaves at is found by a linear interpolation between the last two
        /// steps), and records where it crosses the plane of the disc, which holds the angles `phi0 + k pi`
        /// for `k < max_crossings`: only those at `phi > 0` and no later than the escape are recorded, and
        /// the step that would pass one is shortened to land on it. A ray whose impact parameter is not
        /// positive is radial, and ends at once. The step is rounded to the precision of the call.
        pub fn trace_with(
            config: Config,
            mass: $t,
            r_obs: $t,
            b: $t,
            ingoing: bool,
            phi0: $t,
            max_crossings: usize,
        ) -> Trace<$t> {
            let mut crossings = Vec::with_capacity(max_crossings.min(8));
            if b.is_nan() || b <= 0.0 {
                let outcome = if ingoing { Outcome::Captured } else { Outcome::Escaped };
                return Trace { outcome, phi_inf: 0.0, crossings };
            }
            let pi = std::f64::consts::PI as $t;
            let horizon_u = 1.0 / (2.0 * mass);
            let step = config.step as $t;
            let u0 = 1.0 / r_obs;
            let first = 1.0 / (b * b) - u0 * u0 + 2.0 * mass * u0 * u0 * u0;
            let w0 = if first > 0.0 { first.sqrt() } else { 0.0 };
            let (mut u, mut w) = (u0, if ingoing { w0 } else { -w0 });
            let target = |k: usize| phi0 + k as $t * pi;
            let mut next = 0;
            while next < max_crossings && target(next) <= 0.0 {
                next += 1;
            }
            let mut phi: $t = 0.0;
            for _ in 0..config.max_steps {
                let (mut h, mut landing) = (step, false);
                let ahead = target(next);
                if next < max_crossings {
                    if phi + h > ahead {
                        h = ahead - phi;
                        landing = true;
                    } else if phi + h == ahead {
                        landing = true;
                    }
                }
                let (un, wn) = rk4_step(mass, u, w, h);
                if un >= horizon_u {
                    return Trace { outcome: Outcome::Captured, phi_inf: 0.0, crossings };
                }
                if un <= 0.0 {
                    let phi_inf = phi + h * u / (u - un);
                    return Trace { outcome: Outcome::Escaped, phi_inf, crossings };
                }
                phi = if landing { ahead } else { phi + h };
                (u, w) = (un, wn);
                if landing {
                    crossings.push((phi, 1.0 / u));
                    next += 1;
                }
            }
            Trace { outcome: Outcome::Captured, phi_inf: 0.0, crossings }
        }
    };
}

integrator!(f64);

/// The same arithmetic as the top level, in single precision, for comparing a shader with it.
pub mod f32 {
    use super::{Config, Outcome, Trace};

    integrator!(f32);
}

/// The closed forms and the quadratures that the integration is checked against.
pub mod oracle {
    /// The radius of the horizon, `2M`.
    pub fn horizon(mass: f64) -> f64 {
        2.0 * mass
    }

    /// The radius of the photon sphere, `3M`.
    pub fn photon_sphere(mass: f64) -> f64 {
        3.0 * mass
    }

    /// The radius of the shadow as the impact parameter that separates capture from escape, `sqrt(27) M`.
    pub fn shadow_radius(mass: f64) -> f64 {
        27f64.sqrt() * mass
    }

    /// The radius of the innermost stable circular orbit, `6M`.
    pub fn isco(mass: f64) -> f64 {
        6.0 * mass
    }

    /// The angular velocity of a circular orbit of radius `r` for the distant observer, `sqrt(M / r^3)`.
    pub fn kepler_omega(mass: f64, r: f64) -> f64 {
        (mass / (r * r * r)).sqrt()
    }

    /// The smallest positive `u` at which `1/b^2 - u^2 + 2 M u^3` vanishes, the turning point `1/r_min` of a ray
    /// of impact parameter `b`; none when `b` does not exceed the shadow's radius, because the ray is captured.
    pub fn turning_point(mass: f64, b: f64) -> Option<f64> {
        // P(u) = 1/b^2 - u^2 + 2 M u^3 is positive at 0, falls to a minimum at u = 1/(3M), where it is
        // 1/b^2 - 1/(27 M^2): negative exactly when b exceeds the shadow's radius, and then it has one root
        // between the two, found by bisection
        let p = |u: f64| 1.0 / (b * b) - u * u + 2.0 * mass * u * u * u;
        let (mut low, mut high) = (0.0, 1.0 / (3.0 * mass));
        if b.is_nan() || b <= 0.0 || p(high) >= 0.0 {
            return None;
        }
        for _ in 0..200 {
            let mid = 0.5 * (low + high);
            if p(mid) > 0.0 {
                low = mid;
            } else {
                high = mid;
            }
        }
        Some(0.5 * (low + high))
    }

    /// The angle by which a ray from infinity to infinity is deflected, in radians, by quadrature of the first
    /// integral to 1e-12; none for a ray that is captured.
    pub fn deflection(mass: f64, b: f64) -> Option<f64> {
        let u_m = turning_point(mass, b)?;
        // The ray sweeps 2 * integral of du / sqrt(P(u)) from 0 to u_m. P = (u_m - u) Q(u) with
        // Q(u) = (1 - 2 M u_m)(u + u_m) - 2 M u^2, so with u = u_m (1 - s^2) the integrand is
        // 2 sqrt(u_m) / sqrt(Q) on s in [0, 1]: smooth, with no cancellation near the turning point.
        let k = 1.0 - 2.0 * mass * u_m;
        let integrand = |s: f64| {
            let u = u_m * (1.0 - s * s);
            let q = k * (u + u_m) - 2.0 * mass * u * u;
            2.0 * u_m.sqrt() / q.sqrt()
        };
        Some(2.0 * romberg(integrand, 0.0, 1.0) - std::f64::consts::PI)
    }

    /// Romberg's extrapolation of the trapezoid rule on `[a, b]`, to 1e-14 of the result or 2^20 intervals.
    fn romberg(f: impl Fn(f64) -> f64, a: f64, b: f64) -> f64 {
        let mut previous = vec![0.5 * (b - a) * (f(a) + f(b))];
        for level in 1..=20 {
            let n = 1usize << (level - 1);
            let h = (b - a) / n as f64;
            let mid: f64 = (0..n).map(|i| f(a + (i as f64 + 0.5) * h)).sum();
            let mut row = Vec::with_capacity(level + 1);
            row.push(0.5 * previous[0] + 0.5 * h * mid);
            for j in 1..=level {
                let factor = 4f64.powi(j as i32);
                row.push((factor * row[j - 1] - previous[j - 1]) / (factor - 1.0));
            }
            let (new, old) = (row[level], previous[level - 1]);
            previous = row;
            if level >= 4 && (new - old).abs() <= 1e-14 * new.abs() {
                break;
            }
        }
        *previous.last().expect("a row")
    }

    /// The deflection in the weak field, `4M/b`.
    pub fn weak_field_deflection(mass: f64, b: f64) -> f64 {
        4.0 * mass / b
    }

    /// The weak field to the second order, `4M/b + 15 pi/4 (M/b)^2`.
    pub fn weak_field_deflection_second_order(mass: f64, b: f64) -> f64 {
        4.0 * mass / b + 15.0 * std::f64::consts::PI / 4.0 * (mass / b) * (mass / b)
    }

    /// The weak-field series to the fourth order in `M/b` (Keeton and Petters, 2005, from memory, and checked
    /// against the quadrature): `4x + 15 pi/4 x^2 + 128/3 x^3 + 3465 pi/64 x^4`, `x = M/b`.
    pub fn weak_field_deflection_series(mass: f64, b: f64) -> f64 {
        let x = mass / b;
        let pi = std::f64::consts::PI;
        4.0 * x + 15.0 * pi / 4.0 * x * x + 128.0 / 3.0 * x * x * x + 3465.0 * pi / 64.0 * x * x * x * x
    }

    /// The redshift `g` of Luminet (1979) of an emitter in a circular orbit of radius `r` seen from the
    /// inclination `inclination` (radians, the angle between the axis of the disc and the line of sight) at the
    /// angle `alpha` on the observer's sky (radians, positive where the disc recedes), for the ray of impact
    /// parameter `b`: `1 + z = (1 + Omega b sin(inclination) sin(alpha)) / sqrt(1 - 3M/r)`, `g = 1 / (1 + z)`.
    pub fn luminet_redshift(mass: f64, r: f64, b: f64, inclination: f64, alpha: f64) -> f64 {
        let one_plus_z =
            (1.0 + kepler_omega(mass, r) * b * inclination.sin() * alpha.sin()) / (1.0 - 3.0 * mass / r).sqrt();
        1.0 / one_plus_z
    }

    /// The temperature of a thin disc of Shakura and Sunyaev around a hole with its inner edge at `r_in`, in
    /// units of `scale`: `scale r^(-3/4) (1 - sqrt(r_in / r))^(1/4)`, and zero at and inside `r_in`.
    pub fn disc_temperature(r: f64, r_in: f64, scale: f64) -> f64 {
        if r <= r_in {
            return 0.0;
        }
        scale * r.powf(-0.75) * (1.0 - (r_in / r).sqrt()).powf(0.25)
    }
}

/// The image of a black hole with a disc, traced ray by ray on the CPU in double precision: the reference the
/// shader of `sr-gpu` is compared with, pixel by pixel.
///
/// # The convention, shared with the shader
///
/// Both do the same sums, so that a difference of convention shows as a difference of image and not as
/// physics:
///
/// - Scene axes are right-handed; the hole is at `hole`, the observer at `eye`, `M` is `mass`, and lengths are
///   the scene's. The camera has the unit vectors `fwd`, `right` and `down` (`right x down = fwd`, so `down`
///   points to the bottom of the image), a focal length `focal` in pixels, and `size = [width, height]`. The
///   ray of pixel `(x, y)` goes through the centre of the pixel: `d = normalize(fwd * focal + right * (x + 0.5
///   - width / 2) + down * (y + 0.5 - height / 2))`.
/// - The ray lies in the plane of the observer, the hole and itself. With `n = (eye - hole) / r_o` (the unit
///   vector from the hole to the observer, `r_o` the distance), `cos a = -d . n`, `perp = d + n cos a` and `sin
///   a = |perp|`, the first basis vector of the plane is `e1 = n` and the second `e2 = perp / sin a` (any unit
///   vector orthogonal to `n` when `sin a` is below 1e-6: the cross product of `n` with the x axis, or with the
///   y axis if `|n.x| > 0.9`, normalized). The angle `phi` of the orbit equation grows from `e1` toward `e2`
///   along the path traced from the camera outward, so the point of the ray at `phi` and radius `r` is
///   `hole + r (cos(phi) e1 + sin(phi) e2)`.
/// - The impact parameter is `b = max(r_o sin a / sqrt(1 - 2M / r_o), 1e-5)` and the ray goes in
///   (`ingoing`) when `cos a > 0`.
/// - The disc is thin, opaque and flat, with the unit axes `dx`, `dy` in its plane (`dx x dy = dz`) and `dz`
///   its axis of spin; it spans `r_in <= r <= r_out`. The rays cross its plane where `phi = phi0 + k pi`,
///   with `phi0` in `(0, pi]` the first angle at which the plane of the ray meets it: with `a = e1 . dz` and
///   `c = e2 . dz`, `phi0 = atan2(-a, c)`, plus `pi` if it is negative, plus `pi` again if it is below
///   1e-6; none exists when `a^2 + c^2 < 1e-12` (the planes coincide). Only the first four crossings (`k < 4`)
///   are followed, and the pixel takes the first of them that is inside the disc.
/// - The azimuth of the crossing in the disc is `psi = atan2(p . dy, p . dx)` with `p = cos(phi_k) e1 + sin(phi_k)
///   e2` for `phi_k = phi0 + k pi`. The redshift is [`redshift`](super::redshift) with `h = (e1 x e2) . dz`.
/// - A ray that escapes ends in the direction `cos(phi_inf) e1 + sin(phi_inf) e2`.
///
/// The tracer is serial and does no sum of more than one pixel, so the same arguments give the same bits.
pub mod image {
    use super::Outcome;

    /// What a pixel shows.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Class {
        /// The ray fell through the horizon: black.
        Captured,
        /// The ray escaped and met no disc: the sky in `direction`.
        Background,
        /// The ray crossed the disc: `r`, `psi`, `g` and `order` tell where and how.
        Disk,
    }

    /// The camera: where it is, how it is turned, and its focal length in pixels.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Camera {
        pub eye: [f64; 3],
        pub hole: [f64; 3],
        pub fwd: [f64; 3],
        pub right: [f64; 3],
        pub down: [f64; 3],
        pub focal: f64,
        pub size: [usize; 2],
    }

    /// The disc, flat and thin around the hole.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Disk {
        pub r_in: f64,
        pub r_out: f64,
        pub dx: [f64; 3],
        pub dy: [f64; 3],
        pub dz: [f64; 3],
    }

    /// What one ray did.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Pixel {
        pub class: Class,
        /// For a background pixel, the direction at infinity; zero otherwise.
        pub direction: [f64; 3],
        /// For a ray that escaped, the angle swept; zero otherwise.
        pub phi_inf: f64,
        /// For a disc pixel, the radius, azimuth and redshift of the point seen, and which crossing it was
        /// (0 is the first plane crossed); zero and none otherwise.
        pub r: f64,
        pub psi: f64,
        pub g: f64,
        pub order: Option<usize>,
    }

    impl Camera {
        /// The camera of an observer at `distance` from a hole at the origin, `inclination` radians from the
        /// spin axis of [`Disk::flat`] (the y axis; 0 looks down it, `pi / 2` is the plane of the disc) in the
        /// plane of the x and y axes, looking at the hole, with the y axis up in the image and a vertical field
        /// of view `fov_y`.
        pub fn orbiting(distance: f64, inclination: f64, fov_y: f64, size: [usize; 2]) -> Camera {
            let _ = (distance, inclination, fov_y, size);
            todo!("the camera of an observer around the hole")
        }

        /// The unit direction of the ray of pixel `(x, y)`.
        pub fn ray(&self, x: usize, y: usize) -> [f64; 3] {
            let _ = (x, y);
            todo!("the direction of a pixel")
        }
    }

    impl Disk {
        /// The disc in the plane of the x and z axes, `dx = x`, `dy = -z`, spinning about `y` (`dx x dy = y`).
        pub fn flat(r_in: f64, r_out: f64) -> Disk {
            let _ = (r_in, r_out);
            todo!("the flat disc")
        }
    }

    /// What the ray of direction `d` from the camera's eye does around a hole of mass `mass`, with or without a
    /// disc.
    pub fn trace_direction(camera: &Camera, mass: f64, disk: Option<&Disk>, d: [f64; 3]) -> Pixel {
        let _ = (camera, mass, disk, d, Outcome::Captured);
        todo!("the ray of a direction")
    }

    /// The ray of pixel `(x, y)`.
    pub fn trace_pixel(camera: &Camera, mass: f64, disk: Option<&Disk>, x: usize, y: usize) -> Pixel {
        trace_direction(camera, mass, disk, camera.ray(x, y))
    }

    /// Every pixel, by rows from the top.
    pub fn render(camera: &Camera, mass: f64, disk: Option<&Disk>) -> Vec<Pixel> {
        let [w, h] = camera.size;
        (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| trace_pixel(camera, mass, disk, x, y)).collect()
    }
}
