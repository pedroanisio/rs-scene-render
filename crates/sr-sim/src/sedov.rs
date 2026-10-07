//! The Sedov-Taylor blast: the self-similar solution of a point release of energy `E` in a gas of density `rho0`, in which the front of the
//! blast is at `R(t) = xi0 (E t^2 / rho0)^(1/5)`.
//!
//! The constant `xi0` is not given: it is found. With `xi = r / R(t)` the fields are `u = Rdot U(xi)`, `rho = rho0 G(xi)` and
//! `p = rho0 Rdot^2 P(xi)`, and the equations of an ideal gas (mass, momentum and entropy along the flow) reduce to three ordinary
//! differential equations in `xi`, integrated here from the shock (`xi = 1`, where the strong-shock conditions of Rankine and Hugoniot give
//! `U = P = 2 / (gamma + 1)` and `G = (gamma + 1) / (gamma - 1)`) toward the centre by the fourth order Runge-Kutta method in `ln xi`. The
//! energy inside the front is `E = (16 pi / 25) J rho0 R^5 / t^2` with `J = integral of (G U^2 / 2 + P / (gamma - 1)) xi^2 dxi`, which fixes
//! `xi0 = (25 / (16 pi J))^(1/5)`. That the mass inside the front is that of the sphere of ambient gas (`integral of G xi^2 dxi = 1/3`) is
//! not used in the solution, and is what the tests check it against.
//!
//! What is valid: the strong shock, while the pressure behind the front is much more than the ambient `p0`, that is, for `R` below about
//! [`Sedov::max_radius`] (an order of magnitude and not a boundary). The gas is ideal with a constant ratio of specific heats.

use std::f64::consts::PI;

/// The solution for one gas: the constants of the self-similar blast.
#[derive(Clone, Debug)]
pub struct Sedov {
    gamma: f64,
    xi0: f64,
    swept_mass: f64,
    kinetic: f64,
    thermal: f64,
    centre_pressure: f64,
}

/// The default number of steps of the integration in `ln xi` (from the front to `xi = 1e-5`).
pub const STEPS: usize = 40_000;

/// The least `xi` that the integration reaches: below it the mass is `xi^(3 + 3 / (gamma - 1))` and the energy `xi^3`, both beyond the precision.
const XI_MIN: f64 = 1e-5;

/// The solution for the ratio of specific heats `gamma`, from `1.1` to `3` (below it the density goes to zero toward the centre as a power
/// of `xi` so high (`3 / (gamma - 1)`) that the integration is lost: the step and the order are not enough, and no result is better than a wrong one).
pub fn solve(gamma: f64) -> Result<Sedov, String> {
    Sedov::with_steps(gamma, STEPS)
}

/// The solution for `gamma`, computed once for each ratio in the process (a blast asks for it for every step of a simulation).
pub fn solve_cached(gamma: f64) -> Result<std::sync::Arc<Sedov>, String> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static SOLVED: OnceLock<Mutex<HashMap<u64, Arc<Sedov>>>> = OnceLock::new();
    let key = gamma.to_bits();
    if let Some(found) = SOLVED.get_or_init(Default::default).lock().unwrap().get(&key) {
        return Ok(found.clone());
    }
    let solved = Arc::new(solve(gamma)?);
    SOLVED.get_or_init(Default::default).lock().unwrap().insert(key, solved.clone());
    Ok(solved)
}

/// The right-hand side of the three equations, as derivatives with respect to `xi`: `(U', (ln G)', (ln P)')` (the logarithms keep `G`, which
/// goes to zero as a high power of `xi` toward the centre, and `P`, positive).
fn slopes(gamma: f64, xi: f64, [u, lng, lnp]: [f64; 3]) -> [f64; 3] {
    // mass (U - xi) G' + G (U' + 2 U / xi) = 0; momentum (U - xi) U' - 3 U / 2 + P' / G = 0; entropy (U - xi) (P' / P - gamma G' / G) = 3.
    // Eliminating G' and P' leaves U' = [3 U (U - xi) / 2 - 3 P / G + 2 gamma (P / G) U / xi] / [(U - xi)^2 - gamma P / G].
    let c2 = (lnp - lng).exp();
    let d = u - xi;
    let du = (1.5 * u * d - 3.0 * c2 + 2.0 * gamma * c2 * u / xi) / (d * d - gamma * c2);
    let dlng = -(du + 2.0 * u / xi) / d;
    let dlnp = gamma * dlng + 3.0 / d;
    [du, dlng, dlnp]
}

