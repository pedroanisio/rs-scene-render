//! What the rest of a rigid body puts on a part of it, from its motion in a step and the loads on it. The oracles are the cases with a closed form (the centripetal
//! load of a spinning body, a beam at rest with a weight on its tip, a body that falls) and the properties that must hold for any motion: what the rest puts
//! on a part and what the part puts on the rest are equal and opposite, and what is not a load on the body's points (damping) is no stress.
#![allow(clippy::needless_range_loop)]

use sr_sim::stress::balance::{momentum, spin_momentum, Located, MassSum, Rigid, Step};
use sr_sim::stress::{cut_stresses, JointSection};

const G: f64 = 9.80665;

/// A cube of `edge` metres and the density 1000 kg/m^3 whose centre is at `c`, as a mass sum.
fn cube(c: [f64; 3], edge: f64) -> MassSum {
    let m = 1000.0 * edge.powi(3);
    let mut second = [[0.0; 3]; 3];
    for a in 0..3 {
        for b in 0..3 {
            second[a][b] = m * (c[a] * c[b] + if a == b { edge * edge / 12.0 } else { 0.0 });
        }
    }
    MassSum { mass: m, first: c.map(|v| v * m), second }
}

fn sum(parts: &[MassSum]) -> MassSum {
    let mut total = MassSum::default();
    for p in parts {
        total.add(p);
    }
    total
}

const EYE: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn at_rest(whole: &MassSum, position: [f64; 3]) -> Rigid {
    Rigid { position, rotation: EYE, linear: [0.0; 3], angular: [0.0; 3], centre: whole.centre() }
}

/// A beam of `n` cubes of 0.5 m along +x with its first cube's centre at x = 0.25, as the pieces 0 to n - 1.
fn beam(n: usize) -> Vec<MassSum> {
    (0..n).map(|i| cube([0.25 + 0.5 * i as f64, 0.0, 0.0], 0.5)).collect()
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * a.abs().max(b.abs()).max(1.0)
}

#[test]
fn a_body_that_falls_freely_puts_no_load_on_any_part_of_itself() {
    let parts = beam(4);
    let whole = sum(&parts);
    let dt = 1.0 / 240.0;
    let before = Rigid { linear: [3.0, -2.0, 1.0], ..at_rest(&whole, [1.0, 20.0, -4.0]) };
    // a step of free fall: the velocity gains g dt and the body moves with the mean of the two
    let mut after = before;
    after.linear[1] -= G * dt;
    after.position = std::array::from_fn(|a| before.position[a] + dt * (before.linear[a] + after.linear[a]) / 2.0);
    let step = Step { whole, before, after, dt, accel: [0.0, -G, 0.0], contacts: &[], anchor: None };
    for cut in 1..4 {
        let part = sum(&parts[cut..]);
        let w = step.on_part(&part, &|i| i >= cut, [0.5 * cut as f64, 0.0, 0.0]);
        for a in 0..3 {
            assert!(w.force[a].abs() < 1e-8 * whole.mass && w.moment[a].abs() < 1e-8 * whole.mass, "cut {cut}: {w:?}");
        }
    }
}

#[test]
fn a_spinning_body_pulls_each_part_toward_its_axis_by_the_centripetal_force() {
    // two cubes 2 m apart spinning about the vertical axis through their middle at 10 rad/s: each is held by the other with m w^2 r toward the axis
    let parts = vec![cube([-1.0, 0.0, 0.0], 0.5), cube([1.0, 0.0, 0.0], 0.5)];
    let whole = sum(&parts);
    let (w, dt) = (10.0, 1e-4);
    let turn = |angle: f64| [[angle.cos(), 0.0, angle.sin()], [0.0, 1.0, 0.0], [-angle.sin(), 0.0, angle.cos()]];
    let before = Rigid { rotation: turn(0.0), angular: [0.0, w, 0.0], ..at_rest(&whole, [0.0; 3]) };
    let after = Rigid { rotation: turn(w * dt), ..before };
    let step = Step { whole, before, after, dt, accel: [0.0; 3], contacts: &[], anchor: None };
    let right = &parts[1];
    let load = step.on_part(right, &|i| i == 1, [0.0; 3]);
    let want = right.mass * w * w * 1.0;
    // along -x (toward the axis), to the first order of the angle turned in the step (the load is that of the middle of the step)
    assert!(close(load.force[0].abs(), want, 2e-3), "{:?} against {want}", load.force);
    assert!(load.force[0] < 0.0 && load.force[1].abs() < 1e-6 * want, "{:?}", load.force);
}

