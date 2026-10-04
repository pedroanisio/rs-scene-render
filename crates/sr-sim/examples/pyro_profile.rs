//! Per-stage timing of the smoke solver on a cinematic-impact-like plume.
//!
//! `cargo run --release -p sr-sim --example pyro_profile -- [cells-x] [steps] [pressure-iterations] [colliders] [jacobi|multigrid]`
//!
//! The domain is 192 x 156 x 192 scene units like `examples/cinematic-impact`
//! (open edges, impulse + expanding source, buoyancy, vorticity, turbulence);
//! `cells-x` sets the resolution (64 -> 64x52x64, 128 -> 128x104x128). A nonzero
//! `colliders` adds a slab and a sphere to exercise voxelization.
//! The impulse lands at step 2 so the short run is dominated by the pressure solve.
//! Control threads with `RAYON_NUM_THREADS`.

use sr_sim::pyro::{Boundary, Impulse, Inputs, Obstacle, PressureSolver, Shape, Simulation, Source, Spec, StepProfile};

fn main() {
    let mut args = std::env::args().skip(1);
    let nx: usize = args.next().map_or(64, |v| v.parse().expect("cells-x"));
    let steps: usize = args.next().map_or(12, |v| v.parse().expect("steps"));
    let iterations: usize = args.next().map_or(200, |v| v.parse().expect("pressure-iterations"));
    let colliders = args.next().is_some_and(|v| v != "0");
    let solver = match args.next().as_deref() {
        None | Some("jacobi") => PressureSolver::Jacobi,
        Some("multigrid") => PressureSolver::Multigrid,
        Some(other) => panic!("solver must be jacobi or multigrid, not {other}"),
    };
    let (extent, height) = (192.0, 156.0);
    let voxel = extent / nx as f64;
    let ny = (height / voxel).round() as usize;
    let dt = 1.0 / 24.0;
    let spec = Spec {
        cells: [nx, ny, nx],
        origin: [-96.0, -78.0, -96.0],
        voxel_size: voxel,
        dt,
        boundary: Boundary::Open,
        dissipation: 0.15,
        cooling: 0.4,
        buoyancy: 0.003,
        vorticity: 0.5,
        turbulence: 2.0,
        seed: 20_261_003,
        pressure_iterations: iterations,
        pressure_tolerance: 1e-3,
        solver,
        max_bytes: 16 << 30,
        ..Spec::default()
    };
    let input = Inputs {
        impulses: vec![Impulse {
            shape: Shape::Sphere { center: [0.0, 35.0, 0.0], radius: 22.0 },
            time: 2.0 * dt,
            density: 0.8,
            temperature: 3000.0,
            velocity: [0.0, -35.0, 0.0],
            expansion: 2.0,
        }],
        sources: vec![Source {
            shape: Shape::Sphere { center: [0.0, 30.0, 0.0], radius: 16.0 },
            start: 2.0 * dt,
            end: Some(14.0 * dt),
            density_rate: 4.0,
            temperature_rate: 1200.0,
            velocity_rate: [0.0, -20.0, 0.0],
            expansion: 2.0,
        }],
        obstacles: if colliders {
            vec![
                Obstacle::stationary(Shape::Box { min: [-96.0, 66.0, -96.0], max: [96.0, 78.0, 96.0] }),
                Obstacle::stationary(Shape::Sphere { center: [40.0, 0.0, 10.0], radius: 14.0 }),
            ]
        } else {
            Vec::new()
        },
        ..Inputs::default()
    };
    let mut sim = Simulation::new(spec).expect("spec");
    println!("{nx}x{ny}x{nx} cells, voxel {voxel:.3}, {} rayon threads", rayon::current_num_threads());
    println!("step  cg   total  clone  obst   bnd  advect(copy)  inject forces  p.setup  p.apply p.precond p.reduce p.update p.finish  project   (ms)");
    let mut sum = StepProfile::default();
    let mut counted = 0;
    for step in 0..steps {
        let (_, p) = sim.step_profiled(&input).expect("step");
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1e3;
        println!(
            "{step:>4} {:>4} {:>7.1} {:>6.1} {:>5.1} {:>5.1} {:>7.1}({:>5.1}) {:>6.1} {:>6.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1}",
            p.pressure_iterations,
            ms(p.total),
            ms(p.clone),
            ms(p.obstacles),
            ms(p.boundaries_validate),
            ms(p.advect),
            ms(p.advect_clone),
            ms(p.inject),
            ms(p.forces),
            ms(p.project_setup),
            ms(p.project_apply),
            ms(p.project_precondition),
            ms(p.project_reduce),
            ms(p.project_update),
            ms(p.project_finish),
            ms(p.project_setup
                + p.project_apply
                + p.project_precondition
                + p.project_reduce
                + p.project_update
                + p.project_finish),
        );
        if step >= 3 {
            counted += 1;
            sum.pressure_iterations += p.pressure_iterations;
            sum.total += p.total;
            sum.clone += p.clone;
            sum.obstacles += p.obstacles;
            sum.boundaries_validate += p.boundaries_validate;
            sum.advect += p.advect;
            sum.advect_clone += p.advect_clone;
            sum.inject += p.inject;
            sum.forces += p.forces;
            sum.project_setup += p.project_setup;
            sum.project_apply += p.project_apply;
            sum.project_precondition += p.project_precondition;
            sum.project_reduce += p.project_reduce;
            sum.project_update += p.project_update;
            sum.project_finish += p.project_finish;
        }
    }
    if counted > 0 {
        let n = counted as f64;
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1e3 / n;
        println!(
            "mean (steps >= 3, n={counted}): cg {:.1}, total {:.1} ms = clone {:.1} + obstacles {:.1} + bnd/validate {:.1} + advect {:.1} (copy {:.1}) + inject {:.1} + forces {:.1} + project {:.1} (setup {:.1}, apply {:.1}, precond {:.1}, reduce {:.1}, update {:.1}, finish {:.1})",
            sum.pressure_iterations as f64 / n,
            ms(sum.total),
            ms(sum.clone),
            ms(sum.obstacles),
            ms(sum.boundaries_validate),
            ms(sum.advect),
            ms(sum.advect_clone),
            ms(sum.inject),
            ms(sum.forces),
            ms(sum.project_setup
                + sum.project_apply
                + sum.project_precondition
                + sum.project_reduce
                + sum.project_update
                + sum.project_finish),
            ms(sum.project_setup),
            ms(sum.project_apply),
            ms(sum.project_precondition),
            ms(sum.project_reduce),
            ms(sum.project_update),
            ms(sum.project_finish),
        );
    }
}
