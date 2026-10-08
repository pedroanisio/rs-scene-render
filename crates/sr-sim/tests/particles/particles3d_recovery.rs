//! Particles that start overlapping a surface are pushed clear of it by collisions that advance no
//! time. Such recoveries have a limit of their own, apart from the 16 collisions of a step that
//! move time forward, and what worked before the limit was separated keeps its bits.
use rapier3d_f64::parry::{math::Vector, shape::SharedShape};
use sr_sim::{
    particles3d::{collider::Collider, Driver, Emission, Emitter, Error, Hit, Spec},
    physics3d::Pose3,
};

/// A trough: two slopes meeting along z at x = 0, y = 2 (y downward, so the bottom is at y = 2).
struct Trough {
    collider: Collider,
    height: f64,
    /// Collisions reported that advance no time.
    recoveries: u64,
}
impl Trough {
    fn new(height: f64) -> Self {
        let slope = |x: f64| 2.0 - (x.abs()) * 0.8;
        let mesh = SharedShape::trimesh(
            vec![
                Vector::new(-6., slope(6.), -6.),
                Vector::new(0., slope(0.), -6.),
                Vector::new(6., slope(6.), -6.),
                Vector::new(-6., slope(6.), 6.),
                Vector::new(0., slope(0.), 6.),
                Vector::new(6., slope(6.), 6.),
            ],
            vec![[0, 1, 4], [0, 4, 3], [1, 2, 5], [1, 5, 4]],
        )
        .unwrap();
        Trough { collider: Collider::new(mesh, Pose3::default(), [0.; 3], [0.; 3], 0.).unwrap(), height, recoveries: 0 }
    }
}
impl Driver for Trough {
    fn emission(&mut self, _t: f64) -> Result<Emission, Error> {
        let mut m = sr_volume::Transform::identity().columns();
        m[13] = self.height;
        Ok(Emission { transform: sr_volume::Transform::new(m).unwrap(), ..Default::default() })
    }
    fn acceleration(&mut self, _t: f64, _p: [f64; 3], _v: [f64; 3]) -> Result<[f64; 3], Error> {
        Ok([0.0, 10.0, 0.0])
    }
    fn sweep(&mut self, t: f64, dt: f64, a: [f64; 3], b: [f64; 3], r: f64) -> Result<Option<Hit>, Error> {
        let hit = self.collider.sweep(t, dt, a, b, r)?;
        self.recoveries += hit.as_ref().is_some_and(|h| h.fraction == 0.) as u64;
        Ok(hit)
    }
}

fn spec() -> Spec {
    Spec {
        step: 0.05,
        lifetime: 20.0,
        speed: 3.0,
        speed_variance: 2.0,
        spread: 180.0,
        radius: 0.5,
        restitution: 0.3,
        friction: 0.2,
        max_particles: 200,
        seed: 11,
        bursts: vec![sr_sim::particles3d::Burst { time: 0.0, count: 60, repeat: 0, interval: 1.0 }],
        ..Default::default()
    }
}

/// A digest of every field of the particles of the frames, stable across runs.
fn digest(emitter: &mut Emitter, driver: &mut Trough, times: &[f64]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut word = |v: u64| {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for &t in times {
        let frame = emitter.at(t, driver).unwrap();
        word(frame.particles.len() as u64);
        for p in &frame.particles {
            word(p.id);
            for v in p.position.iter().chain(&p.velocity) {
                word(v.to_bits());
            }
        }
    }
    h
}

/// Recorded on the solver as it was before recoveries were counted apart: particles born
/// overlapping both slopes of a trough, pushed out by several collisions in a row.
const OVERLAPPED_BIRTHS: u64 = 7715294112800720132;

#[test]
fn a_scene_that_recovers_from_overlapped_births_keeps_its_bits() {
    let mut emitter = Emitter::new(spec()).unwrap();
    let mut trough = Trough::new(1.9);
    let got = digest(&mut emitter, &mut trough, &[0.05, 0.3, 1.0, 2.0, 4.0]);
    println!("DIGEST {got} after {} recoveries", trough.recoveries);
    assert!(trough.recoveries > 100, "the scene must recover from overlap: {}", trough.recoveries);
    assert_eq!(got, OVERLAPPED_BIRTHS);
}

/// Reports `recoveries` contacts at fraction 0, one after the other, then none.
struct Stuck {
    left: u32,
}
impl Driver for Stuck {
    fn emission(&mut self, _t: f64) -> Result<Emission, Error> {
        Ok(Emission::default())
    }
    fn acceleration(&mut self, _t: f64, _p: [f64; 3], _v: [f64; 3]) -> Result<[f64; 3], Error> {
        Ok([0.0; 3])
    }
    fn sweep(&mut self, _t: f64, _dt: f64, from: [f64; 3], _to: [f64; 3], _r: f64) -> Result<Option<Hit>, Error> {
        if self.left == 0 {
            return Ok(None);
        }
        self.left -= 1;
        Ok(Some(Hit { fraction: 0., position: from, normal: [0., -1., 0.], velocity: [0.; 3] }))
    }
}
fn one_particle() -> Emitter {
    Emitter::new(Spec {
        step: 0.1,
        lifetime: 10.0,
        speed: 0.0,
        gravity: [0.0; 3],
        max_particles: 4,
        bursts: vec![sr_sim::particles3d::Burst { time: 0.0, count: 1, repeat: 0, interval: 1.0 }],
        ..Default::default()
    })
    .unwrap()
}

#[test]
fn many_recoveries_in_a_step_are_fine_and_do_not_use_up_the_collisions_that_move_time() {
    // 50 pushes in a row, more than the 16 collisions a step may have
    let mut emitter = one_particle();
    let frame = emitter.at(0.3, &mut Stuck { left: 50 }).unwrap();
    assert_eq!(frame.particles.len(), 1);
}

#[test]
fn recoveries_past_their_own_limit_are_an_error_that_says_so() {
    let mut emitter = one_particle();
    let error = emitter.at(0.3, &mut Stuck { left: 1000 }).unwrap_err();
    assert!(error.to_string().contains("penetration recoveries"), "{error}");
    // the published frame is untouched, and the emitter still answers other times
    assert!(emitter.at(0.0, &mut Stuck { left: 0 }).is_ok());
}
