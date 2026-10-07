//! What the window of a following domain changes in a step besides where it is: the seeded turbulence belongs to a
//! place in space, so that moving the window does not change the noise at a point.

use super::*;

fn spec(follow: Option<Follow>) -> Spec {
    Spec {
        cells: [12, 12, 12],
        origin: [0.0; 3],
        voxel_size: 1.0,
        dt: 0.1,
        boundary: Boundary::Open,
        turbulence: 3.0,
        seed: 11,
        follow,
        ..Spec::default()
    }
}

/// The velocity that turbulence alone gives a state at rest in one step.
fn pushed(state: &State, spec: &Spec) -> State {
    let mut state = state.working_copy();
    forces(&mut state, spec, &Inputs::default(), 5);
    state
}

#[test]
fn the_turbulence_of_a_following_window_is_the_same_at_the_same_place_wherever_the_window_is() {
    let follow = Some(Follow { margin: 2, loss: 0.0 });
    let spec = spec(follow);
    let base = Simulation::new(spec.clone()).unwrap().state().clone();
    let by = [1i64, -2, 3];
    let moved = base.shifted(by, true).unwrap();
    let (a, b) = (pushed(&base, &spec), pushed(&moved, &spec));
    let n = [12i64; 3];
    let mut compared = 0;
    for axis in 0..3 {
        let dims = face_dims(spec.cells, axis).map(|d| d as i64);
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    // the face of the moved window at (x, y, z) is the face of the first at (x, y, z) + by, and both
                    // of the cells it is between are inside the first window and are not on its edge
                    let there = [x + by[0], y + by[1], z + by[2]];
                    let inside = |c: [i64; 3]| {
                        (1..n[0] - 1).contains(&c[0]) && (1..n[1] - 1).contains(&c[1]) && (1..n[2] - 1).contains(&c[2])
                    };
                    if !inside(there) || !inside([x, y, z]) {
                        continue;
                    }
                    let (kb, ka) = (
                        index([x as usize, y as usize, z as usize], face_dims(spec.cells, axis)),
                        index(there.map(|v| v as usize), face_dims(spec.cells, axis)),
                    );
                    assert_eq!(
                        b.velocity[axis][kb].to_bits(),
                        a.velocity[axis][ka].to_bits(),
                        "face {axis} at {there:?}"
                    );
                    compared += 1;
                }
            }
        }
    }
    assert!(compared > 200, "{compared} faces compared");
}

#[test]
fn without_a_follow_the_turbulence_is_keyed_by_the_cell_of_the_window_as_it_always_was() {
    let spec = spec(None);
    let base = Simulation::new(spec.clone()).unwrap().state().clone();
    let moved = base.shifted([1, 0, 0], true).unwrap();
    let (a, b) = (pushed(&base, &spec), pushed(&moved, &spec));
    // the cells did not change place in the window: the same noise, whatever the window says
    assert_eq!(a.velocity, b.velocity);
}

#[test]
fn a_following_window_that_has_not_moved_has_the_turbulence_that_it_has_without_following() {
    let follow = Some(Follow { margin: 2, loss: 0.0 });
    let (with, without) = (spec(follow), spec(None));
    let base = Simulation::new(without.clone()).unwrap().state().clone();
    let (a, b) = (pushed(&base, &without), pushed(&base, &with));
    assert_eq!(a.velocity, b.velocity, "the noise does not depend on follow while the window is where it began");
}
#[test]
fn a_cell_of_the_window_that_moved_out_of_where_it_began_has_a_noise_of_its_own() {
    let spec = spec(Some(Follow { margin: 2, loss: 0.0 }));
    // the cell one past the first window on the high side of x is not the cell one past it on the next row: no two cells share a key
    let mut keys = std::collections::HashSet::new();
    for z in -3i64..15 {
        for y in -3i64..15 {
            for x in -3i64..15 {
                assert!(
                    keys.insert(global_cell([0; 3], [x, y, z], spec.cells)),
                    "the cell {:?} shares a key",
                    [x, y, z]
                );
            }
        }
    }
}
