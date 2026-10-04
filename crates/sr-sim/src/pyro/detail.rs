//! Detail retention: a free vortex loses energy to numerical diffusion; the
//! MacCormack scheme must retain more of it than semi-Lagrangian without ever
//! gaining energy.

use super::*;

const N: usize = 32;
const DT: f64 = 0.1;

fn spec(advection: Advection) -> Spec {
    Spec {
        cells: [N; 3],
        voxel_size: 1.0,
        origin: [0.0; 3],
        dt: DT,
        boundary: Boundary::Closed,
        advection,
        solver: PressureSolver::Multigrid,
        pressure_iterations: 400,
        pressure_tolerance: 1e-6,
        ..Spec::default()
    }
}

/// Gaussian swirl about the z axis through the box centre.
fn swirl() -> Vec<[f64; 3]> {
    let c = 0.5 * N as f64;
    (0..N * N * N)
        .map(|k| {
            let p = coords(k, [N; 3]).map(|v| v as f64 + 0.5);
            let (x, y) = (p[0] - c, p[1] - c);
            let g = 10.0 * (-(x * x + y * y) / 36.0).exp();
            [-y * g / 6.0, x * g / 6.0, 0.0]
        })
        .collect()
}

/// Kinetic energy and enstrophy of the face velocities.
fn measures(s: &State) -> (f64, f64) {
    let kinetic = 0.5 * s.velocity.iter().flatten().map(|v| v * v).sum::<f64>();
    let mut enstrophy = 0.0;
    for k in 0..s.density.len() {
        let c = coords(k, s.cells);
        if (0..3).any(|a| c[a] == 0 || c[a] + 1 == s.cells[a]) {
            continue;
        }
        let p = c.map(|v| v as f64 + 0.5);
        let d: [[f64; 3]; 3] = std::array::from_fn(|a| {
            let (mut lo, mut hi) = (p, p);
            lo[a] -= 1.0;
            hi[a] += 1.0;
            let (vl, vh) = (s.velocity_grid(lo), s.velocity_grid(hi));
            std::array::from_fn(|b| (vh[b] - vl[b]) / 2.0)
        });
        let w = [d[1][2] - d[2][1], d[2][0] - d[0][2], d[0][1] - d[1][0]];
        enstrophy += w.iter().map(|v| v * v).sum::<f64>();
    }
    (kinetic, enstrophy)
}

fn run(advection: Advection, free_steps: usize) -> Vec<(f64, f64)> {
    let mut sim = Simulation::new(spec(advection)).unwrap();
    let spin = Inputs { spatial_acceleration: swirl(), ..Inputs::default() };
    for _ in 0..3 {
        sim.step(&spin).unwrap();
    }
    let mut history = vec![measures(sim.state())];
    for _ in 0..free_steps {
        sim.step(&Inputs::default()).unwrap();
        history.push(measures(sim.state()));
    }
    history
}

#[test]
fn maccormack_retains_vortex_energy_and_enstrophy_and_never_gains_energy() {
    let sl = run(Advection::SemiLagrangian, 60);
    let mc = run(Advection::MacCormack, 60);
    // Free evolution: kinetic energy must not increase from one step to the next.
    for w in mc.windows(2) {
        assert!(w[1].0 <= w[0].0, "kinetic energy grew: {} -> {}", w[0].0, w[1].0);
    }
    // Declared factors, with margin below what is measured (1.41 and 1.55).
    assert!(mc[60].0 >= 1.25 * sl[60].0, "kinetic energy {} vs {}", mc[60].0, sl[60].0);
    assert!(mc[60].1 >= 1.3 * sl[60].1, "enstrophy {} vs {}", mc[60].1, sl[60].1);
}

#[test]
fn maccormack_creates_no_new_extrema_and_keeps_solids_clean() {
    let mut s = spec(Advection::MacCormack);
    s.ambient_temperature = 300.0;
    let mut sim = Simulation::new(s).unwrap();
    let c = 0.5 * N as f64;
    let blob = Impulse {
        shape: Shape::Sphere { center: [c + 6.0, c, c], radius: 3.5 },
        time: 0.0,
        density: 1.0,
        temperature: 500.0,
        velocity: [0.0; 3],
        expansion: 0.0,
    };
    let wall = Obstacle::stationary(Shape::Box { min: [c - 3.0, c + 4.0, 0.0], max: [c + 3.0, c + 6.0, N as f64] });
    let input = |first: bool| Inputs {
        impulses: if first { vec![blob.clone()] } else { Vec::new() },
        obstacles: vec![wall.clone()],
        spatial_acceleration: if first { swirl() } else { Vec::new() },
        ..Inputs::default()
    };
    for step in 0..40 {
        sim.step(&input(step == 0)).unwrap();
        let state = sim.state();
        assert!(state.density.iter().all(|d| (0.0..=1.0 + 1e-12).contains(d)), "density escaped its range at {step}");
        assert!(state.temperature.iter().all(|t| (300.0..=800.0 + 1e-9).contains(t)), "temperature escaped at {step}");
        for k in (0..state.solid.len()).filter(|&k| state.solid[k]) {
            assert!(state.density[k] == 0.0 && state.temperature[k] == 300.0, "solid cell {k} holds smoke at {step}");
        }
    }
    assert!(sim.state().density.iter().any(|&d| d > 0.05), "the blob should still exist");
}
