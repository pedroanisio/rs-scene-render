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
        let _ = mass;
        todo!("the critical impact parameter")
    }

    /// The radius of the innermost stable circular orbit, `6M`.
    pub fn isco(mass: f64) -> f64 {
        let _ = mass;
        todo!("the innermost stable circular orbit")
    }

    /// The angular velocity of a circular orbit of radius `r` for the distant observer, `sqrt(M / r^3)`.
    pub fn kepler_omega(mass: f64, r: f64) -> f64 {
        let _ = (mass, r);
        todo!("the Keplerian angular velocity")
    }

    /// The smallest positive `u` at which `1/b^2 - u^2 + 2 M u^3` vanishes, the turning point `1/r_min` of a ray
    /// of impact parameter `b`; none when `b` does not exceed the shadow's radius, because the ray is captured.
    pub fn turning_point(mass: f64, b: f64) -> Option<f64> {
        let _ = (mass, b);
        todo!("the turning point of a ray")
    }

    /// The angle by which a ray from infinity to infinity is deflected, in radians, by quadrature of the first
    /// integral to 1e-12; none for a ray that is captured.
    pub fn deflection(mass: f64, b: f64) -> Option<f64> {
        let _ = (mass, b);
        todo!("the exact deflection")
    }

    /// The deflection in the weak field, `4M/b`.
    pub fn weak_field_deflection(mass: f64, b: f64) -> f64 {
        4.0 * mass / b
    }

    /// The weak field to the second order, `4M/b + 15 pi/4 (M/b)^2`.
    pub fn weak_field_deflection_second_order(mass: f64, b: f64) -> f64 {
        4.0 * mass / b + 15.0 * std::f64::consts::PI / 4.0 * (mass / b) * (mass / b)
    }

    /// The redshift `g` of Luminet (1979) of an emitter in a circular orbit of radius `r` seen from the
    /// inclination `inclination` (radians, the angle between the axis of the disc and the line of sight) at the
    /// angle `alpha` on the observer's sky (radians, positive where the disc recedes), for the ray of impact
    /// parameter `b`: `1 + z = (1 + Omega b sin(inclination) sin(alpha)) / sqrt(1 - 3M/r)`, `g = 1 / (1 + z)`.
    pub fn luminet_redshift(mass: f64, r: f64, b: f64, inclination: f64, alpha: f64) -> f64 {
        let _ = (mass, r, b, inclination, alpha);
        todo!("the redshift of Luminet")
    }

    /// The temperature of a thin disc of Shakura and Sunyaev around a hole with its inner edge at `r_in`, in
    /// units of `scale`: `scale r^(-3/4) (1 - sqrt(r_in / r))^(1/4)`, and zero at and inside `r_in`.
    pub fn disc_temperature(r: f64, r_in: f64, scale: f64) -> f64 {
        let _ = (r, r_in, scale);
        todo!("the temperature of the disc")
    }
}
