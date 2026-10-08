//! Fluid pockets sealed by solids make the pressure operator singular on their
//! own. A consistent pocket must solve with either solver; a pocket forced to
//! expand has no pressure that can remove its divergence and must fail with
//! `Error::Pressure`, never produce a silent result.

use super::*;

const SOLVERS: [PressureSolver; 2] = [PressureSolver::Jacobi, PressureSolver::Multigrid];

fn spec(boundary: Boundary, solver: PressureSolver) -> Spec {
    Spec {
        cells: [16, 16, 16],
        voxel_size: 1.0,
        origin: [0.0; 3],
        dt: 1.0 / 24.0,
        boundary,
        solver,
        buoyancy: 0.01,
        vorticity: 0.3,
        turbulence: 0.5,
        seed: 3,
        pressure_iterations: 2000,
        pressure_tolerance: 1e-3,
        ..Spec::default()
    }
}

/// A 2-cell-thick closed shell around the fluid pocket [6,10)^3.
fn shell() -> Vec<Obstacle> {
    [
        ([4.0, 4.0, 4.0], [6.0, 12.0, 12.0]),
        ([10.0, 4.0, 4.0], [12.0, 12.0, 12.0]),
        ([6.0, 4.0, 4.0], [10.0, 6.0, 12.0]),
        ([6.0, 10.0, 4.0], [10.0, 12.0, 12.0]),
        ([6.0, 6.0, 4.0], [10.0, 10.0, 6.0]),
        ([6.0, 6.0, 10.0], [10.0, 10.0, 12.0]),
    ]
    .into_iter()
    .map(|(min, max)| Obstacle::stationary(Shape::Box { min, max }))
    .collect()
}

fn pocket_source(expansion: f64) -> Source {
    Source {
        shape: Shape::Sphere { center: [8.0, 8.0, 8.0], radius: 1.6 },
        density_rate: 5.0,
        temperature_rate: 500.0,
        velocity_rate: [3.0, -4.0, 2.0],
        expansion,
        ..Source::default()
    }
}

fn outside_source() -> Source {
    Source {
        shape: Shape::Sphere { center: [2.0, 2.5, 2.0], radius: 1.5 },
        density_rate: 4.0,
        temperature_rate: 300.0,
        velocity_rate: [1.0, 2.0, 0.5],
        ..Source::default()
    }
}

fn inputs(expansion: f64) -> Inputs {
    Inputs { obstacles: shell(), sources: vec![pocket_source(expansion), outside_source()], ..Inputs::default() }
}

fn run(boundary: Boundary, solver: PressureSolver, expansion: f64, steps: usize) -> Result<Simulation, Error> {
    let mut sim = Simulation::new(spec(boundary, solver))?;
    for _ in 0..steps {
        sim.step(&inputs(expansion))?;
    }
    Ok(sim)
}

fn finite(sim: &Simulation) -> bool {
    let s = sim.state();
    s.density.iter().chain(&s.temperature).chain(s.velocity.iter().flatten()).all(|v| v.is_finite())
}

#[test]
fn a_consistent_sealed_pocket_solves_in_a_closed_domain() {
    for solver in SOLVERS {
        let sim = run(Boundary::Closed, solver, 0.0, 6).unwrap_or_else(|e| panic!("{solver:?}: {e}"));
        assert!(finite(&sim), "{solver:?}");
        // The pocket is moving: its source pushed velocity into sealed fluid.
        assert!(sim.state().velocity_at([8.0, 8.0, 8.0]).iter().any(|v| *v != 0.0), "{solver:?}");
    }
}

#[test]
fn a_consistent_sealed_pocket_solves_in_an_open_domain() {
    for solver in SOLVERS {
        let sim = run(Boundary::Open, solver, 0.0, 6).unwrap_or_else(|e| panic!("{solver:?}: {e}"));
        assert!(finite(&sim), "{solver:?}");
    }
}

#[test]
fn an_expanding_sealed_pocket_fails_with_a_pressure_error_in_both_domains() {
    for boundary in [Boundary::Closed, Boundary::Open] {
        for solver in SOLVERS {
            let Err(error) = run(boundary, solver, 4.0, 3) else {
                panic!("{solver:?} {boundary:?}: expanding sealed pocket must not solve");
            };
            // The unremovable divergence of the pocket is what trips the check.
            // and it says where the residual is largest, which is in the pocket
            let Error::Pressure { residual, worst: Some(worst) } = &error else {
                panic!("{solver:?} {boundary:?}: {error}");
            };
            assert!(*residual > 1e-3, "{solver:?} {boundary:?}: {error}");
            assert!(worst.cell.iter().all(|c| (6..10).contains(c)), "{solver:?} {boundary:?}: {error}");
            assert!(error.to_string().contains("largest at cell ["), "{error}");
        }
    }
}

#[test]
fn both_solvers_agree_on_a_sealed_pocket_scene_when_converged_tightly() {
    for boundary in [Boundary::Closed, Boundary::Open] {
        let tight = |solver| {
            let mut s = spec(boundary, solver);
            s.pressure_tolerance = 1e-10;
            s.pressure_iterations = 5000;
            let mut sim = Simulation::new(s).unwrap();
            for _ in 0..4 {
                sim.step(&inputs(0.0)).unwrap();
            }
            sim
        };
        let (jacobi, multigrid) = (tight(PressureSolver::Jacobi), tight(PressureSolver::Multigrid));
        let (a, b) = (jacobi.state(), multigrid.state());
        let scale = a.velocity.iter().flatten().fold(0.0_f64, |m, v| m.max(v.abs())).max(1e-12);
        let worst = a
            .velocity
            .iter()
            .flatten()
            .zip(b.velocity.iter().flatten())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f64, f64::max);
        assert!(worst <= 1e-6 * scale, "{boundary:?}: velocities differ by {worst} (scale {scale})");
    }
}

#[test]
fn sealed_pocket_results_do_not_depend_on_the_thread_count() {
    let on_pool = |threads: usize, boundary| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| run(boundary, PressureSolver::Multigrid, 0.0, 5).unwrap())
    };
    for boundary in [Boundary::Closed, Boundary::Open] {
        let one = on_pool(1, boundary);
        for threads in [2, 8] {
            assert!(on_pool(threads, boundary).state() == one.state(), "{boundary:?}: {threads} threads");
        }
    }
}