impl Sedov {
    /// The solution by `steps` steps of the integration in `ln xi`.
    pub fn with_steps(gamma: f64, steps: usize) -> Result<Sedov, String> {
        if !(gamma.is_finite() && (1.1..=3.0).contains(&gamma)) {
            return Err(format!("the ratio of specific heats {gamma} is not between 1.1 and 3"));
        }
        if steps == 0 {
            return Err("an integration needs at least one step".into());
        }
        let x_end = XI_MIN.ln();
        let h = x_end / steps as f64; // negative: from the front inward
                                      // the state (U, G, P) and the integrals (mass, kinetic, thermal) with respect to x = ln xi: d/dx = xi d/dxi, and xi^2 dxi = xi^3 dx
        let mut y = [2.0 / (gamma + 1.0), ((gamma + 1.0) / (gamma - 1.0)).ln(), (2.0 / (gamma + 1.0)).ln()];
        let mut x = 0.0f64;
        let integrands = |xi: f64, y: &[f64; 3]| {
            let w = xi * xi * xi;
            {
                let (g, p) = (y[1].exp(), y[2].exp());
                [g * w, 0.5 * g * y[0] * y[0] * w, p * w / (gamma - 1.0)]
            }
        };
        let deriv = |x: f64, y: &[f64; 3]| -> [f64; 3] {
            let xi = x.exp();
            let s = slopes(gamma, xi, *y);
            [s[0] * xi, s[1] * xi, s[2] * xi]
        };
        let mut integral = [0.0f64; 3];
        for _ in 0..steps {
            let k1 = deriv(x, &y);
            let at = |k: &[f64; 3], f: f64| [y[0] + f * k[0], y[1] + f * k[1], y[2] + f * k[2]];
            let k2 = deriv(x + h / 2.0, &at(&k1, h / 2.0));
            let k3 = deriv(x + h / 2.0, &at(&k2, h / 2.0));
            let k4 = deriv(x + h, &at(&k3, h));
            let next: [f64; 3] = std::array::from_fn(|i| y[i] + h / 6.0 * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]));
            // Simpson on the step for the integrals, with the state at the middle from the Hermite interpolation of the step (the slopes k1 and k4 at its ends)
            let mid: [f64; 3] = std::array::from_fn(|i| (y[i] + next[i]) / 2.0 + h / 8.0 * (k1[i] - k4[i]));
            let (f0, fm, f1) =
                (integrands(x.exp(), &y), integrands((x + h / 2.0).exp(), &mid), integrands((x + h).exp(), &next));
            for i in 0..3 {
                // h is negative, so this adds the integral from the front inward with the sign of the inward direction: minus h is the width
                integral[i] += -h / 6.0 * (f0[i] + 4.0 * fm[i] + f1[i]);
            }
            y = next;
            x += h;
            if !y.iter().all(|v| v.is_finite()) {
                return Err(format!("the integration left the solution at xi = {}", x.exp()));
            }
        }
        // the tails below XI_MIN: the pressure there is its central value, the mass and the kinetic energy are far smaller
        let tail = XI_MIN.powi(3) / 3.0;
        integral[2] += y[2].exp() * tail / (gamma - 1.0);
        let (kinetic, thermal) = (integral[1], integral[2]);
        let j = kinetic + thermal;
        let xi0 = (25.0 / (16.0 * PI * j)).powf(0.2);
        if !xi0.is_finite() {
            return Err("the integral of the energy is not a number".into());
        }
        Ok(Sedov {
            gamma,
            xi0,
            swept_mass: integral[0],
            kinetic,
            thermal,
            centre_pressure: y[2].exp() / (2.0 / (gamma + 1.0)),
        })
    }

    /// The ratio of specific heats.
    pub fn gamma(&self) -> f64 {
        self.gamma
    }

    /// The constant of the front, `R = xi0 (E t^2 / rho0)^(1/5)`.
    pub fn xi0(&self) -> f64 {
        self.xi0
    }

    /// The mass inside the front in units of `rho0 R^3` (`integral of G xi^2 dxi`): `1/3` if the solution holds the mass it swept.
    pub fn swept_mass(&self) -> f64 {
        self.swept_mass
    }

    /// The share of the energy that is the motion of the gas (the rest is its heat).
    pub fn kinetic_fraction(&self) -> f64 {
        self.kinetic / (self.kinetic + self.thermal)
    }

    /// The pressure at the centre in units of the pressure behind the shock.
    pub fn central_pressure_ratio(&self) -> f64 {
        self.centre_pressure
    }

    /// The radius of the front, in metres for joules, kilograms per cubic metre and seconds.
    pub fn radius(&self, energy: f64, rho0: f64, t: f64) -> f64 {
        self.xi0 * (energy * t * t / rho0).powf(0.2)
    }

    /// The time at which the front is at the radius `r`.
    pub fn time_at(&self, energy: f64, rho0: f64, r: f64) -> f64 {
        (rho0 / energy).sqrt() * (r / self.xi0).powf(2.5)
    }

    /// The speed of the front, `2 R / (5 t)`.
    pub fn front_speed(&self, energy: f64, rho0: f64, t: f64) -> f64 {
        0.4 * self.radius(energy, rho0, t) / t
    }

    /// The pressure behind a strong shock that moves at `speed` in gas of density `rho0` (Rankine and Hugoniot): `2 rho0 D^2 / (gamma + 1)`.
    pub fn front_pressure(&self, rho0: f64, speed: f64) -> f64 {
        2.0 * rho0 * speed * speed / (self.gamma + 1.0)
    }

    /// The energy that the profiles hold inside a front of radius `radius(energy, rho0, t)`: `E` if the constant is right.
    pub fn energy(&self, energy: f64, rho0: f64, t: f64) -> f64 {
        let r = self.radius(energy, rho0, t);
        let d = self.front_speed(energy, rho0, t);
        4.0 * PI * rho0 * d * d * r * r * r * (self.kinetic + self.thermal)
    }

    /// The radius at which the blast stops being strong, `0.3 (E / p0)^(1/3)`: where the pressure of the ambient gas is of the order of the
    /// pressure of the blast. An order of magnitude and not a boundary.
    pub fn max_radius(&self, energy: f64, p0: f64) -> f64 {
        0.3 * (energy / p0).cbrt()
    }
}
