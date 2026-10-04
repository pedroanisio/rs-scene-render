//! Bit-exactness guard for the smoke solver: reference scenarios hashed over the
//! complete final state and every step report, run on local rayon pools of 1, 2
//! and 8 threads. The Jacobi golden hashes were captured at commit 60a458c, before
//! any performance work; a performance change that alters one of them changes
//! results. The multigrid hashes pin the multigrid solver's own contract.

use super::*;
use sr_volume::Transform;

#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    cells: [usize; 3],
    voxel: f64,
    steps: u64,
    boundary: Boundary,
    solver: PressureSolver,
    /// Moving/deforming colliders, meshes, spatial acceleration and uniform acceleration.
    rich: bool,
    golden: u64,
}

const CASES: [Case; 6] = [
    Case {
        name: "open-rich-odd",
        cells: [23, 19, 17],
        voxel: 4.0,
        steps: 8,
        boundary: Boundary::Open,
        solver: PressureSolver::Jacobi,
        rich: true,
        golden: 0x0c4c2dbcce07af55,
    },
    Case {
        name: "closed-rich",
        cells: [20, 16, 24],
        voxel: 4.0,
        steps: 8,
        boundary: Boundary::Closed,
        solver: PressureSolver::Jacobi,
        rich: true,
        golden: 0x36695446882e6a75,
    },
    Case {
        name: "open-cinematic",
        cells: [40, 32, 40],
        voxel: 4.8,
        steps: 6,
        boundary: Boundary::Open,
        solver: PressureSolver::Jacobi,
        rich: false,
        golden: 0xd0f216a928158341,
    },
    Case {
        name: "open-rich-odd-multigrid",
        cells: [23, 19, 17],
        voxel: 4.0,
        steps: 8,
        boundary: Boundary::Open,
        solver: PressureSolver::Multigrid,
        rich: true,
        golden: 0xd693c122f462feee,
    },
    Case {
        name: "closed-rich-multigrid",
        cells: [20, 16, 24],
        voxel: 4.0,
        steps: 8,
        boundary: Boundary::Closed,
        solver: PressureSolver::Multigrid,
        rich: true,
        golden: 0x8d838d7de47098a2,
    },
    Case {
        name: "open-cinematic-multigrid",
        cells: [40, 32, 40],
        voxel: 4.8,
        steps: 6,
        boundary: Boundary::Open,
        solver: PressureSolver::Multigrid,
        rich: false,
        golden: 0x50dfd08cc0fe0d3e,
    },
];

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn f64s(&mut self, values: &[f64]) {
        values.iter().for_each(|v| self.bytes(&v.to_bits().to_le_bytes()));
    }
    fn vec3s(&mut self, values: &[[f64; 3]]) {
        values.iter().for_each(|v| self.f64s(v));
    }
}

fn spec(c: &Case) -> Spec {
    let extent = c.cells.map(|n| n as f64 * c.voxel);
    Spec {
        cells: c.cells,
        origin: extent.map(|e| -0.5 * e),
        voxel_size: c.voxel,
        dt: 1.0 / 24.0,
        boundary: c.boundary,
        dissipation: 0.15,
        cooling: 0.4,
        buoyancy: 0.003,
        vorticity: 0.5,
        turbulence: 2.0,
        seed: 20_261_003,
        pressure_iterations: 120,
        pressure_tolerance: 1e-3,
        solver: c.solver,
        max_bytes: 1 << 30,
        ..Spec::default()
    }
}

fn cube(radius: f64, centre: [f64; 3]) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let vertices = [
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
    ]
    .map(|p| std::array::from_fn(|a| p[a] * radius + centre[a]))
    .to_vec();
    let faces = vec![
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [3, 7, 6],
        [3, 6, 2],
        [0, 4, 7],
        [0, 7, 3],
        [1, 2, 6],
        [1, 6, 5],
    ];
    (vertices, faces)
}

