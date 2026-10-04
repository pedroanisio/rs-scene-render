//! The gas of a smoke step as particles see it: a copy of its velocity, sampled in the volume's own
//! axes like the solver does, and at rest outside it, with no jump at the boundary.

use sr_sim::pyro::{Boundary, Impulse, Inputs, Shape, Simulation, Spec};

fn spec(boundary: Boundary) -> Spec {
    Spec {
        cells: [16, 12, 16],
        origin: [-8.0, -6.0, -8.0],
        voxel_size: 1.0,
        dt: 0.1,
        boundary,
        pressure_iterations: 400,
        pressure_tolerance: 1e-9,
        ..Spec::default()
    }
}

/// A smoke that has been given a velocity of 5 along x and a swirl, and run for a few steps.
fn blown(boundary: Boundary) -> Simulation {
    let mut sim = Simulation::new(spec(boundary)).unwrap();
    let whole = Shape::Box { min: [-8.0, -6.0, -8.0], max: [8.0, 6.0, 8.0] };
    let push =
        Impulse { shape: whole, time: 0.0, density: 0.0, temperature: 0.0, velocity: [5.0, 0.0, 0.0], expansion: 0.0 };
    let swirl = Impulse {
        shape: Shape::Sphere { center: [0.0, 0.0, 0.0], radius: 3.0 },
        time: 0.0,
        density: 1.0,
        temperature: 600.0,
        velocity: [0.0, 4.0, 2.0],
        expansion: 0.0,
    };
    for _ in 0..4 {
        sim.step(&Inputs { impulses: vec![push.clone(), swirl.clone()], ..Inputs::default() }).unwrap();
    }
    sim
}

#[test]
fn inside_the_volume_the_gas_is_the_states_own_velocity_to_the_last_bit() {
    for boundary in [Boundary::Open, Boundary::Closed] {
        let sim = blown(boundary);
        let gas = sim.state().gas();
        for p in [[0.3, 0.2, -0.7], [-7.9, 5.5, 7.9], [4.123, -3.3, 1.9], [0.0, 0.0, 0.0], [7.99, 5.99, -7.99]] {
            let (a, b) = (gas.velocity_at(p), sim.state().velocity_at(p));
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits), "{boundary:?} at {p:?}");
        }
        assert!(gas.bytes() >= 3 * 16 * 12 * 16 * 8, "{}", gas.bytes());
    }
}

#[test]
fn outside_it_is_at_rest_and_the_gas_goes_to_rest_across_the_boundary_without_a_jump() {
    for boundary in [Boundary::Open, Boundary::Closed] {
        let sim = blown(boundary);
        let gas = sim.state().gas();
        let speed = |p: [f64; 3]| gas.velocity_at(p).iter().map(|c| c * c).sum::<f64>().sqrt();
        // along x through the +x face at y = 1, z = 1, in steps of a thousandth of a cell
        let line: Vec<f64> = (0..4000).map(|k| speed([6.0 + k as f64 * 0.001, 1.0, 1.0])).collect();
        let biggest = line.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f64::max);
        let edge = speed([7.999, 1.0, 1.0]);
        assert!(edge > 0.05, "{boundary:?}: there is gas at the face: {edge}");
        // the gas falls to nothing over one cell outside, linearly, so a thousandth of a cell moves it
        // by about a thousandth of what is at the face
        assert!(biggest < 0.005 * edge.max(5.0), "{boundary:?}: a jump of {biggest} in a thousandth of a cell");
        assert_eq!(speed([9.0, 1.0, 1.0]), 0.0, "{boundary:?}: at rest beyond one cell");
        assert_eq!(speed([-20.0, 30.0, 4.0]), 0.0);
        assert_eq!(gas.velocity_at([f64::NAN, 0.0, 0.0]), [0.0; 3]);
    }
}

#[test]
fn a_copy_does_not_follow_the_simulation() {
    let mut sim = blown(Boundary::Open);
    let gas = sim.state().gas();
    let before = gas.velocity_at([1.0, 1.0, 1.0]);
    let swirl = Impulse {
        shape: Shape::Sphere { center: [0.0, 0.0, 0.0], radius: 3.0 },
        time: 0.5,
        density: 1.0,
        temperature: 900.0,
        velocity: [10.0, 10.0, 10.0],
        expansion: 0.0,
    };
    for _ in 0..3 {
        sim.step(&Inputs { impulses: vec![swirl.clone()], ..Inputs::default() }).unwrap();
    }
    assert_eq!(gas.velocity_at([1.0, 1.0, 1.0]), before);
    assert_ne!(sim.state().velocity_at([1.0, 1.0, 1.0]), before);
}
