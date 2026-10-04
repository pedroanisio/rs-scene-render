//! A step that fails at any stage leaves the previous state and clock untouched,
//! and a corrected retry matches a fresh replay. Failures are injected in the
//! collider stage, field validation and the pressure solve. Advection errors are
//! not injected: the midpoint of an over-long trace leaves an open domain, where
//! velocity is zero, so the 2048-voxel limit is unreachable below ~1000 cells.

use super::*;

fn spec(boundary: Boundary) -> Spec {
    Spec {
        cells: [16, 12, 14],
        voxel_size: 1.0,
        origin: [-8.0, -6.0, -7.0],
        dt: 1.0 / 24.0,
        boundary,
        dissipation: 0.1,
        cooling: 0.3,
        buoyancy: 0.01,
        vorticity: 0.5,
        turbulence: 1.0,
        seed: 7,
        pressure_iterations: 5000,
        pressure_tolerance: 1e-3,
        ..Spec::default()
    }
}

fn good(open: bool) -> Inputs {
    Inputs {
        sources: vec![Source {
            shape: Shape::Sphere { center: [0.0, 2.0, 0.0], radius: 2.0 },
            density_rate: 3.0,
            temperature_rate: 400.0,
            velocity_rate: [0.0, -5.0, 0.0],
            expansion: if open { 1.0 } else { 0.0 },
            ..Source::default()
        }],
        obstacles: vec![Obstacle {
            velocity: [0.1, 0.0, 0.0],
            ..Obstacle::stationary(Shape::Box { min: [-8.0, 3.0, -7.0], max: [8.0, 6.0, 7.0] })
        }],
        ..Inputs::default()
    }
}

fn warm(boundary: Boundary) -> Simulation {
    let mut sim = Simulation::new(spec(boundary)).unwrap();
    for _ in 0..3 {
        sim.step(&good(boundary == Boundary::Open)).unwrap();
    }
    sim
}

fn assert_atomic(boundary: Boundary, bad: Inputs, what: &str) {
    let mut sim = warm(boundary);
    let before = sim.state().clone();
    let time = sim.time();
    let error = sim.step(&bad).expect_err(what);
    assert!(sim.state() == &before, "{what}: state changed by a failed step ({error})");
    assert_eq!(sim.time(), time, "{what}: clock advanced by a failed step");
    let retry = good(boundary == Boundary::Open);
    sim.step(&retry).unwrap_or_else(|e| panic!("{what}: retry failed: {e}"));
    let mut fresh = warm(boundary);
    fresh.step(&retry).unwrap();
    assert!(sim.state() == fresh.state(), "{what}: retry differs from a fresh replay");
}

#[test]
fn collider_stage_failure_is_atomic() {
    let bad = Inputs {
        obstacles: vec![Obstacle {
            velocity_gradient: [[1e308, 0.0, 0.0], [0.0; 3], [0.0; 3]],
            velocity_origin: [-100.0; 3],
            ..Obstacle::stationary(Shape::Sphere { center: [0.0, -3.0, 0.0], radius: 2.0 })
        }],
        ..good(true)
    };
    assert_atomic(Boundary::Open, bad, "nonfinite collider velocity");
}

#[test]
fn validation_stage_failure_is_atomic() {
    let mut bad = good(true);
    bad.sources[0].temperature_rate = 1e9;
    assert_atomic(Boundary::Open, bad, "temperature beyond 50000 K");
}

#[test]
fn pressure_stage_failure_is_atomic() {
    let mut bad = good(false);
    bad.sources[0].expansion = 5.0;
    assert_atomic(Boundary::Closed, bad, "sealed domain with expansion");
}