#[test]
fn a_beam_held_at_one_end_with_a_weight_on_the_other_has_the_bending_stress_of_the_beam() {
    // four cubes of 0.5 m in a row, the first held (a joint at its centre), the tip carrying a weight of 300 N: the joint between the pieces 0 and 1 is at x = 0.5
    let parts = beam(4);
    let whole = sum(&parts);
    let dt = 1.0 / 240.0;
    let rest = at_rest(&whole, [0.0; 3]);
    let tip = Located { at: [2.0, 0.0, 0.0], impulse: [0.0, -300.0 * dt, 0.0], piece: 3 };
    let step = Step { whole, before: rest, after: rest, dt, accel: [0.0, -G, 0.0], contacts: &[tip], anchor: Some(0) };
    // the side that is the tip's (pieces 1 to 3), the rest of the beam being the root: the cut at x = 0.5
    let cut = 0.5;
    let part = sum(&parts[1..]);
    let load = step.on_part(&part, &|i| i >= 1, [cut, 0.0, 0.0]);
    // the rest holds the tip's side up by its weight and the load, and its moment about the cut is the weight at the middle of the side and the load at the tip
    let weight = part.mass * G;
    let lever = part.centre()[0] - cut;
    assert!(close(load.force[1], weight + 300.0, 1e-9), "{:?}", load.force);
    assert!(close(load.moment[2], weight * lever + 300.0 * (2.0 - cut), 1e-9), "{:?}", load.moment);
    // and that is the bending stress of the beam with the exact modulus of the section (0.5 m square, joint 1 between the pieces 0 and 1)
    let b = 0.5;
    let joint = JointSection {
        area: b * b,
        centroid: [cut, 0.0, 0.0],
        second: [[0.0; 3], [0.0, b.powi(4) / 12.0, 0.0], [0.0, 0.0, b.powi(4) / 12.0]],
        normal: [1.0, 0.0, 0.0],
        lo: [cut, -b / 2.0, -b / 2.0],
        hi: [cut, b / 2.0, b / 2.0],
    };
    let s = cut_stresses(&[&joint], &[false], &load)[0];
    let modulus = b.powi(3) / 6.0;
    let want = (weight * lever + 300.0 * (2.0 - cut)) / modulus;
    assert!(close(s.bending, want, 1e-9), "{} against {want}", s.bending);
    assert!(close(s.shear, (weight + 300.0) / (b * b), 1e-9));
    // the whole body is held: the joint's load is the weight of all and the load, and the part that is the whole has no load on it
    let all = step.on_part(&whole, &|_| true, [0.0; 3]);
    assert!(all.force.iter().chain(&all.moment).all(|v| v.abs() < 1e-9 * whole.mass), "{all:?}");
}

