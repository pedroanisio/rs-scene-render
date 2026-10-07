//! The window of a smoke that follows its plume: it moves by whole cells, what it keeps is bit for bit what it
//! had, what it takes in is the background an open face gives, and it never lets go of smoke.

use sr_sim::pyro::{Boundary, Impulse, Inputs, Shape, Simulation, Source, Spec};

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

use sr_sim::pyro::{Follow, State, Timeline};

/// A column of cells along y (the axis smoke rises along, toward negative y) that is `cells_y` tall and whose
/// bottom face is at y = 8 whatever its height.
fn column(cells_y: usize, follow: Option<Follow>) -> Spec {
    Spec {
        cells: [16, cells_y, 16],
        origin: [-4.0, 8.0 - cells_y as f64 * 0.5, -4.0],
        voxel_size: 0.5,
        dt: 0.05,
        boundary: Boundary::Open,
        buoyancy: 6.0,
        pressure_iterations: 500,
        pressure_tolerance: 1e-8,
        follow,
        ..Spec::default()
    }
}

/// A blob of smoke made in the first step 1.5 units above the bottom face of the first window, in air that
/// everywhere accelerates upward (toward negative y) at 2 units a second squared, so that it rises as it is, a
/// passive scalar in a uniform flow, with no stem behind it.
fn blob(step: u64, _time: f64, _state: &State) -> Result<Inputs, sr_sim::pyro::Error> {
    let impulse = Impulse {
        shape: Shape::Sphere { center: [0.0, 6.5, 0.0], radius: 1.5 },
        time: 0.0,
        density: 1.0,
        temperature: 0.0,
        velocity: [0.0; 3],
        expansion: 0.0,
    };
    Ok(Inputs {
        impulses: if step == 0 { vec![impulse] } else { vec![] },
        acceleration: [0.0, -2.0, 0.0],
        ..Inputs::default()
    })
}

fn run(spec: Spec, steps: u64) -> State {
    let mut timeline = Timeline::new(spec, 64 << 20).unwrap();
    timeline.at_with_state(steps as f64 * 0.05, &mut |s, t, st| blob(s, t, st)).unwrap().clone()
}

/// The smoke-weighted height of a state's cells, in scene units, and the smoke.
fn centroid_y(st: &State) -> (f64, f64) {
    let n = st.cells();
    let (mut sum, mut mass) = (0.0, 0.0);
    for y in 0..n[1] {
        let row: f64 =
            (0..n[2]).flat_map(|z| (0..n[0]).map(move |x| (x, z))).map(|(x, z)| st.density()[at(n, [x, y, z])]).sum();
        sum += row * (st.origin()[1] + (y as f64 + 0.5) * 0.5);
        mass += row;
    }
    (sum / mass, mass)
}

fn mass(st: &State) -> f64 {
    st.density().iter().sum()
}

#[test]
fn a_blob_that_rises_far_past_its_window_is_kept_when_the_window_may_let_go_of_its_thin_tail() {
    let steps = 140;
    let tall = run(column(640, None), steps);
    let (tall_y, tall_mass) = centroid_y(&tall);
    // a window of the same height that stays where it is cuts the blob off at its top
    let fixed = run(column(80, None), steps);
    assert!(mass(&fixed) < 0.2 * tall_mass, "the fixed window kept {} of {tall_mass}", mass(&fixed));
    let loss = 1e-6;
    let following = run(column(80, Some(Follow { margin: 4, loss })), steps);
    let (y, kept) = centroid_y(&following);
    assert!((kept - tall_mass).abs() < 0.1 * tall_mass, "kept {kept} of {tall_mass}");
    assert!((y - tall_y).abs() < 2.0, "the blob is at {y} and not at {tall_y}");
    // it moved a long way, four times its own height, and let go of a very small part of the smoke
    assert!(-following.window()[1] >= 20, "window {:?}", following.window());
    assert!(following.lost() > 0.0 && following.lost() < 1e-4 * tall_mass, "lost {}", following.lost());
}

