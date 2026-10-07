//! A blast in the smoke: the front of a Sedov-Taylor blast as an incompressible spherical piston. The oracles are the volume that the front
//! sweeps (the air displaced is exactly that), the potential flow outside the sphere, the displacement of the smoke by it, the units, and that
//! a scene with no blast is the scene it was.
// the tests index three axes at a time
#![allow(clippy::needless_range_loop)]

use sr_sim::pyro::{Blast, Boundary, Impulse, Inputs, Shape, Simulation, Spec};
use sr_sim::sedov::solve;

const RHO0: f64 = 1.2;
const P0: f64 = 101_325.0;
const GAMMA: f64 = 1.4;
/// 3.75e9 J: the strong phase ends at 0.3 (E / p0)^(1/3) = 9.9975 m, after about 5 ms.
const ENERGY: f64 = 3.75e9;
const DT: f64 = 5e-4;

fn blast(ppm: f64, center: [f64; 3], time: f64) -> Blast {
    Blast::new(center, time, ENERGY, RHO0, P0, GAMMA, ppm).unwrap()
}

fn domain(n: usize, h: f64, boundary: Boundary) -> Spec {
    let half = n as f64 * h / 2.0;
    Spec {
        cells: [n; 3],
        origin: [-half; 3],
        voxel_size: h,
        dt: DT,
        boundary,
        pressure_iterations: 2000,
        pressure_tolerance: 1e-8,
        max_bytes: 1 << 32,
        ..Spec::default()
    }
}

#[test]
fn the_radius_of_the_piston_is_the_sedov_front_up_to_the_end_of_the_strong_phase_and_then_it_stops() {
    let b = blast(1.0, [0.0; 3], 0.0);
    let s = solve(GAMMA).unwrap();
    let r_max = s.max_radius(ENERGY, P0);
    assert!((r_max - 9.9975).abs() < 1e-3, "{r_max}");
    let mut swept = 0.0;
    let mut last = 0.0;
    for step in 0..40u64 {
        let (r0, r1) = b.radii(DT, step);
        // each step starts where the last ended
        assert!((r0 - last).abs() < 1e-12, "step {step}: {r0} against {last}");
        // the end of the step is the front at the end of the step, or the end of the strong phase
        let want = s.radius(ENERGY, RHO0, (step + 1) as f64 * DT).min(r_max);
        assert!((r1 - want).abs() < 1e-9 * want, "step {step}: {r1} against {want}");
        // what the front sweeps in the step is the volume between the two spheres
        match b.swept(DT, step) {
            Some((radius, volume)) => {
                assert_eq!(radius, r1);
                let want = 4.0 / 3.0 * std::f64::consts::PI * (r1.powi(3) - r0.powi(3));
                assert!((volume - want).abs() < 1e-12 * want, "step {step}: {volume} against {want}");
                swept += r1.powi(3) - r0.powi(3);
            }
            None => assert_eq!(r0, r1, "a step in which the front does not move sweeps nothing"),
        }
        last = r1;
    }
    // it stopped at the end of the strong phase, and the volume swept in all is the volume of the sphere of that radius
    assert!((last - r_max).abs() < 1e-9);
    assert!((swept - r_max.powi(3)).abs() < 1e-9 * r_max.powi(3));
    // the blast has nothing to do before it is released: a release at 7.3 steps starts in step 7
    let late = blast(1.0, [0.0; 3], 7.3 * DT);
    assert_eq!(late.radii(DT, 6), (0.0, 0.0));
    let (r0, r1) = late.radii(DT, 7);
    assert_eq!(r0, 0.0);
    // the front at the end of the step in which it was released has had 0.7 of a step
    assert!((r1 - s.radius(ENERGY, RHO0, 0.7 * DT)).abs() < 1e-9 * r1);
}