#[test]
fn what_the_rest_puts_on_a_part_and_what_the_part_puts_on_the_rest_are_equal_and_opposite_whatever_the_motion() {
    let parts = beam(5);
    let whole = sum(&parts);
    let dt = 1.0 / 120.0;
    let turn = |angle: f64| -> [[f64; 3]; 3] {
        let (c, s) = (angle.cos(), angle.sin());
        // a turn about a slanted axis (1, 2, 2) / 3 by `angle`
        let k = [1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0];
        std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                let kk = k[i] * k[j] * (1.0 - c);
                let skew = [[0.0, -k[2], k[1]], [k[2], 0.0, -k[0]], [-k[1], k[0], 0.0]][i][j] * s;
                kk + skew + if i == j { c } else { 0.0 }
            })
        })
    };
    let before = Rigid {
        rotation: turn(0.4),
        linear: [2.0, -1.0, 0.5],
        angular: [1.5, -0.5, 2.0],
        ..at_rest(&whole, [3.0, 1.0, -2.0])
    };
    let after = Rigid {
        rotation: turn(0.45),
        position: [3.02, 0.99, -1.98],
        linear: [2.3, -1.4, 0.6],
        angular: [1.1, -0.9, 2.2],
        centre: before.centre,
    };
    let contacts = [
        Located { at: after.world([0.1, 0.25, 0.0]), impulse: [4.0, 9.0, -3.0], piece: 0 },
        Located { at: after.world([1.9, -0.25, 0.1]), impulse: [-2.0, 1.0, 7.0], piece: 3 },
    ];
    let q = [3.0, 1.2, -1.5];
    for anchor in [None, Some(0usize)] {
        let step = Step { whole, before, after, dt, accel: [0.0, -G, 0.0], contacts: &contacts, anchor };
        for cut in 1..5 {
            let (low, high) = (sum(&parts[..cut]), sum(&parts[cut..]));
            let a = step.on_part(&low, &|i| i < cut, q);
            let b = step.on_part(&high, &|i| i >= cut, q);
            for k in 0..3 {
                assert!(
                    close(a.force[k] + b.force[k], 0.0, 1e-9) && a.force[k].abs() + b.force[k].abs() >= 0.0,
                    "cut {cut}: {a:?} {b:?}"
                );
                assert!(close(a.moment[k] + b.moment[k], 0.0, 1e-9), "cut {cut} moment: {a:?} {b:?}");
            }
        }
    }
}

#[test]
fn damping_is_a_rigid_acceleration_that_stresses_nothing_and_the_centripetal_load_stays() {
    // four cubes spinning about the vertical axis through their centre of mass at 3 rad/s, which damping slows by a hundredth in the step, and moving
    // at 10 m/s, which it slows too: the centripetal load of the spin is a load of the pieces on each other, and the slowing is not
    let parts = beam(4);
    let whole = sum(&parts);
    let dt = 1.0 / 240.0;
    let com_world = [5.0, 0.0, 0.0];
    let rotation = |angle: f64| [[angle.cos(), -angle.sin(), 0.0], [angle.sin(), angle.cos(), 0.0], [0.0, 0.0, 1.0]];
    let frame = |angle: f64, omega: f64, speed: f64| {
        let r = rotation(angle);
        let c = whole.centre();
        let rc = [r[0][0] * c[0], r[1][0] * c[0], 0.0];
        Rigid {
            position: [com_world[0] - rc[0], com_world[1] - rc[1], 0.0],
            rotation: r,
            linear: [speed, 0.0, 0.0],
            angular: [0.0, 0.0, omega],
            centre: c,
        }
    };
    let (w0, w1) = (3.0, 3.0 * 0.99);
    let mean = (w0 + w1) / 2.0;
    let before = frame(0.2, w0, 10.0);
    let mut after = frame(0.2 + mean * dt, w1, 9.9);
    after.position[0] += 0.5 * (10.0 + 9.9) * dt;
    let step = Step { whole, before, after, dt, accel: [0.0; 3], contacts: &[], anchor: None };
    let part = sum(&parts[2..]);
    let load = step.on_part(&part, &|i| i >= 2, after.world(whole.centre()));
    // the centre of the part is 0.5 m from the centre of mass along the body's x axis; the impulse of a step of a turn is along the inward direction of
    // the middle of it
    let angle = 0.2 + mean * dt / 2.0;
    let (radial, tangent) = ([angle.cos(), angle.sin(), 0.0], [-angle.sin(), angle.cos(), 0.0]);
    let along = |v: [f64; 3], d: [f64; 3]| v[0] * d[0] + v[1] * d[1] + v[2] * d[2];
    let centripetal = part.mass * mean * mean * 0.5;
    // (the slowing is taken out along the direction of the end of the step, and the turn of the direction in the step leaves of it the part of a hundredth of the
    // slowing's own size, which is the half per cent of difference here)
    assert!(close(-along(load.force, radial), centripetal, 1e-2), "{:?} against {centripetal} inward", load.force);
    // the slowing of the spin would be a tangential force of m alpha r = m 0.03 / dt 0.5 on the part if it were a load of the pieces on each other: it is not
    let tangential_if_load = part.mass * (w0 - w1) / dt * 0.5;
    assert!(
        along(load.force, tangent).abs() < 1e-3 * tangential_if_load,
        "{:?} against {tangential_if_load}",
        load.force
    );
    // and the slowing of the translation is no load either
    assert!(load.force[2].abs() < 1e-9 * whole.mass);
}