#[test]
fn a_loss_of_zero_lets_go_only_of_slabs_with_no_smoke_and_so_does_not_follow_a_blob_with_a_tail() {
    let steps = 140;
    let tall_mass = mass(&run(column(640, None), steps));
    let following = run(column(80, Some(Follow { margin: 4, loss: 0.0 })), steps);
    assert_eq!(following.lost(), 0.0, "no smoke is let go of");
    // the numerical tail of the blob is never exactly zero, so the window cannot leave its rear and the blob goes
    // out of the top face as it does with no follow
    assert!(mass(&following) < 0.1 * tall_mass, "{} of {tall_mass}", mass(&following));
}

#[test]
fn what_a_move_lets_go_of_is_counted_and_is_never_more_than_the_share_asked() {
    let loss = 1e-3;
    let spec = column(80, Some(Follow { margin: 4, loss }));
    let mut sim = Simulation::new(spec).unwrap();
    let mut moves = 0;
    for step in 0..140u64 {
        let mut input = blob(step, step as f64 * 0.05, sim.state()).unwrap();
        let (before, lost_before) = (mass(sim.state()), sim.state().lost());
        let by = sim.follow(&input).unwrap();
        if by != [0; 3] {
            moves += 1;
            input = blob(step, step as f64 * 0.05, sim.state()).unwrap();
            let let_go = sim.state().lost() - lost_before;
            let after = mass(sim.state());
            assert!(
                (after + let_go - before).abs() <= 1e-12 * before,
                "step {step}: {after} + {let_go} against {before}"
            );
            assert!(let_go <= 1.5 * loss * before, "step {step}: let go of {let_go} of {before}");
        }
        sim.step(&input).unwrap();
    }
    assert!(moves >= 10, "the window moved {moves} times");
}

#[test]
fn a_window_never_leaves_the_slab_of_a_source_that_is_acting_whatever_the_loss() {
    // a source that makes smoke all the time at the bottom of the window, and a loss that would let go of almost
    // everything: the window rises with the smoke as far as the source allows, and no farther
    let spec = column(48, Some(Follow { margin: 4, loss: 0.5 }));
    let mut sim = Simulation::new(spec).unwrap();
    let source = Source {
        shape: Shape::Sphere { center: [0.0, 6.5, 0.0], radius: 1.2 },
        density_rate: 8.0,
        temperature_rate: 100.0,
        ..Source::default()
    };
    let mut rose = false;
    for _ in 0..100 {
        let input = Inputs { sources: vec![source.clone()], ..Inputs::default() };
        sim.follow(&input).unwrap();
        let state = sim.state();
        let bottom = state.origin()[1] + state.cells()[1] as f64 * 0.5;
        assert!(bottom > 6.5 + 1.2 - 0.5, "the window left the source behind: its bottom is at {bottom}");
        rose |= state.window()[1] != 0;
        sim.step(&input).unwrap();
    }
    assert!(rose, "the window did not move at all");
}

#[test]
fn a_following_window_that_the_smoke_never_nears_the_faces_of_is_bit_for_bit_not_following() {
    let make = |follow| Spec {
        cells: [24, 24, 24],
        origin: [-6.0, -6.0, -6.0],
        voxel_size: 0.5,
        dt: 0.05,
        boundary: Boundary::Open,
        buoyancy: 1.0,
        pressure_iterations: 500,
        pressure_tolerance: 1e-8,
        follow,
        ..Spec::default()
    };
    let input = |step: u64, _: f64, _: &State| {
        let source = Source {
            shape: Shape::Sphere { center: [0.0, 0.0, 0.0], radius: 1.0 },
            density_rate: 2.0,
            temperature_rate: 50.0,
            ..Source::default()
        };
        Ok(Inputs { sources: if step < 4 { vec![source] } else { vec![] }, ..Inputs::default() })
    };
    let run = |follow: Option<Follow>| {
        let mut timeline = Timeline::new(make(follow), 64 << 20).unwrap();
        timeline.at_with_state(1.0, &mut |s, t, st| input(s, t, st)).unwrap().clone()
    };
    let (following, plain) = (run(Some(Follow { margin: 3, loss: 1e-6 })), run(None));
    assert_eq!(following.window(), [0, 0, 0], "the window did not move");
    assert_eq!(following, plain);
}

