//! SPIKE for the fracture by stress: a block made of N rigid pieces welded at the faces they share. Does the welded block rest where a block rests (no drift, no
//! creep in the joints), does it survive an impact at 100 m/s (nothing is NaN, no energy comes from nowhere), and what does a step of it cost with 256 pieces?
//! Ignored (it measures): `cargo test --release -p sr-sim --test rigid stress_spike -- --ignored --nocapture`.
use sr_sim::{fields::Field, physics3d::*};
use std::time::Instant;

struct Still;
impl Driver3 for Still {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}

fn piece(edge: f64, at: [f64; 3], velocity: [f64; 3], spin: [f64; 3]) -> Body3Spec {
    Body3Spec {
        kind: BodyKind::Dynamic,
        shape: Shape3::Box([edge / 2.0; 3]),
        mass: 2400.0 * edge.powi(3),
        friction: 0.6,
        restitution: 0.0,
        linear_damping: 0.0,
        angular_damping: 0.0,
        velocity,
        angular_velocity: spin,
        group: 0,
        collides_with: None,
        sensor: false,
        fixed_rotation: false,
        bullet: false,
        activate_at: 0.0,
        start: Pose3 { pos: at, rot: [0.0, 0.0, 0.0, 1.0] },
    }
}

fn weld(a: usize, b: usize, anchor: [f64; 3]) -> Joint3Spec {
    Joint3Spec {
        kind: Joint3Kind::Weld,
        a,
        b: Some(b),
        anchor: Some(anchor),
        axis: [0.0, 1.0, 0.0],
        rest_length: None,
        stiffness: None,
        damping: None,
        min: None,
        max: None,
        motor_speed: 0.0,
        max_force: None,
        break_force: None,
    }
}

/// `dims` pieces of `edge` metres welded face to face, the block's centre at `centre` (scene axes: y down, up is minus y).
fn block(
    dims: [usize; 3],
    edge: f64,
    centre: [f64; 3],
    velocity: [f64; 3],
    spin: [f64; 3],
) -> (Vec<Body3Spec>, Vec<Joint3Spec>) {
    let index = |i: usize, j: usize, k: usize| i + dims[0] * (j + dims[1] * k);
    let place = |i: usize, j: usize, k: usize| -> [f64; 3] {
        [
            centre[0] + (i as f64 - (dims[0] as f64 - 1.0) / 2.0) * edge,
            centre[1] + (j as f64 - (dims[1] as f64 - 1.0) / 2.0) * edge,
            centre[2] + (k as f64 - (dims[2] as f64 - 1.0) / 2.0) * edge,
        ]
    };
    let mut bodies = Vec::new();
    let mut joints = Vec::new();
    for k in 0..dims[2] {
        for j in 0..dims[1] {
            for i in 0..dims[0] {
                bodies.push(piece(edge, place(i, j, k), velocity, spin));
            }
        }
    }
    for k in 0..dims[2] {
        for j in 0..dims[1] {
            for i in 0..dims[0] {
                let here = place(i, j, k);
                for (di, dj, dk) in [(1, 0, 0), (0, 1, 0), (0, 0, 1)] {
                    if i + di < dims[0] && j + dj < dims[1] && k + dk < dims[2] {
                        let there = place(i + di, j + dj, k + dk);
                        let mid = [(here[0] + there[0]) / 2.0, (here[1] + there[1]) / 2.0, (here[2] + there[2]) / 2.0];
                        joints.push(weld(index(i, j, k), index(i + di, j + dj, k + dk), mid));
                    }
                }
            }
        }
    }
    (bodies, joints)
}

fn world(step: f64, floor: bool, gravity: f64, bodies: Vec<Body3Spec>, joints: Vec<Joint3Spec>) -> World3 {
    World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.0,
        step,
        gravity: [0.0, -gravity, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: if floor { Bounds3::Floor { y: 0.0 } } else { Bounds3::None },
        joints,
        bodies,
    })
}

const SIZES: [[usize; 3]; 3] = [[2, 2, 2], [4, 4, 4], [8, 8, 4]];