#[test]
fn the_load_on_a_part_is_the_same_in_a_frame_that_moves_at_three_hundred_metres_a_second() {
    // the same step seen by an observer who sees the body and its contacts carried 300 m/s along: the balance is about the centre of mass and in the motion
    // relative to it, so a step of 1/60 s that moves the body five metres is the same step
    let parts = beam(5);
    let whole = sum(&parts);
    let dt = 1.0 / 60.0;
    let turn = |angle: f64| [[angle.cos(), -angle.sin(), 0.0], [angle.sin(), angle.cos(), 0.0], [0.0, 0.0, 1.0]];
    let before =
        Rigid { rotation: turn(0.1), linear: [1.0, 2.0, 0.0], angular: [0.0, 0.0, 2.0], ..at_rest(&whole, [0.0; 3]) };
    let after = Rigid {
        rotation: turn(0.14),
        position: [0.02, 0.03, 0.0],
        linear: [1.5, 1.2, 0.0],
        angular: [0.0, 0.0, 2.4],
        centre: before.centre,
    };
    let make = |boost: [f64; 3]| {
        let shift = boost.map(|v| v * dt);
        let b = Rigid {
            linear: [before.linear[0] + boost[0], before.linear[1] + boost[1], before.linear[2] + boost[2]],
            ..before
        };
        let a = Rigid {
            linear: [after.linear[0] + boost[0], after.linear[1] + boost[1], after.linear[2] + boost[2]],
            position: [after.position[0] + shift[0], after.position[1] + shift[1], after.position[2] + shift[2]],
            ..after
        };
        let contacts = [
            Located { at: a.world([0.1, 0.25, 0.0]), impulse: [4.0, 9.0, -3.0], piece: 0 },
            Located { at: a.world([2.4, -0.25, 0.0]), impulse: [-2.0, 1.0, 7.0], piece: 4 },
        ];
        (b, a, contacts, a.world([1.0, 0.0, 0.0]))
    };
    let result = |boost: [f64; 3]| {
        let (b, a, contacts, q) = make(boost);
        let step = Step { whole, before: b, after: a, dt, accel: [0.0, -G, 0.0], contacts: &contacts, anchor: None };
        let part = sum(&parts[2..]);
        step.on_part(&part, &|i| i >= 2, q)
    };
    let (slow, fast) = (result([0.0; 3]), result([300.0, -120.0, 40.0]));
    for k in 0..3 {
        assert!(close(slow.force[k], fast.force[k], 1e-9), "force {k}: {:?} {:?}", slow.force, fast.force);
        assert!(close(slow.moment[k], fast.moment[k], 1e-9), "moment {k}: {:?} {:?}", slow.moment, fast.moment);
    }
}

