//! The Sedov-Taylor blast: the self-similar solution of a point release of energy in a gas, found by integrating its equations, and what
//! the blast of the smoke is made of it. The oracles are the conserved mass and energy of the solution itself and the published values of the
//! constant (Sedov 1959; Landau and Lifshitz, Fluid Mechanics, section 106), none of which the module is given.

use sr_sim::sedov::{solve, Sedov};

/// The published constant xi0 of R = xi0 (E t^2 / rho0)^(1/5): 1.033 for gamma 1.4 (Landau and Lifshitz give alpha = 0.851, xi0 = alpha^(-1/5)),
/// and about 1.15 for gamma 5/3 (Taylor's value for a monatomic gas).
#[test]
fn the_constant_of_the_front_is_the_published_one_to_two_percent_for_two_gases() {
    let air = solve(1.4).unwrap();
    assert!((air.xi0() / 1.033 - 1.0).abs() < 0.02, "xi0(1.4) = {}", air.xi0());
    let monatomic = solve(5.0 / 3.0).unwrap();
    assert!((monatomic.xi0() / 1.15 - 1.0).abs() < 0.02, "xi0(5/3) = {}", monatomic.xi0());
    // and the pressure at the centre of the monatomic blast is about 0.306 of the pressure behind the shock (Taylor and Sedov, as tabulated)
    assert!((0.30..0.312).contains(&monatomic.central_pressure_ratio()), "{}", monatomic.central_pressure_ratio());
    // to the digits that Landau and Lifshitz print for air: alpha = 0.851 is xi0 = 1.0328
    assert!((air.xi0() - 1.0328).abs() < 5e-4, "{}", air.xi0());
    // and alpha = xi0^-5 = 0.851 for air
    assert!((air.xi0().powi(-5) / 0.851 - 1.0).abs() < 0.02);
}

#[test]
fn the_solution_holds_all_the_mass_it_swept_and_all_the_energy_it_was_given() {
    for gamma in [1.2, 1.4, 5.0 / 3.0] {
        let s = solve(gamma).unwrap();
        // the mass inside the front is the mass of the sphere of the ambient gas, rho0 4 pi R^3 / 3: the integral of G xi^2 from 0 to 1 is 1/3
        assert!((s.swept_mass() - 1.0 / 3.0).abs() < 1e-6, "gamma {gamma}: {}", s.swept_mass());
        // the energy: R from the constant, then the energy integral over the profiles is E (in units E = rho0 = t = 1)
        let r = s.radius(1.0, 1.0, 1.0);
        let e = s.energy(1.0, 1.0, 1.0);
        assert!((e - 1.0).abs() < 1e-9, "gamma {gamma}: the energy of the profiles at R = {r} is {e}");
        // the kinetic fraction is a fraction (in the literature about 0.3 for air; this is the solver's own number)
        let k = s.kinetic_fraction();
        assert!((0.1..0.6).contains(&k), "gamma {gamma}: {k}");
    }
}

#[test]
fn the_constant_converges_with_the_step_of_the_integration() {
    let coarse = Sedov::with_steps(1.4, 2_000).unwrap();
    let fine = Sedov::with_steps(1.4, 20_000).unwrap();
    let finer = Sedov::with_steps(1.4, 200_000).unwrap();
    let (a, b) = ((coarse.xi0() - finer.xi0()).abs(), (fine.xi0() - finer.xi0()).abs());
    assert!(b < a && b < 1e-8, "{a} then {b}");
}