#[test]
fn a_small_energy_is_one_pulse_of_the_volume_of_the_end_of_the_strong_phase_and_the_energy_that_makes_more_is_the_one_stated(
) {
    // 1e6 J: the strong phase ends at 0.64 m, long before the end of one step of 1/24 s: one pulse of the volume of R_max
    let small = Blast::new([0.0; 3], 0.0, 1e6, RHO0, P0, GAMMA, 1.0).unwrap();
    let dt = 1.0 / 24.0;
    let s = solve(GAMMA).unwrap();
    let r_max = s.max_radius(1e6, P0);
    let (r0, r1) = small.radii(dt, 0);
    assert_eq!(r0, 0.0);
    assert!((r1 - r_max).abs() < 1e-12 && (r_max - 0.6435).abs() < 1e-3, "{r1}");
    let (_, volume) = small.swept(dt, 0).unwrap();
    assert!((volume - 4.0 / 3.0 * std::f64::consts::PI * r_max.powi(3)).abs() < 1e-12 * volume);
    // the volume does not depend on the cell, only the cells that are given it do (a test of the solver below, with an energy of 1 J)
    // the energy above which the strong phase has not ended when the first step does, at this step: E* = [xi0 (dt^2 / rho0)^(1/5) p0^(1/3) / 0.3]^(15/2),
    // 1.91e12 J at 1/24 s and 2.64e10 J at 1/100 s (it goes as dt^3)
    let e_star = |dt: f64| (s.xi0() * (dt * dt / RHO0).powf(0.2) * P0.cbrt() / 0.3).powf(7.5);
    assert!((e_star(1.0 / 24.0) / 1.912e12 - 1.0).abs() < 1e-3, "{}", e_star(1.0 / 24.0));
    assert!((e_star(0.01) / 2.643e10 - 1.0).abs() < 1e-3, "{}", e_star(0.01));
    assert!((e_star(0.01) / e_star(0.02) - 0.125).abs() < 1e-12, "it goes as the cube of the step");
    let below = Blast::new([0.0; 3], 0.0, 0.5 * e_star(dt), RHO0, P0, GAMMA, 1.0).unwrap();
    let above = Blast::new([0.0; 3], 0.0, 2.0 * e_star(dt), RHO0, P0, GAMMA, 1.0).unwrap();
    // below it the first step ends at R_max (one pulse), above it the first step ends inside the strong phase
    let max_below = s.max_radius(0.5 * e_star(dt), P0);
    let max_above = s.max_radius(2.0 * e_star(dt), P0);
    assert!((below.radii(dt, 0).1 - max_below).abs() < 1e-9 * max_below);
    assert!(above.radii(dt, 0).1 < max_above * 0.999);
    // the tabled front for 1e15 J at 1/24 s: 279 m and 369 m after one and two steps, R_max 643.5 m, reached after 8.06 steps (so in the ninth)
    let big = Blast::new([0.0; 3], 0.0, 1e15, RHO0, P0, GAMMA, 1.0).unwrap();
    assert!((big.radii(dt, 0).1 - 279.3).abs() < 0.1 && (big.radii(dt, 1).1 - 368.6).abs() < 0.1);
    assert!((big.radii(dt, 8).1 - 643.5).abs() < 0.1 && big.radii(dt, 7).1 < 643.0);
    assert!(big.swept(dt, 8).is_some() && big.swept(dt, 9).is_none());
    assert!((s.time_at(1e15, RHO0, 643.5) / dt - 8.06).abs() < 0.01);
}

#[test]
fn the_radius_in_scene_units_is_the_radius_in_metres_times_the_pixels_to_the_metre() {
    let metres: Vec<f64> = [1.0, 100.0, 37.0]
        .iter()
        .map(|&ppm| {
            let b = blast(ppm, [0.0; 3], 0.0);
            let (_, r1) = b.radii(DT, 3);
            r1 / ppm
        })
        .collect();
    assert!(
        (metres[0] - metres[1]).abs() < 1e-12 * metres[0] && (metres[0] - metres[2]).abs() < 1e-12 * metres[0],
        "{metres:?}"
    );
    // the volume swept is in cubic scene units: in cubic metres it is the same
    let shares: Vec<f64> =
        [1.0, 100.0, 37.0].iter().map(|&ppm| blast(ppm, [0.0; 3], 0.0).swept(DT, 3).unwrap().1 / ppm.powi(3)).collect();
    assert!((shares[0] / shares[1] - 1.0).abs() < 1e-12 && (shares[0] / shares[2] - 1.0).abs() < 1e-12, "{shares:?}");
}