fn inputs(c: &Case, spec: &Spec) -> Inputs {
    let extent = c.cells.map(|n| n as f64 * c.voxel);
    let at = |f: [f64; 3]| -> [f64; 3] { std::array::from_fn(|a| spec.origin[a] + f[a] * extent[a]) };
    let open = if c.boundary == Boundary::Open { 1.0 } else { 0.0 };
    let r = extent[0];
    let mut input = Inputs {
        sources: vec![Source {
            shape: Shape::Sphere { center: at([0.5, 0.75, 0.5]), radius: 0.09 * r },
            start: 2.0 * spec.dt,
            end: Some(6.0 * spec.dt),
            density_rate: 4.0,
            temperature_rate: 1200.0,
            velocity_rate: [0.0, -20.0, 0.0],
            expansion: 2.0 * open,
        }],
        impulses: vec![Impulse {
            shape: Shape::Sphere { center: at([0.5, 0.8, 0.5]), radius: 0.12 * r },
            time: 3.0 * spec.dt,
            density: 0.8,
            temperature: 3000.0,
            velocity: [0.0, -35.0, 0.0],
            expansion: 2.0 * open,
        }],
        obstacles: vec![Obstacle::stationary(Shape::Box { min: at([0.0, 0.0, 0.0]), max: at([1.0, 0.12, 1.0]) })],
        ..Inputs::default()
    };
    if c.rich {
        let (v, f) = cube(0.07 * r, at([0.7, 0.5, 0.6]));
        let (v0, f0) = cube(0.05 * r, at([0.25, 0.55, 0.7]));
        let end: Vec<_> = v0.iter().map(|p| [p[0] + 0.02 * r, p[1] + 0.03 * r, p[2]]).collect();
        let scale = Transform::new([
            1.0,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            at([0.4, 0.3, 0.4])[0],
            at([0.4, 0.3, 0.4])[1],
            at([0.4, 0.3, 0.4])[2],
            1.0,
        ])
        .unwrap();
        input.obstacles.extend([
            Obstacle {
                velocity: [0.2, -0.3, 0.4],
                velocity_origin: at([0.3, 0.5, 0.4]),
                velocity_gradient: [[0.0, -0.05, 0.0], [0.05, 0.0, 0.0], [0.0, 0.0, 0.01]],
                ..Obstacle::stationary(Shape::Sphere { center: at([0.3, 0.5, 0.4]), radius: 0.1 * r })
            },
            // Overlaps the seabed box and the sphere: the first listed collider must win.
            Obstacle::stationary(Shape::Transformed {
                shape: Box::new(Shape::Cylinder { radius: 0.08 * r, half_height: 0.2 * extent[1] }),
                transform: Box::new(scale),
            }),
            Obstacle::stationary(Shape::Mesh(std::sync::Arc::new(mesh::Mesh::new(&v, &f, 1 << 20).unwrap()))),
            Obstacle::stationary(Shape::Mesh(std::sync::Arc::new(
                mesh::Mesh::moving(&v0, &end, &f0, spec.dt, 1 << 20).unwrap(),
            ))),
            Obstacle::stationary(Shape::Capsule { radius: 0.04 * r, half_segment: 0.06 * r }),
            Obstacle::stationary(Shape::Torus { major_radius: 0.12 * r, minor_radius: 0.03 * r }),
            Obstacle::stationary(Shape::Cone { radius: 0.06 * r, half_height: 0.08 * r }),
        ]);
        input.acceleration = [0.1, -0.2, 0.05];
        let count: usize = c.cells.iter().product();
        input.spatial_acceleration =
            (0..count).map(|k| std::array::from_fn(|a| 0.3 * crate::rng::signed(5, k as u64, a as u64))).collect();
        input.sources.push(Source {
            shape: Shape::Box { min: at([0.4, 0.2, 0.1]), max: at([0.6, 0.35, 0.3]) },
            start: 0.0,
            end: None,
            density_rate: 1.0,
            temperature_rate: 100.0,
            velocity_rate: [1.0, 2.0, 3.0],
            expansion: 0.0,
        });
    }
    input
}

/// Final state plus a hash of every step report and every state array.
fn run(c: &Case) -> (u64, State, Vec<usize>) {
    let spec = spec(c);
    let input = inputs(c, &spec);
    let mut sim = Simulation::new(spec).unwrap();
    let mut hash = Fnv::new();
    let mut iterations = Vec::new();
    for _ in 0..c.steps {
        let report = sim.step(&input).unwrap_or_else(|e| panic!("{}: {e}", c.name));
        hash.f64s(&[report.divergence_before, report.divergence_after]);
        hash.bytes(&(report.pressure_iterations as u64).to_le_bytes());
        iterations.push(report.pressure_iterations);
    }
    let s = sim.state();
    hash.f64s(&s.density);
    hash.f64s(&s.temperature);
    s.velocity.iter().for_each(|v| hash.f64s(v));
    hash.bytes(&s.solid.iter().map(|&b| u8::from(b)).collect::<Vec<_>>());
    hash.vec3s(&s.solid_velocity_low);
    hash.vec3s(&s.solid_velocity_high);
    (hash.0, s.clone(), iterations)
}

fn on_pool<T: Send>(threads: usize, f: impl FnOnce() -> T + Send) -> T {
    rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(f)
}

#[test]
fn results_are_identical_for_one_two_and_eight_threads() {
    for c in &CASES {
        let (one, state, _) = on_pool(1, || run(c));
        for threads in [2, 8] {
            let (hash, other, _) = on_pool(threads, || run(c));
            assert_eq!(hash, one, "{}: {threads} threads changed the result hash", c.name);
            assert!(other == state, "{}: {threads} threads changed the final state", c.name);
        }
    }
}

/// Pins the exact bits produced by the unoptimized solver (golden captured at
/// 60a458c on x86-64 Linux/glibc; libm `exp` is platform-defined, so other
/// targets only get the thread-invariance test above).
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
#[test]
fn final_state_matches_the_reference_hashes() {
    for c in &CASES {
        let (hash, _, _) = run(c);
        assert_eq!(hash, c.golden, "{}: golden {:#018x}, got {hash:#018x}", c.name, c.golden);
    }
}

/// `cargo test --release -p sr-sim --lib -- --ignored --nocapture print_hashes`
#[test]
#[ignore = "prints the hashes used to pin the reference"]
fn print_hashes() {
    for c in &CASES {
        let (hash, state, iterations) = run(c);
        let solid = state.solid.iter().filter(|&&b| b).count();
        println!("{} {hash:#018x} solid={solid} cg={iterations:?}", c.name);
    }
}

/// The point of the multigrid preconditioner: iteration counts stay low (observed at most 9), and do
/// not grow with resolution, on the impact-like scene (impulse at step 3).
#[test]
fn multigrid_iteration_counts_stay_low_as_resolution_grows() {
    let mut worst = Vec::new();
    for (cells, voxel) in [([12, 10, 12], 16.0), ([24, 20, 24], 8.0), ([48, 39, 48], 4.0)] {
        let case = Case {
            name: "resolution",
            cells,
            voxel,
            steps: 6,
            boundary: Boundary::Open,
            solver: PressureSolver::Multigrid,
            rich: false,
            golden: 0,
        };
        let (_, _, iterations) = run(&case);
        let most = *iterations.iter().max().unwrap();
        assert!(most <= 12, "{cells:?}: {iterations:?}");
        worst.push(most);
    }
    assert!(worst[2] <= worst[0] + 8, "iterations grew with resolution: {worst:?}");
}
