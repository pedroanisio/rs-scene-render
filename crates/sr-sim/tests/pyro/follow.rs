//! The window of a smoke that follows its plume: it moves by whole cells, what it keeps is bit for bit what it
//! had, what it takes in is the background an open face gives, and it never lets go of smoke.

use sr_sim::pyro::{Boundary, Inputs, Shape, Simulation, Source, Spec};

/// A 24-cell open domain of half-unit cells, with a column of hot smoke made in its middle.
fn smoking() -> Simulation {
    let spec = Spec {
        cells: [24, 24, 24],
        origin: [-6.0, -6.0, -6.0],
        voxel_size: 0.5,
        dt: 0.05,
        boundary: Boundary::Open,
        buoyancy: 2.0,
        pressure_iterations: 500,
        pressure_tolerance: 1e-8,
        ..Spec::default()
    };
    let mut sim = Simulation::new(spec).unwrap();
    let source = Source {
        shape: Shape::Sphere { center: [0.0, 0.0, 0.0], radius: 1.0 },
        density_rate: 4.0,
        temperature_rate: 400.0,
        ..Source::default()
    };
    for step in 0..6 {
        let input = Inputs { sources: if step < 3 { vec![source.clone()] } else { vec![] }, ..Inputs::default() };
        sim.step(&input).unwrap();
    }
    sim
}

fn at(cells: [usize; 3], p: [usize; 3]) -> usize {
    p[0] + cells[0] * (p[1] + cells[1] * p[2])
}

#[test]
fn a_shift_keeps_the_cells_it_keeps_to_the_bit_and_takes_in_the_background() {
    let mut sim = smoking();
    let before = sim.state().clone();
    assert_eq!(before.window(), [0, 0, 0]);
    let by = [1, -2, 3];
    sim.shift_window(by).unwrap();
    let after = sim.state();
    let n = before.cells();
    assert_eq!(after.window(), by);
    let ambient = Spec::default().ambient_temperature;
    let mut kept = 0;
    for z in 0..n[2] {
        for y in 0..n[1] {
            for x in 0..n[0] {
                let new = at(n, [x, y, z]);
                let old = [x as i64 + by[0], y as i64 + by[1], z as i64 + by[2]];
                if old.iter().zip(n).all(|(&o, n)| (0..n as i64).contains(&o)) {
                    let old = at(n, old.map(|o| o as usize));
                    assert_eq!(after.density()[new].to_bits(), before.density()[old].to_bits());
                    assert_eq!(after.temperature()[new].to_bits(), before.temperature()[old].to_bits());
                    kept += 1;
                } else {
                    assert_eq!(after.density()[new], 0.0, "a cell that came in has no smoke");
                    assert_eq!(after.temperature()[new], ambient, "and is at ambient temperature");
                }
            }
        }
    }
    assert!(kept > 0);
    // the faces too: the velocity of a face that is kept is the one it had, and a face that came in is at rest
    for axis in 0..3 {
        let dims: [usize; 3] = std::array::from_fn(|a| n[a] + usize::from(a == axis));
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    let new = after.velocity_faces(axis)[at(dims, [x, y, z])];
                    let old = [x as i64 + by[0], y as i64 + by[1], z as i64 + by[2]];
                    if old.iter().zip(dims).all(|(&o, d)| (0..d as i64).contains(&o)) {
                        let old = before.velocity_faces(axis)[at(dims, old.map(|o| o as usize))];
                        assert_eq!(new.to_bits(), old.to_bits());
                    } else {
                        assert_eq!(new, 0.0);
                    }
                }
            }
        }
    }
}

#[test]
fn a_shift_loses_no_smoke_and_moves_the_origin_by_whole_cells() {
    let mut sim = smoking();
    let before = sim.state().clone();
    let mut mass: Vec<u64> = before.density().iter().filter(|d| **d != 0.0).map(|d| d.to_bits()).collect();
    mass.sort_unstable();
    assert!(mass.len() > 10, "there is smoke to lose");
    // the smoke is in the middle, so the slab that leaves is empty: toward the top and back out again
    sim.shift_window([0, -3, 2]).unwrap();
    sim.shift_window([1, 1, 0]).unwrap();
    let after = sim.state();
    let mut kept: Vec<u64> = after.density().iter().filter(|d| **d != 0.0).map(|d| d.to_bits()).collect();
    kept.sort_unstable();
    assert_eq!(kept, mass, "every value of density is still there, none new and none lost");
    let (h, w) = (0.5, after.window());
    assert_eq!(w, [1, -2, 2]);
    for ((got, base), shift) in after.origin().into_iter().zip(before.origin()).zip(w) {
        assert_eq!(got.to_bits(), (base + shift as f64 * h).to_bits());
    }
}

#[test]
fn a_shift_that_would_let_go_of_smoke_is_refused_and_changes_nothing() {
    let mut sim = smoking();
    let before = sim.state().clone();
    // 12 cells toward the middle of the smoke: the slab that leaves holds it
    assert!(sim.shift_window([12, 0, 0]).is_err());
    assert!(sim.shift_window([0, 0, -12]).is_err());
    assert!(sim.shift_window([30, 0, 0]).is_err(), "a shift past the whole window");
    assert_eq!(sim.state(), &before);
}