#[test]
fn a_blast_needs_an_open_domain_and_a_gas_and_an_energy_that_are_something() {
    let closed = Spec {
        cells: [8; 3],
        origin: [-4.0; 3],
        voxel_size: 1.0,
        dt: DT,
        boundary: Boundary::Closed,
        ..Spec::default()
    };
    let mut sim = Simulation::new(closed).unwrap();
    let inputs = Inputs { blasts: vec![blast(1.0, [0.0; 3], 0.0)], ..Inputs::default() };
    let error = sim.step(&inputs).unwrap_err().to_string();
    assert!(error.contains("open"), "{error}");
    for (energy, rho, p, gamma, ppm) in [
        (-1.0, RHO0, P0, GAMMA, 1.0),
        (f64::NAN, RHO0, P0, GAMMA, 1.0),
        (ENERGY, 0.0, P0, GAMMA, 1.0),
        (ENERGY, RHO0, -1.0, GAMMA, 1.0),
        (ENERGY, RHO0, P0, 1.0, 1.0),
        (ENERGY, RHO0, P0, GAMMA, 0.0),
    ] {
        assert!(Blast::new([0.0; 3], 0.0, energy, rho, p, gamma, ppm).is_err(), "{energy} {rho} {p} {gamma} {ppm}");
    }
    assert!(Blast::new([f64::NAN, 0.0, 0.0], 0.0, ENERGY, RHO0, P0, GAMMA, 1.0).is_err());
    assert!(Blast::new([0.0; 3], -1.0, ENERGY, RHO0, P0, GAMMA, 1.0).is_err());
}

fn bits(sim: &Simulation) -> (Vec<u64>, Vec<u64>, [Vec<u64>; 3]) {
    let s = sim.state();
    (
        s.density().iter().map(|v| v.to_bits()).collect(),
        s.temperature().iter().map(|v| v.to_bits()).collect(),
        std::array::from_fn(|a| s.velocity_faces(a).iter().map(|v| v.to_bits()).collect()),
    )
}

#[test]
fn a_scene_with_no_blast_with_one_of_no_energy_or_one_after_the_film_is_the_scene_it_was() {
    let run = |blasts: Vec<Blast>| {
        let spec = Spec { buoyancy: 2.0, turbulence: 0.5, seed: 9, ..domain(16, 0.5, Boundary::Open) };
        let mut sim = Simulation::new(spec).unwrap();
        let puff = Impulse {
            shape: Shape::Sphere { center: [0.0, 1.0, 0.0], radius: 1.0 },
            time: 0.0,
            density: 1.0,
            temperature: 600.0,
            velocity: [0.0; 3],
            expansion: 0.0,
        };
        for step in 0..8 {
            let inputs = Inputs {
                impulses: if step == 0 { vec![puff.clone()] } else { vec![] },
                blasts: blasts.clone(),
                ..Inputs::default()
            };
            sim.step(&inputs).unwrap();
        }
        bits(&sim)
    };
    let reference = run(vec![]);
    assert_eq!(
        run(vec![Blast::new([0.0; 3], 0.0, 0.0, RHO0, P0, GAMMA, 1.0).unwrap()]),
        reference,
        "a blast of no energy"
    );
    assert_eq!(run(vec![blast(1.0, [0.0; 3], 100.0)]), reference, "a blast after the end");
    assert_ne!(run(vec![blast(1.0, [0.0; 3], 0.0)]), reference, "and a blast that is there is not nothing");
}

/// The outflow through the faces of the open domain, in cubic units a second.
fn outflow(sim: &Simulation, h: f64) -> f64 {
    let s = sim.state();
    let n = s.cells();
    let mut total = 0.0;
    for axis in 0..3 {
        let dims: [usize; 3] = std::array::from_fn(|a| n[a] + usize::from(a == axis));
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        for j in 0..n[v] {
            for i in 0..n[u] {
                let mut lo = [0; 3];
                lo[axis] = 0;
                lo[u] = i;
                lo[v] = j;
                let mut hi = lo;
                hi[axis] = n[axis];
                let at = |p: [usize; 3]| p[0] + dims[0] * (p[1] + dims[1] * p[2]);
                total += (s.velocity_faces(axis)[at(hi)] - s.velocity_faces(axis)[at(lo)]) * h * h;
            }
        }
    }
    total
}