#[test]
fn the_law_of_the_front_is_the_power_one_fifth_of_the_energy_and_two_fifths_of_the_time() {
    let s = solve(1.4).unwrap();
    let r = |e: f64, rho: f64, t: f64| s.radius(e, rho, t);
    // R scales as E^(1/5) t^(2/5) rho^(-1/5): by 32 in E gives 2, by 32 in t gives 4, by 32 in rho gives 1/2
    assert!((r(32.0, 1.0, 1.0) / r(1.0, 1.0, 1.0) - 2.0).abs() < 1e-12);
    assert!((r(1.0, 1.0, 32.0) / r(1.0, 1.0, 1.0) - 4.0).abs() < 1e-12);
    assert!((r(1.0, 32.0, 1.0) / r(1.0, 1.0, 1.0) - 0.5).abs() < 1e-12);
    // the speed of the front is 2 R / (5 t)
    let (e, rho, t) = (4.2e6, 1.2, 0.01);
    assert!((s.front_speed(e, rho, t) - 0.4 * s.radius(e, rho, t) / t).abs() < 1e-9);
    // the pressure behind the strong shock is 2 rho0 D^2 / (gamma + 1) (Rankine-Hugoniot)
    let d = s.front_speed(e, rho, t);
    assert!((s.front_pressure(rho, d) - 2.0 * rho * d * d / 2.4).abs() < 1e-6 * rho * d * d);
}

#[test]
fn a_blast_of_a_known_size_has_the_overpressure_and_the_radius_of_the_order_it_is() {
    // 1 kg of TNT is 4.2 MJ. At 10 m in air (rho 1.2, p0 101325 Pa) the blast has long ceased to be strong (R_max = 0.3 (E/p0)^(1/3) is
    // 1.0 m), but at 1 m the front of the strong phase has the speed that a few metres in a millisecond make: R = 1 m at t about 0.8 ms (the
    // published fireball scaling R = 0.3 to 0.6 E^(1/3) m per kg^(1/3) puts the strong phase inside about 1 m)
    let s = solve(1.4).unwrap();
    let (e, rho, p0) = (4.2e6, 1.2, 101_325.0);
    let r_max = s.max_radius(e, p0);
    assert!((r_max - 0.3 * (e / p0).powf(1.0 / 3.0)).abs() < 1e-12);
    assert!((0.9..1.2).contains(&r_max), "{r_max}");
    // the time at which the front is at 0.5 m, the speed and the pressure there: order of magnitude of a km/s and of tens of megapascals
    let t = s.time_at(e, rho, 0.5);
    assert!((s.radius(e, rho, t) - 0.5).abs() < 1e-12);
    let d = s.front_speed(e, rho, t);
    let p = s.front_pressure(rho, d);
    assert!((500.0..5000.0).contains(&d), "front speed {d} m/s at 0.5 m");
    assert!((1.0e6..1.0e8).contains(&p), "front pressure {p} Pa at 0.5 m");
    // where the pressure of the strong front falls to a few times p0 the strong phase is over: that is beyond R_max
    let t_end = s.time_at(e, rho, r_max);
    assert!(
        s.front_pressure(rho, s.front_speed(e, rho, t_end)) > 5.0 * p0,
        "the front is still strong at R_max by this law"
    );
}

#[test]
fn a_gas_that_cannot_be_is_an_error() {
    for gamma in [1.0, 0.5, f64::NAN, f64::INFINITY, 1.0 + 1e-12, 1.05, 1.0999, 9.0] {
        assert!(solve(gamma).is_err(), "{gamma}");
    }
    assert!(Sedov::with_steps(1.4, 0).is_err()); // the least gas that is solved is solved: its mass inside the front is that of the sphere that it swept (1e-4: the density goes to zero as the 30th power of xi
                                                 // there, so the integral is the least exact), and its constant is a number between the values for 1 and for 1.2
    let thin = solve(1.1).unwrap();
    assert!((thin.swept_mass() - 1.0 / 3.0).abs() < 1e-4, "{}", thin.swept_mass());
    assert!(thin.xi0() > 0.7 && thin.xi0() < solve(1.2).unwrap().xi0(), "{}", thin.xi0());
    // the solution for a gas is the same one the second time (and is computed once)
    let again = sr_sim::sedov::solve_cached(1.4).unwrap();
    assert!(std::sync::Arc::ptr_eq(&again, &sr_sim::sedov::solve_cached(1.4).unwrap()));
}