#[test]
fn the_momentum_of_a_body_is_its_mass_by_the_velocity_of_its_centre_and_its_spin_is_the_inertia_by_the_angular_velocity(
) {
    // a cube of 0.5 m and 125 kg spinning at 4 rad/s about its own axis while it moves at 7 m/s: m v, and (m a^2 / 6) w
    let one = cube([2.0, 0.0, 0.0], 0.5);
    let s = Rigid { linear: [7.0, 0.0, 0.0], angular: [0.0, 0.0, 4.0], ..at_rest(&one, [0.0; 3]) };
    let p = momentum(&one, &s);
    assert!(close(p[0], 125.0 * 7.0, 1e-12) && p[1].abs() < 1e-9 && p[2].abs() < 1e-9, "{p:?}");
    let l = spin_momentum(&one, &s);
    assert!(close(l[2], 125.0 * 0.25 / 6.0 * 4.0, 1e-12) && l[0].abs() < 1e-9 && l[1].abs() < 1e-9, "{l:?}");
    // a part that is off the centre of mass of the body has the momentum of a point that goes round it
    let parts = [cube([0.0, 0.0, 0.0], 0.5), cube([2.0, 0.0, 0.0], 0.5)];
    let whole = sum(&parts);
    let turning = Rigid { angular: [0.0, 0.0, 3.0], ..at_rest(&whole, [0.0; 3]) };
    let far = momentum(&parts[1], &turning);
    // the centre of mass is at x = 1: the part is 1 m from it along x, so it goes at 3 m/s along y
    assert!(far[0].abs() < 1e-9 && close(far[1], 125.0 * 3.0, 1e-12), "{far:?}");
}

#[test]
fn a_spinning_beam_pulled_apart_along_its_axis_has_no_bending_whatever_it_turns_in_a_step() {
    // four cubes of 0.5 m along x spinning at 12 rad/s about their centre of mass (x = 1, held at the origin of the world), in a step of 1/60 s (0.2 rad), with two contacts on
    // its axis, at its two ends, pulling it apart with 5000 N along the axis (the direction that the axis has in the middle of the step). The part of two cubes beyond the middle is held
    // by the pull and the centripetal m w^2 r = 250 * 144 * 0.5 = 18000 N, all along the axis: no shear and no moment. The arms of the contacts are those of the body at the
    // middle of the step, like the force: a load whose arm is of the end of the step while the force is of the middle makes a moment of half the turn times the arm times the force
    let parts = beam(4);
    let whole = sum(&parts);
    let (omega, dt) = (12.0f64, 1.0 / 60.0);
    let turn = |angle: f64| [[angle.cos(), -angle.sin(), 0.0], [angle.sin(), angle.cos(), 0.0], [0.0, 0.0, 1.0]];
    let at = |angle: f64| {
        let r = turn(angle);
        let c = whole.centre();
        // the centre of mass stays at the origin of the world
        Rigid {
            position: [-r[0][0] * c[0], -r[1][0] * c[0], 0.0],
            rotation: r,
            linear: [0.0; 3],
            angular: [0.0, 0.0, omega],
            centre: c,
        }
    };
    let (before, after) = (at(0.0), at(omega * dt));
    let direction = [(omega * dt / 2.0).cos(), (omega * dt / 2.0).sin(), 0.0];
    let force = 5000.0 * dt;
    let contacts = [
        Located { at: after.world([0.0, 0.0, 0.0]), impulse: direction.map(|d| -d * force), piece: 0 },
        Located { at: after.world([2.0, 0.0, 0.0]), impulse: direction.map(|d| d * force), piece: 3 },
    ];
    let step = Step { whole, before, after, dt, accel: [0.0; 3], contacts: &contacts, anchor: None };
    let part = sum(&parts[2..]);
    let q = [1.0, 0.0, 0.0];
    let load = step.on_part_local(&part, &|i| i >= 2, q);
    let want = 5000.0 + 250.0 * omega * omega * 0.5;
    // the rest pulls the part toward the centre of mass: along -x
    assert!(close(-load.force[0], want, 2e-3), "{:?} against {want} along the axis", load.force);
    assert!(load.force[1].abs() < 2e-3 * want && load.force[2].abs() < 1e-9 * want, "across: {:?}", load.force);
    // no moment about the cut: the old arms, of the end of the step, left 5000 * 0.1 * 1 = 500 N m
    assert!(
        load.moment.iter().all(|m| m.abs() < 2e-3 * want),
        "{:?} of a pull of {want} at a lever of 1 m",
        load.moment
    );
}