#[test]
fn the_air_displaced_is_the_volume_the_front_swept_to_the_cells_that_the_sphere_covers() {
    // a domain of 64 cells of a quarter of a metre (16 m), the front of the first step at about 3.9 m: a sphere of 16 cells, inside the domain
    let h = 0.25;
    let mut sim = Simulation::new(domain(64, h, Boundary::Open)).unwrap();
    let b = blast(1.0, [0.0; 3], 0.0);
    let (_, r1) = b.radii(DT, 0);
    assert!(r1 > 12.0 * h && r1 < 20.0 * h, "the first front is at {r1} metres");
    let inputs = Inputs { blasts: vec![b.clone()], ..Inputs::default() };
    sim.step(&inputs).unwrap();
    // the number of cells whose centres are in the sphere, counted here
    let covered = (0..64usize.pow(3))
        .filter(|&k| {
            let (x, y, z) = (k % 64, k / 64 % 64, k / 4096);
            let c = |i: usize| (i as f64 + 0.5) * h - 32.0 * h;
            let r2 = c(x).powi(2) + c(y).powi(2) + c(z).powi(2);
            r2 <= r1 * r1
        })
        .count() as f64;
    // the air that the domain lets out is exactly the volume that the front swept over the step, whatever the cells make of the sphere
    let (_, swept) = b.swept(DT, 0).unwrap();
    let flux = outflow(&sim, h);
    assert!((flux / (swept / DT) - 1.0).abs() < 1e-6, "the domain lets out {flux} and the front swept {}", swept / DT);
    // and the cells of the sphere are its volume to a few percent at this size
    let volume = 4.0 / 3.0 * std::f64::consts::PI * r1.powi(3);
    assert!((covered * h.powi(3) / volume - 1.0).abs() < 0.05, "{covered} cells against {volume}");
}

#[test]
fn outside_the_sphere_the_flow_is_the_potential_flow_of_the_volume_that_was_swept() {
    let h = 0.25;
    let mut sim = Simulation::new(domain(64, h, Boundary::Open)).unwrap();
    let b = blast(1.0, [0.0; 3], 0.0);
    let (_, r1) = b.radii(DT, 0);
    sim.step(&Inputs { blasts: vec![b], ..Inputs::default() }).unwrap();
    // u_r = Q / (4 pi r^2) with Q = (4 pi / 3) r1^3 / dt, along the x axis at 1.25 and 1.5 radii (the faces are at whole cells; the open faces of the
    // domain are at 32 cells, so the far field is not exactly 1/r^2: this is what the box makes of it, at a tolerance for that)
    let q = 4.0 / 3.0 * std::f64::consts::PI * r1.powi(3) / DT;
    let s = sim.state();
    let n = 64usize;
    let at = |x: usize, y: usize, z: usize| x + (n + 1) * (y + n * z);
    for factor in [1.25, 1.5] {
        let r = factor * r1;
        // the face of the row nearest the axis (its centre is half a cell off it on y and z), at x = r
        let face = (r / h).round() as usize + 32;
        let measured = s.velocity_faces(0)[at(face, 31, 31)];
        let x = (face as f64 - 32.0) * h;
        let dist = (x * x + 0.5 * h * h).sqrt();
        // the radial speed is q / (4 pi dist^2) and its x component is that times x / dist
        let want = q / (4.0 * std::f64::consts::PI * dist * dist) * x / dist;
        assert!((measured / want - 1.0).abs() < 0.15, "at {x}: {measured} against {want}");
    }
}

/// A puff of smoke on the x axis at `distance` metres, then the pulse of a blast of `energy` joules in the next step: the centroid of the smoke
/// before and after (in metres, along x), the total smoke before and after, and the sphere's radius in metres.
fn puff_and_pulse(energy: f64, distance: f64, ppm: f64, cell: f64) -> (f64, f64, f64, f64, f64) {
    let h = cell * ppm;
    let n = (32.0 / cell).round() as usize;
    let mut spec = domain(n, h, Boundary::Open);
    spec.origin = [-16.0 * ppm; 3];
    let mut sim = Simulation::new(spec).unwrap();
    let puff = Impulse {
        shape: Shape::Sphere { center: [distance * ppm, 0.0, 0.0], radius: 1.5 * ppm },
        time: 0.0,
        density: 1.0,
        temperature: 0.0,
        velocity: [0.0; 3],
        expansion: 0.0,
    };
    sim.step(&Inputs { impulses: vec![puff], ..Inputs::default() }).unwrap();
    let measure = |sim: &Simulation| {
        let s = sim.state();
        let (mut mass, mut moment) = (0.0, 0.0);
        for (k, d) in s.density().iter().enumerate() {
            let x = (k % n) as f64 * h + 0.5 * h - 16.0 * ppm;
            mass += d;
            moment += d * x;
        }
        (mass, moment / mass / ppm)
    };
    let (m0, x0) = measure(&sim);
    let b = Blast::new([0.0; 3], DT, energy, RHO0, P0, GAMMA, ppm).unwrap();
    let (_, r1) = b.radii(DT, 1);
    sim.step(&Inputs { blasts: vec![b], ..Inputs::default() }).unwrap();
    // the step advects with the velocity that the last step left (the pulse), so the smoke moves in the step after it: one more, with no blast
    sim.step(&Inputs::default()).unwrap();
    let (m1, x1) = measure(&sim);
    (x0, x1, m0, m1, r1 / ppm)
}