#[test]
fn any_order_of_times_a_fresh_run_and_a_tight_budget_give_the_same_window_and_cells() {
    let spec = column(80, Some(Follow { margin: 4, loss: 1e-6 }));
    let one = Simulation::new(spec.clone()).unwrap().state().bytes();
    let mut timeline = Timeline::new(spec.clone(), one * 2).unwrap();
    let times = [6.5, 1.0, 5.0, 0.0, 6.5, 3.0];
    let mut moved = false;
    for &time in &times {
        let got = timeline.at_with_state(time, &mut |s, t, st| blob(s, t, st)).unwrap().clone();
        let mut fresh = Simulation::new(spec.clone()).unwrap();
        for step in 0..(time / 0.05_f64).round() as u64 {
            let mut input = blob(step, step as f64 * 0.05, fresh.state()).unwrap();
            if fresh.follow(&input).unwrap() != [0; 3] {
                input = blob(step, step as f64 * 0.05, fresh.state()).unwrap();
            }
            fresh.step(&input).unwrap();
        }
        assert_eq!(&got, fresh.state(), "time {time}");
        moved |= got.window() != [0, 0, 0];
    }
    assert!(moved, "the window moved in this run");
}

#[test]
fn a_window_with_smoke_going_toward_both_faces_says_so_and_stays() {
    // smoke fills the column from the bottom to the top and the air in it expands: it is going toward both faces
    // and cannot be moved from either; smoke at rest has no way to go and asks for room on both, and cannot be
    // given it either
    let spec = || Spec { buoyancy: 0.0, ..column(16, Some(Follow { margin: 3, loss: 0.0 })) };
    let full = |expansion: f64| Source {
        shape: Shape::Box { min: [-4.0, 0.0, -4.0], max: [4.0, 8.0, 4.0] },
        density_rate: 1.0,
        expansion,
        ..Source::default()
    };
    let mut resting = Simulation::new(spec()).unwrap();
    resting.step(&Inputs { sources: vec![full(0.0)], ..Inputs::default() }).unwrap();
    assert_eq!(resting.state().follow_decision(3, 0.0, 0.05, &[]).0, [0, 0, 0]);
    assert!(resting.state().follow_decision(3, 0.0, 0.05, &[]).1[1]);
    let mut sim = Simulation::new(spec()).unwrap();
    sim.step(&Inputs { sources: vec![full(1.0)], ..Inputs::default() }).unwrap();
    let before = sim.state().clone();
    let (shift, blocked) = before.follow_decision(3, 0.0, 0.05, &[]);
    assert_eq!(shift, [0, 0, 0]);
    assert!(blocked[1], "smoke going toward both faces along y: blocked {blocked:?}");
    assert_eq!(sim.follow(&Inputs::default()).unwrap(), [0, 0, 0]);
    assert_eq!(sim.state(), &before);
}

#[test]
fn a_follow_with_a_closed_domain_a_margin_that_leaves_no_middle_or_a_loss_out_of_range_is_refused() {
    let mut spec = column(32, Some(Follow { margin: 4, loss: 0.0 }));
    spec.boundary = Boundary::Closed;
    assert!(Simulation::new(spec).is_err());
    assert!(Simulation::new(column(32, Some(Follow { margin: 0, loss: 0.0 }))).is_err());
    // 16 cells across: a margin of 8 leaves no cell between the two faces
    assert!(Simulation::new(column(32, Some(Follow { margin: 8, loss: 0.0 }))).is_err());
    assert!(Simulation::new(column(32, Some(Follow { margin: 7, loss: 0.0 }))).is_ok());
    for loss in [-0.1, 1.5, f64::NAN] {
        assert!(Simulation::new(column(32, Some(Follow { margin: 4, loss }))).is_err(), "loss {loss}");
    }
}

#[test]
fn a_window_does_not_move_against_the_drift_of_its_smoke_to_make_room_behind_it() {
    // the blob is made near the bottom face and rises: the tail it leaves near that face is no reason to move the
    // window down, which would take the room that the blob rises into
    let mut sim = Simulation::new(column(80, Some(Follow { margin: 4, loss: 1e-6 }))).unwrap();
    for step in 0..140u64 {
        let mut input = blob(step, step as f64 * 0.05, sim.state()).unwrap();
        if sim.follow(&input).unwrap() != [0; 3] {
            input = blob(step, step as f64 * 0.05, sim.state()).unwrap();
        }
        assert!(sim.state().window()[1] <= 0, "step {step}: the window is at {:?}", sim.state().window());
        sim.step(&input).unwrap();
    }
}
