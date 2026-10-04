//! Sources whose expansion follows from their heat: an ideal gas heated at constant pressure
//! expands at (dT/dt)/T, so a hot source needs no authored expansion.

use sr_sim::pyro::{Boundary, Impulse, Inputs, Shape, Simulation, Source, Spec};

fn spec() -> Spec {
    Spec {
        cells: [8, 8, 8],
        dt: 0.1,
        boundary: Boundary::Open,
        pressure_iterations: 500,
        pressure_tolerance: 1e-8,
        ..Spec::default()
    }
}

fn whole() -> Shape {
    Shape::Box { min: [0.0; 3], max: [8.0; 3] }
}

fn heated_source(rate: f64) -> Source {
    Source { shape: whole(), temperature_rate: rate, density_rate: 1.0, ..Source::default() }
}

#[test]
fn heating_a_source_makes_the_divergence_of_an_ideal_gas_at_constant_pressure() {
    let (rate, dt) = (2000.0, 0.1);
    let ambient = spec().ambient_temperature;
    let mut sim = Simulation::new(spec()).unwrap();
    let report = sim.step(&Inputs { heated: vec![heated_source(rate)], ..Inputs::default() }).unwrap();
    assert!(report.divergence_after < 1e-7, "{report:?}");
    // dT = 200 K over the step in a gas at 300 + 200 K: dT / (T dt)
    let want = rate * dt / ((ambient + rate * dt) * dt);
    for z in 0..8 {
        for y in 0..8 {
            for x in 0..8 {
                let got = sim.state().divergence_at([x, y, z]);
                assert!((got - want).abs() < 1e-7 * want, "{got} vs {want}");
            }
        }
    }
}

#[test]
fn the_expansion_is_what_an_authored_source_with_that_expansion_makes() {
    let (rate, dt) = (800.0, 0.1);
    let ambient = spec().ambient_temperature;
    let expansion = rate * dt / ((ambient + rate * dt) * dt);
    let mut derived = Simulation::new(spec()).unwrap();
    let mut authored = Simulation::new(spec()).unwrap();
    derived.step(&Inputs { heated: vec![heated_source(rate)], ..Inputs::default() }).unwrap();
    authored.step(&Inputs { sources: vec![Source { expansion, ..heated_source(rate) }], ..Inputs::default() }).unwrap();
    let (a, b) = (derived.state().temperature(), authored.state().temperature());
    assert_eq!(a, b, "the heat is the same");
    for p in [[2.0, 3.0, 4.0], [0.5, 7.0, 3.0]] {
        let (u, v) = (derived.state().velocity_at(p), authored.state().velocity_at(p));
        for k in 0..3 {
            assert!((u[k] - v[k]).abs() <= 1e-9 * v[k].abs().max(1e-12), "{u:?} {v:?}");
        }
    }
}

#[test]
fn a_heated_impulse_expands_once_in_its_step() {
    let ambient = spec().ambient_temperature;
    let mut sim = Simulation::new(spec()).unwrap();
    let impulse =
        Impulse { shape: whole(), time: 0.2, density: 1.0, temperature: 300.0, velocity: [0.0; 3], expansion: 0.0 };
    let input = Inputs { heated_impulses: vec![impulse], ..Inputs::default() };
    sim.step(&input).unwrap();
    sim.step(&input).unwrap();
    assert!(sim.state().divergence_at([3, 3, 3]).abs() < 1e-9, "nothing before its step");
    sim.step(&input).unwrap();
    let want = 300.0 / ((ambient + 300.0) * 0.1);
    assert!((sim.state().divergence_at([3, 3, 3]) - want).abs() < 1e-7 * want);
    sim.step(&input).unwrap();
    assert!(sim.state().divergence_at([3, 3, 3]).abs() < 1e-7, "and not after it");
}

#[test]
fn a_source_with_no_heat_has_no_expansion_and_authored_expansion_is_refused() {
    let mut sim = Simulation::new(spec()).unwrap();
    sim.step(&Inputs { heated: vec![Source { temperature_rate: 0.0, ..heated_source(0.0) }], ..Inputs::default() })
        .unwrap();
    assert!(sim.state().divergence_at([3, 3, 3]).abs() < 1e-9);
    let mut refused = Simulation::new(spec()).unwrap();
    let bad = Inputs { heated: vec![Source { expansion: 1.0, ..heated_source(100.0) }], ..Inputs::default() };
    assert!(refused.step(&bad).is_err(), "the expansion of a heated source is derived");
    assert_eq!(refused.time(), 0.0, "failed steps are atomic");
}

#[test]
fn sources_that_do_not_ask_for_it_are_unchanged() {
    // the same step with no heated sources is the step there always was
    let mut a = Simulation::new(spec()).unwrap();
    let mut b = Simulation::new(spec()).unwrap();
    let plain = Inputs { sources: vec![heated_source(500.0)], ..Inputs::default() };
    a.step(&plain).unwrap();
    b.step(&Inputs { heated: vec![], heated_impulses: vec![], ..plain.clone() }).unwrap();
    assert_eq!(a.state(), b.state());
}