#[test]
fn smoke_outside_the_sphere_is_displaced_by_the_volume_the_front_swept_and_the_smoke_is_the_smoke_to_the_advection() {
    // the air at r0 is pushed to r1 with r1^3 = r0^3 + R^3 (the volume that the sphere has swept goes between the two): the exact displacement
    let exact = |d: f64, radius: f64| (d.powi(3) + radius.powi(3)).cbrt() - d;
    let mut errors = Vec::new();
    for cell in [0.5, 0.25] {
        let (x0, x1, m0, m1, radius) = puff_and_pulse(ENERGY, 6.0, 1.0, cell);
        let want = exact(6.0, radius);
        // the displacement is the exact one to 5 percent at half a metre and to 3 at a quarter (0.984 and 0.990 of it, measured)
        let tolerance = if cell == 0.5 { 0.05 } else { 0.03 };
        assert!(
            (x1 - x0) / want > 1.0 - tolerance && (x1 - x0) / want < 1.0 + tolerance,
            "cell {cell}: the smoke moved {} m of {want} m",
            x1 - x0
        );
        errors.push((m1 / m0 - 1.0).abs());
    }
    // the semi-Lagrangian advection is not conservative: the smoke is not made or lost to a tolerance of 1e-6 but to the interpolation, which falls
    // with the cell (3.6 percent at half a metre, 0.70 at a quarter, measured)
    assert!(errors[1] < errors[0] && errors[1] < 0.015, "the smoke changed by {errors:?}");
    // the same shape in other units (a metre of 100 and of 37 scene units): the same displacement in metres and the same smoke
    let (x0, x1, m0, m1, _) = puff_and_pulse(ENERGY, 6.0, 1.0, 0.5);
    for ppm in [100.0, 37.0] {
        let (y0, y1, n0, n1, _) = puff_and_pulse(ENERGY, 6.0, ppm, 0.5);
        assert!((y1 - y0 - (x1 - x0)).abs() < 1e-9, "ppm {ppm}: {} against {}", y1 - y0, x1 - x0);
        assert!(((n1 / n0) - (m1 / m0)).abs() < 1e-9);
    }
}

#[test]
fn a_pulse_of_many_cells_in_one_step_still_displaces_the_smoke_by_the_volume_swept_to_the_trace_of_the_advection() {
    // 1e11 J: the front of one step is at about 7.5 m, and smoke at 9 m is pushed to about 10.5 m: 3 cells in the step at half a metre (a Courant
    // number of 3) and 6 at a quarter. What the semi-Lagrangian trace does with it, measured: it neither runs out of the domain nor crosses the
    // sphere, and the displacement is the exact one to 5 percent (a little under, 0.990 and 0.981 of it), the smoke changing by 1.8 and 1.4 percent
    let mut errors = Vec::new();
    for cell in [0.5, 0.25] {
        let (x0, x1, m0, m1, radius) = puff_and_pulse(1e11, 9.0, 1.0, cell);
        let want = (9.0f64.powi(3) + radius.powi(3)).cbrt() - 9.0;
        let cfl = want / cell;
        assert!(cfl > 2.9, "a Courant number of {cfl}");
        let ratio = (x1 - x0) / want;
        assert!((0.95..=1.03).contains(&ratio), "cell {cell}: the displacement is {ratio} of the exact one");
        errors.push((m1 / m0 - 1.0).abs());
    }
    assert!(errors[1] < errors[0] && errors[1] < 0.03, "the smoke changed by {errors:?}");
}