#[test]
#[ignore = "a measurement"]
fn the_welded_block_rests_without_drift_or_creep_in_its_joints() {
    for dims in SIZES {
        let edge = 2.0 / dims[0] as f64;
        let height = edge * dims[1] as f64;
        // resting on the floor (y = 0, up is minus y): the centre is half the height above it
        let (bodies, joints) = block(dims, edge, [0.0, -height / 2.0, 0.0], [0.0; 3], [0.0; 3]);
        let n = bodies.len();
        let start: Vec<[f64; 3]> = bodies.iter().map(|b| b.start.pos).collect();
        let mut w = world(1.0 / 120.0, true, 9.80665, bodies, joints);
        let began = Instant::now();
        let first = w.frame_at(1.0, &mut Still);
        let one_second = w.joint_reactions();
        let last = w.frame_at(2.0, &mut Still);
        let two_seconds = w.joint_reactions();
        let elapsed = began.elapsed();
        assert!(first.errors.is_empty() && last.errors.is_empty(), "{:?}", last.errors);
        let drift = |f: &Frame3| {
            f.bodies
                .iter()
                .zip(&start)
                .map(|(p, s)| (0..3).map(|a| (p.pos[a] - s[a]).powi(2)).sum::<f64>().sqrt())
                .fold(0.0f64, f64::max)
        };
        let force = |r: &[Option<[f64; 6]>]| {
            r.iter().flatten().map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()).fold(0.0f64, f64::max)
        };
        println!(
            "SPIKE REST {n} pieces: drift at 1 s {:.3e} m, at 2 s {:.3e} m; largest joint force {:.4e} N at 1 s and {:.4e} N at 2 s; {:.2} ms a step (240 steps, replays included)",
            drift(&first),
            drift(&last),
            force(&one_second),
            force(&two_seconds),
            elapsed.as_secs_f64() * 1000.0 / 240.0
        );
    }
}

#[test]
#[ignore = "a measurement"]
fn the_welded_block_hits_the_floor_at_100_m_s_without_nan_and_without_energy_from_nowhere() {
    // the first size is one box of the block's size, no welds: the control, what a body does at this speed with no joint in it
    let cases = [
        ([1, 1, 1], 240.0),
        ([1, 1, 1], 960.0),
        ([2, 2, 2], 240.0),
        ([2, 2, 2], 960.0),
        ([4, 4, 4], 240.0),
        ([4, 4, 4], 960.0),
        ([8, 8, 4], 240.0),
        ([8, 8, 4], 960.0),
    ];
    for (dims, rate) in cases {
        let edge = 2.0 / dims[0] as f64;
        let height = edge * dims[1] as f64;
        let (bodies, joints) = block(dims, edge, [0.0, -height / 2.0 - 3.0, 0.0], [0.0, 100.0, 0.0], [0.0; 3]);
        let n = bodies.len();
        let kinetic = |velocities: &[Velocity3], masses: &[f64]| -> f64 {
            velocities.iter().zip(masses).map(|(v, m)| 0.5 * m * v.linear.iter().map(|c| c * c).sum::<f64>()).sum()
        };
        let masses: Vec<f64> = bodies.iter().map(|b| b.mass).collect();
        let before = kinetic(&vec![Velocity3 { linear: [0.0, 100.0, 0.0], ..Default::default() }; n], &masses);
        let mut w = world(1.0 / rate, true, 9.80665, bodies, joints);
        let began = Instant::now();
        let frame = w.frame_at(0.5, &mut Still);
        let elapsed = began.elapsed();
        let finite = frame.bodies.iter().all(|p| p.pos.iter().chain(&p.rot).all(|v| v.is_finite()))
            && frame.velocities.iter().all(|v| v.linear.iter().all(|c| c.is_finite()));
        let after = kinetic(&frame.velocities, &masses);
        let reactions = w.joint_reactions();
        let worst =
            reactions.iter().flatten().map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()).fold(0.0f64, f64::max);
        println!(
            "SPIKE IMPACT {n} pieces at {rate} steps/s: errors {:?}, finite {finite}, translational KE before {before:.4e} J, after 0.5 s {after:.4e} J; largest joint force {worst:.3e} N; {:.2} ms a step",
            frame.errors,
            elapsed.as_secs_f64() * 1000.0 / (0.5 * rate)
        );
    }
}

#[test]
#[ignore = "a measurement"]
fn the_welded_block_in_free_flight_keeps_its_momentum_and_does_not_gain_energy() {
    for dims in SIZES {
        let edge = 2.0 / dims[0] as f64;
        // a spinning, moving block in no gravity and no floor: nothing external, so the momentum of the pieces is the block's, whatever the welds do
        let (bodies, joints) = block(dims, edge, [0.0; 3], [3.0, -2.0, 1.0], [40.0, -25.0, 60.0]);
        let n = bodies.len();
        let masses: Vec<f64> = bodies.iter().map(|b| b.mass).collect();
        let mut w = world(1.0 / 240.0, false, 0.0, bodies, joints);
        let began = Instant::now();
        let frame = w.frame_at(1.0, &mut Still);
        let elapsed = began.elapsed();
        let total: f64 = masses.iter().sum();
        let momentum: [f64; 3] =
            std::array::from_fn(|a| frame.velocities.iter().zip(&masses).map(|(v, m)| m * v.linear[a]).sum::<f64>());
        let want = [3.0 * total, -2.0 * total, 1.0 * total];
        let error = (0..3).map(|a| (momentum[a] - want[a]).powi(2)).sum::<f64>().sqrt() / total;
        println!(
            "SPIKE FLIGHT {n} pieces: errors {:?}, momentum error {error:.3e} (m/s of the block); {:.2} ms a step",
            frame.errors,
            elapsed.as_secs_f64() * 1000.0 / 240.0
        );
    }
}
