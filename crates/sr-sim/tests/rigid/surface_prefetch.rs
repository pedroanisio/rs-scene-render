//! The deforming surface of a body that has been hit is rebuilt at every rigid step while its crater grows, and building a
//! trimesh is most of the step. The shapes of a frame's steps are functions of their times, so they are built ahead on
//! several threads and each step takes its own: the simulation must be the same bit for bit, whatever the number of
//! threads, and the same after a restore from a checkpoint.
use sr_sim::{fields::Field, physics3d::*};

const STEP: f64 = 0.005;
/// Seconds the pit takes to reach its depth, from the impact.
const GROWTH: f64 = 0.3;
const SIDE: usize = 90;

struct Pit;

impl Driver3 for Pit {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
    fn surface(
        &mut self,
        t: f64,
        which: usize,
        revision: Option<u64>,
        impact: Option<&Impact3>,
    ) -> Result<Option<ColliderUpdate3>, String> {
        let Some(impact) = impact.filter(|_| which == 1) else { return Ok(None) };
        let progress = ((t - impact.time) / GROWTH).clamp(0.0, 1.0);
        if revision == Some(progress.to_bits()) {
            return Ok(None);
        }
        // the top face of the slab, in its own axes (10 above its centre, y down), a grid of 90 x 90 squares with a pit
        // under where the sphere came down
        let at = |i: usize| -200.0 + 400.0 * i as f64 / SIDE as f64;
        let mut vertices = Vec::new();
        for z in 0..=SIDE {
            for x in 0..=SIDE {
                let (px, pz) = (at(x), at(z));
                let r2 = (px + 30.0).powi(2) + pz * pz;
                vertices.push([px, -10.0 + 12.0 * progress * (-r2 / (2.0 * 25.0 * 25.0)).exp(), pz]);
            }
        }
        let mut triangles = Vec::new();
        for z in 0..SIDE {
            for x in 0..SIDE {
                let a = (z * (SIDE + 1) + x) as u32;
                let (b, c, d) = (a + 1, a + SIDE as u32 + 1, a + SIDE as u32 + 2);
                triangles.push([a, c, b]);
                triangles.push([b, c, d]);
            }
        }
        Ok(Some(ColliderUpdate3 { revision: progress.to_bits(), vertices, triangles, max_bytes: 1 << 26 }))
    }
}

fn sphere(at: [f64; 3], velocity: [f64; 3]) -> Body3Spec {
    Body3Spec {
        kind: BodyKind::Dynamic,
        shape: Shape3::Sphere(10.0),
        mass: 2.0,
        friction: 0.5,
        restitution: 0.0,
        linear_damping: 0.0,
        angular_damping: 0.0,
        velocity,
        angular_velocity: [0.0; 3],
        group: 0,
        collides_with: None,
        sensor: false,
        fixed_rotation: false,
        bullet: false,
        activate_at: 0.0,
        start: Pose3 { pos: at, ..Pose3::default() },
    }
}

/// A 2 kg sphere falling at 200 px/s onto the top face (y = 90) of a slab whose surface is the one of `Pit`.
fn world(prefetch: bool) -> World3 {
    let slab = Body3Spec {
        kind: BodyKind::Static,
        shape: Shape3::Box([200.0, 10.0, 200.0]),
        start: Pose3 { pos: [30.0, 100.0, 0.0], ..Pose3::default() },
        ..sphere([0.0; 3], [0.0; 3])
    };
    World3::new(World3Spec {
        fix_internal_edges: true,
        start: 0.0,
        step: STEP,
        gravity: [0.0, -9.81, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: vec![sphere([0.0; 3], [0.0, 200.0, 0.0]), slab],
    })
    .with_impact_watches(vec![ImpactWatch { source: 0, owner: 1, min_impulse: 1.0 }])
    .unwrap()
    .with_surface_prefetch(prefetch)
}

/// The poses of the frames at the times, to the bit.
fn poses(world: &mut World3, times: &[f64]) -> Vec<[u64; 14]> {
    times
        .iter()
        .map(|&t| {
            let frame = world.frame_at(t, &mut Pit);
            assert!(frame.errors.is_empty(), "t = {t}: {:?}", frame.errors);
            let (a, b) = (frame.bodies[0], frame.bodies[1]);
            let mut bits = [0u64; 14];
            for (k, v) in a.pos.iter().chain(&a.rot).chain(&b.pos).chain(&b.rot[..3]).enumerate() {
                bits[k] = v.to_bits();
            }
            bits
        })
        .collect()
}

fn on_threads<T: Send>(threads: usize, work: impl FnOnce() -> T + Send) -> T {
    rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(work)
}

/// A frame every 1/24 s for 2 s: the sphere lands at 0.4 s and the pit grows for 0.3 s after.
fn frames() -> Vec<f64> {
    (0..=48).map(|k| k as f64 / 24.0).collect()
}

#[test]
fn building_the_shapes_ahead_gives_the_same_simulation_on_any_number_of_threads() {
    let reference = poses(&mut world(false), &frames());
    for threads in [1, 2, 8] {
        let (w, got) = on_threads(threads, || {
            let mut w = world(true);
            let got = poses(&mut w, &frames());
            (w, got)
        });
        assert_eq!(got, reference, "{threads} threads");
        assert!(w.prefetch_hits() > 0, "{threads} threads: no step took a shape that was built ahead");
        println!("PREFETCH {threads} threads: {} steps took a shape built ahead", w.prefetch_hits());
    }
}

#[test]
fn a_frame_replayed_from_a_checkpoint_is_the_frame_that_was_simulated() {
    let live = poses(&mut world(true), &[0.1, 0.5, 0.6, 0.9, 1.5, 2.0]);
    // with no frame memory every earlier time restores a checkpoint and replays its steps, shapes included
    let mut replayer = world(true).with_frame_log_budget(0);
    let _ = poses(&mut replayer, &[2.0]);
    let again = poses(&mut replayer, &[0.9, 0.6, 0.5, 0.1]);
    assert_eq!(again, [live[3], live[2], live[1], live[0]]);
    assert_eq!(poses(&mut replayer, &[1.5, 2.0]), [live[4], live[5]]);
}