/// The kinetic energy in joules of the flow inside the ball of `a` metres about the centre, from the velocity of the faces.
fn kinetic_energy(sim: &Simulation, h: f64, ppm: f64, a: f64) -> f64 {
    let s = sim.state();
    let n = s.cells();
    let half = n[0] as f64 * h / 2.0;
    let faces = |axis: usize, p: [usize; 3]| {
        let dims: [usize; 3] = std::array::from_fn(|k| n[k] + usize::from(k == axis));
        s.velocity_faces(axis)[p[0] + dims[0] * (p[1] + dims[1] * p[2])]
    };
    let mut total = 0.0;
    for z in 0..n[2] {
        for y in 0..n[1] {
            for x in 0..n[0] {
                let c = [x, y, z].map(|i| (i as f64 + 0.5) * h - half);
                if (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt() > a * ppm {
                    continue;
                }
                let u = [
                    0.5 * (faces(0, [x, y, z]) + faces(0, [x + 1, y, z])),
                    0.5 * (faces(1, [x, y, z]) + faces(1, [x, y + 1, z])),
                    0.5 * (faces(2, [x, y, z]) + faces(2, [x, y, z + 1])),
                ];
                let speed2 = u.iter().map(|v| (v / ppm).powi(2)).sum::<f64>();
                total += 0.5 * RHO0 * speed2 * (h / ppm).powi(3);
            }
        }
    }
    total
}

#[test]
fn the_kinetic_energy_of_the_piston_is_the_flow_it_makes_inside_and_outside_the_sphere_and_converges() {
    // the flow of a pulse: inside the sphere u = Q r / (4 pi R^3), outside u = Q / (4 pi r^2), Q = (4 pi / 3) R^3 / dt; the energy in the ball
    // of radius a is rho Q^2 / (8 pi R) (1/5 + 1 - R / a) (the sphere's own 1/5 and the potential flow's 1 - R/a)
    let mut errors = Vec::new();
    for (n, h) in [(32usize, 0.5f64), (64, 0.25)] {
        let mut sim = Simulation::new(domain(n, h, Boundary::Open)).unwrap();
        let b = blast(1.0, [0.0; 3], 0.0);
        let (_, r) = b.radii(DT, 0);
        sim.step(&Inputs { blasts: vec![b], ..Inputs::default() }).unwrap();
        let q = 4.0 / 3.0 * std::f64::consts::PI * r.powi(3) / DT;
        let a = 1.5 * r;
        let want = RHO0 * q * q / (8.0 * std::f64::consts::PI * r) * (0.2 + 1.0 - r / a);
        let got = kinetic_energy(&sim, h, 1.0, a);
        errors.push((got / want - 1.0).abs());
        println!("BLAST KE h {h}: {got:.4e} J against {want:.4e} J");
    }
    assert!(errors[1] < errors[0], "{errors:?}");
    // 0.9 and 0.85 percent, measured
    assert!(errors[1] < 0.02, "{errors:?}");
    // and in other units (a metre of 37 scene units) the same energy in joules
    let ppm = 37.0;
    let mut spec = domain(64, 0.25 * ppm, Boundary::Open);
    spec.origin = [-8.0 * ppm; 3];
    let mut sim = Simulation::new(spec).unwrap();
    let b = blast(ppm, [0.0; 3], 0.0);
    let (_, r) = b.radii(DT, 0);
    sim.step(&Inputs { blasts: vec![b], ..Inputs::default() }).unwrap();
    let in_37 = kinetic_energy(&sim, 0.25 * ppm, ppm, 1.5 * r / ppm);
    let mut base = Simulation::new(domain(64, 0.25, Boundary::Open)).unwrap();
    let b = blast(1.0, [0.0; 3], 0.0);
    let (_, r1) = b.radii(DT, 0);
    base.step(&Inputs { blasts: vec![b], ..Inputs::default() }).unwrap();
    let in_1 = kinetic_energy(&base, 0.25, 1.0, 1.5 * r1);
    assert!((in_37 / in_1 - 1.0).abs() < 1e-6, "{in_37} against {in_1}");
}

fn covered_by(cells: usize, h: f64, center: [f64; 3], radius: f64) -> f64 {
    let half = cells as f64 * h / 2.0;
    let c = |i: usize| (i as f64 + 0.5) * h - half;
    let mut n = 0u64;
    for z in 0..cells {
        for y in 0..cells {
            for x in 0..cells {
                let d = [c(x) - center[0], c(y) - center[1], c(z) - center[2]];
                n += u64::from(d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= radius * radius);
            }
        }
    }
    n as f64
}

#[test]
fn an_energy_of_one_joule_displaces_the_volume_of_its_own_front_and_not_the_volume_of_a_cell() {
    // 1 J: the strong phase ends at 0.3 (1 / p0)^(1/3) = 6.4 mm, a sphere of 1.1e-6 cubic metres, in cells of half a metre: the cells that
    // the sphere is given to (a cell is the least that it covers) hold that volume and no more
    let h = 0.5;
    let mut sim = Simulation::new(domain(16, h, Boundary::Open)).unwrap();
    let b = Blast::new([0.0; 3], 0.0, 1.0, RHO0, P0, GAMMA, 1.0).unwrap();
    let (_, volume) = b.swept(DT, 0).unwrap();
    let r_max = 0.3 * (1.0 / P0).cbrt();
    assert!((volume - 4.0 / 3.0 * std::f64::consts::PI * r_max.powi(3)).abs() < 1e-9 * volume, "{volume}");
    sim.step(&Inputs { blasts: vec![b], ..Inputs::default() }).unwrap();
    let flux = outflow(&sim, h);
    // (to the tolerance of the projection, 1e-8 a second over the 512 cubic metres of the domain, of 2.2e-3 cubic metres a second)
    assert!((flux / (volume / DT) - 1.0).abs() < 5e-3, "the domain lets out {flux}, the front swept {}", volume / DT);
    // and not the 0.125 cubic metres of a cell over dt that the volume of one cell would have been
    assert!(flux < 0.01 * 0.125 / DT);
}

#[test]
fn a_window_that_the_sphere_cuts_is_pushed_from_the_blast_and_not_from_its_own_middle() {
    // a domain of 32 cells of half a metre (16 m) and a blast of 1e11 J at x = 7 m, 1 m from the face: its front (7.5 m in the first step) covers
    // most of the window, and its centre is not the window's
    let h = 0.5;
    let n = 32;
    let mut sim = Simulation::new(domain(n, h, Boundary::Open)).unwrap();
    let centre = [7.0, 0.0, 0.0];
    let b = Blast::new(centre, 0.0, 1e11, RHO0, P0, GAMMA, 1.0).unwrap();
    let (_, r1) = b.radii(DT, 0);
    assert!(r1 > 7.0, "{r1}");
    sim.step(&Inputs { blasts: vec![b.clone()], ..Inputs::default() }).unwrap();
    let swept = b.swept(DT, 0).unwrap().1;
    // the flow that the window lets out is the volume of the cells that the window holds of the sphere, and not the whole sphere's: the faces are open
    let covered = covered_by(n, h, centre, r1);
    let s = sim.state();
    let ux = |x: f64| {
        // the x velocity on the face at x (on the row nearest the axis)
        let face = ((x + 8.0) / h).round() as usize;
        s.velocity_faces(0)[face + (n + 1) * (n / 2 - 1 + n * (n / 2 - 1))]
    };
    // between the middle of the window (x = 0) and the blast (x = 7) the air goes AWAY from the blast, toward -x: a push from the middle of the
    // window would send it toward +x
    for x in [-2.0, 0.0, 2.0, 4.0] {
        assert!(ux(x) < 0.0, "u_x({x}) = {}", ux(x));
    }
    // inside the sphere the flow is linear in the distance from the blast: u(x) / u(2) = (x - 7) / (2 - 7)
    let ratio = ux(4.0) / ux(2.0);
    assert!((ratio - (4.0 - 7.0) / (2.0 - 7.0)).abs() < 0.1, "{ratio}");
    // and the rate at which the cells are given the divergence is the volume of the sphere over the cells of the window that it covers
    let rate = swept / DT / (covered * h.powi(3));
    let d = (ux(4.0) - ux(2.0)) / 2.0 * 3.0 / 1.0;
    assert!(d.abs() < 3.0 * rate, "{d} against {rate}");
}

#[test]
fn the_volume_swept_over_all_the_steps_of_the_strong_phase_is_the_volume_of_its_last_sphere() {
    let h = 0.5;
    let mut sim = Simulation::new(domain(64, h, Boundary::Open)).unwrap();
    let b = blast(1.0, [0.0; 3], 0.0);
    let r_max = solve(GAMMA).unwrap().max_radius(ENERGY, P0);
    let mut released = 0.0;
    let mut previous = 0.0;
    for step in 0..14u64 {
        sim.step(&Inputs { blasts: vec![b.clone()], ..Inputs::default() }).unwrap();
        let out = outflow(&sim, h) * DT;
        let want = b.swept(DT, step).map_or(0.0, |s| s.1);
        assert!(
            (out - want).abs() < 1e-6 * want + 1e-7,
            "step {step}: the domain let out {out} and the front swept {want}"
        );
        released += out;
        previous = b.radii(DT, step).1;
    }
    assert!((previous - r_max).abs() < 1e-9, "the front has stopped at the end of the strong phase");
    assert!((released / (4.0 / 3.0 * std::f64::consts::PI * r_max.powi(3)) - 1.0).abs() < 1e-6, "{released}");
}

#[test]
fn a_solid_in_the_sphere_takes_none_of_the_volume_and_a_source_in_the_same_step_adds_its_own() {
    use sr_sim::pyro::Obstacle;
    let h = 0.25;
    let n = 64;
    let b = blast(1.0, [0.0; 3], 0.0);
    let (_, swept) = b.swept(DT, 0).unwrap();
    // a stationary box in the middle of the sphere: its cells are solid, so the cells that are given the volume are the sphere's that are not
    let wall = Obstacle::stationary(Shape::Box { min: [-1.0, -1.0, -1.0], max: [1.0, 1.0, 1.0] });
    let mut sim = Simulation::new(domain(n, h, Boundary::Open)).unwrap();
    sim.step(&Inputs { blasts: vec![b.clone()], obstacles: vec![wall], ..Inputs::default() }).unwrap();
    let flux = outflow(&sim, h);
    assert!(
        (flux / (swept / DT) - 1.0).abs() < 1e-4,
        "with a solid: the domain lets out {flux}, the front swept {}",
        swept / DT
    );
    // a source with an expansion in the same step: the domain lets out the volume of the front and the expansion times the volume of the cells
    // that the source covers
    let source = sr_sim::pyro::Source {
        shape: Shape::Sphere { center: [6.0, 0.0, 0.0], radius: 1.0 },
        expansion: 3.0,
        ..sr_sim::pyro::Source::default()
    };
    let mut sim = Simulation::new(domain(n, h, Boundary::Open)).unwrap();
    sim.step(&Inputs { blasts: vec![b], sources: vec![source], ..Inputs::default() }).unwrap();
    let cells = covered_by(n, h, [6.0, 0.0, 0.0], 1.0);
    let want = swept / DT + 3.0 * cells * h.powi(3);
    let flux = outflow(&sim, h);
    assert!((flux / want - 1.0).abs() < 1e-4, "the domain lets out {flux}, the front and the source say {want}");
}

#[test]
fn a_window_that_follows_its_smoke_is_the_window_it_was_where_it_does_not_move() {
    use sr_sim::pyro::Follow;
    let run = |follow: Option<Follow>| {
        let spec = Spec { follow, ..domain(32, 0.5, Boundary::Open) };
        let mut sim = Simulation::new(spec).unwrap();
        let puff = Impulse {
            shape: Shape::Sphere { center: [4.0, 0.0, 0.0], radius: 1.0 },
            time: 0.0,
            density: 1.0,
            temperature: 0.0,
            velocity: [0.0; 3],
            expansion: 0.0,
        };
        for step in 0..4 {
            let inputs = Inputs {
                impulses: if step == 0 { vec![puff.clone()] } else { vec![] },
                blasts: vec![blast(1.0, [0.0; 3], DT)],
                ..Inputs::default()
            };
            sim.step(&inputs).unwrap();
        }
        (bits(&sim), sim.state().window())
    };
    let (plain, _) = run(None);
    let (followed, window) = run(Some(Follow { margin: 2, loss: 0.0 }));
    assert_eq!(window, [0, 0, 0], "the window did not move");
    assert_eq!(followed, plain);
}
