//! A crater that grows from the impact of a body: nothing in the document says when or how
//! big, and the crater, the rigid world and everything that samples either agree on it.

use sr_eval::Evaluator;
use sr_sim::cratering::{crater, Impact, Material, Target};

struct Setup {
    /// Metres per scene unit's reciprocal: scene units per metre.
    ppm: f64,
    mass: f64,
    /// Speed toward the ground, metres per second.
    down: f64,
    /// Speed along the ground, metres per second.
    along: f64,
    material: &'static str,
    extra: &'static str,
    gravity: f64,
}

impl Default for Setup {
    fn default() -> Self {
        Setup { ppm: 1.0, mass: 1500.0, down: 100.0, along: 0.0, material: "softRock", extra: "", gravity: 9.80665 }
    }
}

const RADIUS: f64 = 0.5;

impl Setup {
    fn xml(&self) -> String {
        let k = self.ppm;
        format!(
            r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/>
            <composition>
              <object3D id="rock" primitive="sphere" radius="{r}" x="{x}" y="{y}">
                <rigidBody shape="sphere" mass="{mass}" velocityX="{vx}" velocityY="{vy}" restitution="0" linearDamping="0" angularDamping="0"/>
              </object3D>
              <object3D id="ground" primitive="plane" width="{w}" height="{w}" segments="64" y="{ground}" rotationX="-90">
                <crater source="rock" targetMaterial="{material}" {extra}/>
                <rigidBody type="static" shape="auto"/>
              </object3D>
            </composition>
            <physics gravityY="{g}" pixelsPerMeter="{k}" fixedStep="0.008333333333333333" bounds="none"/></scene>"##,
            r = RADIUS * k,
            // start far enough back to land on the middle of the ground
            x = -self.along * 19.5 / self.down * k,
            y = 40.0 * k,
            mass = self.mass,
            vx = self.along * k,
            vy = self.down * k,
            w = 40.0 * k,
            ground = 60.0 * k,
            material = self.material,
            extra = self.extra,
            g = -self.gravity,
        )
    }

    fn evaluator(&self) -> Evaluator {
        let doc =
            sr_model::load_str(&self.xml(), &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
        Evaluator::new(&doc, &Default::default()).unwrap()
    }
}

fn node<'a>(frame: &'a sr_eval::FrameGraph, id: &str) -> &'a sr_eval::FrameNode {
    frame.nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}"))
}

/// The crater of the ground at `t`: its spec and growth.
fn ground_crater(ev: &Evaluator, t: f64) -> (sr_3d::crater::Spec, f64) {
    let frame = ev.evaluate(t);
    assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
    let c = sr_eval::crater::at(node(&frame, "ground")).unwrap().expect("a crater element");
    (c.kernel.spec(), c.progress)
}

fn full(ev: &Evaluator) -> sr_3d::crater::Spec {
    let (spec, progress) = ground_crater(ev, 5.5);
    assert_eq!(progress, 1.0, "the crater has finished growing");
    spec
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance * b.abs().max(1e-12)
}

/// What the law gives for the rock of `setup` arriving at `speed` along the normal.
fn law(setup: &Setup, speed: f64) -> sr_sim::cratering::Crater {
    let volume = 4.0 / 3.0 * std::f64::consts::PI * RADIUS.powi(3);
    let material = Material::parse(setup.material).unwrap();
    crater(
        &Impact { mass: setup.mass, density: setup.mass / volume, normal_speed: speed },
        &Target { material, density: None, strength: None, gravity: setup.gravity },
    )
    .unwrap()
}

#[test]
fn nothing_grows_until_the_body_arrives_and_then_the_law_decides_how_big() {
    let setup = Setup::default();
    let ev = setup.evaluator();
    // the rock covers 19.5 m at 100 m/s and more: it lands near 0.19 s
    let (_, before) = ground_crater(&ev, 0.1);
    assert_eq!(before, 0.0);
    let (_, during) = ground_crater(&ev, 0.4);
    assert!(during > 0.0 && during < 1.0, "growing: {during}");
    let spec = full(&ev);
    // it arrived at about 100.2 m/s along the normal (it fell 19.5 m more under gravity)
    let wanted = law(&setup, (100.0f64.powi(2) + 2.0 * 9.80665 * 19.5).sqrt());
    // the crest sits at the rim radius, the rim's width reaches back to the crater's edge
    assert!(close(spec.radius, wanted.rim_radius, 0.03), "{} vs {}", spec.radius, wanted.rim_radius);
    assert!(close(spec.depth, wanted.depth, 0.03), "{} vs {}", spec.depth, wanted.depth);
    assert!(close(spec.rim_height, wanted.rim_height, 0.03));
    assert!(close(spec.rim_width, 0.3 * wanted.radius, 0.03));
    // centred where the rock hit (x = 0, on the ground y = 60 seen in the plane's own axes),
    // with its axis along the ground's normal
    assert!(spec.center[0].abs() < 0.2, "{:?}", spec.center);
    let axis = spec.outward;
    assert!(axis.iter().map(|c| c * c).sum::<f64>().sqrt() - 1.0 < 1e-9);
    assert!(axis[2].abs() > 0.99, "along the plane's z: {axis:?}");
}

#[test]
fn it_takes_the_laws_time_to_grow_and_starts_at_the_impact() {
    let setup = Setup::default();
    let ev = setup.evaluator();
    let wanted = law(&setup, 100.2);
    let mut last = 0.0;
    let mut start = None;
    for k in 0..40 {
        let t = 0.1 + 0.05 * k as f64;
        let (_, p) = ground_crater(&ev, t);
        assert!(p >= last, "growth never reverses: {p} after {last} at {t}");
        if p > 0.0 && start.is_none() {
            start = Some(t);
        }
        last = p;
    }
    let start = start.expect("it grows");
    assert!((0.15..0.35).contains(&start), "begins at the impact: {start}");
    let (_, after) = ground_crater(&ev, start + wanted.duration + 0.2);
    assert_eq!(after, 1.0, "done after the law's {} s", wanted.duration);
}

#[test]
fn a_faster_or_heavier_body_makes_a_bigger_crater_and_a_glancing_one_a_smaller() {
    let radius = |s: Setup| full(&s.evaluator()).radius;
    let depth = |s: Setup| full(&s.evaluator()).depth;
    let by_speed: Vec<f64> = [60.0, 100.0, 150.0].map(|v| radius(Setup { down: v, ..Setup::default() })).to_vec();
    assert!(by_speed.windows(2).all(|w| w[0] < w[1]), "speed: {by_speed:?}");
    let by_mass: Vec<f64> = [800.0, 1500.0, 3000.0].map(|m| radius(Setup { mass: m, ..Setup::default() })).to_vec();
    assert!(by_mass.windows(2).all(|w| w[0] < w[1]), "mass: {by_mass:?}");
    // the same 150 m/s speed arriving at 90, 60 and 30 degrees from the ground's plane
    let by_angle: Vec<f64> = [90.0f64, 60.0, 30.0]
        .map(|degrees| {
            let (down, along) = (150.0 * degrees.to_radians().sin(), 150.0 * degrees.to_radians().cos());
            radius(Setup { down, along, ..Setup::default() })
        })
        .to_vec();
    assert!(by_angle.windows(2).all(|w| w[0] > w[1]), "a shallower approach: {by_angle:?}");
    let deep: Vec<f64> = [60.0, 100.0, 150.0].map(|v| depth(Setup { down: v, ..Setup::default() })).to_vec();
    assert!(deep.windows(2).all(|w| w[0] < w[1]), "depth by speed: {deep:?}");
}

#[test]
fn the_material_and_the_overrides_are_read() {
    let radius = |s: Setup| full(&s.evaluator()).radius;
    let soft = radius(Setup::default());
    assert!(radius(Setup { material: "hardRock", ..Setup::default() }) < soft, "a harder target resists more");
    assert!(radius(Setup { extra: r#"strength="100000""#, ..Setup::default() }) > soft, "weaker than the table says");
    assert!(radius(Setup { extra: r#"gravity="1.6""#, ..Setup::default() }) >= soft, "lower gravity");
}

#[test]
fn the_same_scene_at_another_scale_makes_the_same_physical_crater() {
    let small = full(&Setup::default().evaluator());
    let scaled = full(&Setup { ppm: 100.0, ..Setup::default() }.evaluator());
    for (a, b) in [(small.radius, scaled.radius), (small.depth, scaled.depth), (small.rim_height, scaled.rim_height)] {
        assert!(close(b, 100.0 * a, 1e-3), "{a} against {b}");
    }
}

#[test]
fn any_order_of_requests_and_a_fresh_evaluator_give_the_same_crater_bit_for_bit() {
    let setup = Setup::default();
    let ev = setup.evaluator();
    let times = [3.0, 0.1, 0.5, 0.25, 5.5, 0.3, 0.19, 1.0];
    let first: Vec<_> = times.iter().map(|&t| ground_crater(&ev, t)).collect();
    let again: Vec<_> = times.iter().rev().map(|&t| ground_crater(&ev, t)).collect();
    let fresh = setup.evaluator();
    for (k, &t) in times.iter().enumerate() {
        let (a, pa) = first[k];
        let (b, pb) = again[times.len() - 1 - k];
        let (c, pc) = ground_crater(&fresh, t);
        for (x, px) in [(b, pb), (c, pc)] {
            assert_eq!(pa.to_bits(), px.to_bits(), "t = {t}");
            assert_eq!(
                [a.radius, a.depth, a.rim_height, a.rim_width, a.center[0], a.center[2], a.outward[2]]
                    .map(f64::to_bits),
                [x.radius, x.depth, x.rim_height, x.rim_width, x.center[0], x.center[2], x.outward[2]]
                    .map(f64::to_bits),
                "t = {t}"
            );
        }
    }
}

#[test]
fn a_baked_cache_gives_the_same_crater_as_the_live_simulation() {
    let setup = Setup::default();
    let live = setup.evaluator();
    let bytes = live.physics_cache().unwrap();
    let path = std::env::temp_dir().join(format!("sr-crater-impact-{}.physics", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    let xml = setup.xml().replace("<physics ", &format!("<physics cache=\"{}\" ", path.display()));
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let cached = Evaluator::new(&doc, &Default::default()).unwrap();
    for t in [0.1, 0.25, 0.4, 1.0, 5.5, 0.3] {
        let (a, pa) = ground_crater(&live, t);
        let (b, pb) = ground_crater(&cached, t);
        assert_eq!(pa.to_bits(), pb.to_bits(), "t = {t}");
        assert_eq!(
            [
                a.radius,
                a.depth,
                a.rim_height,
                a.rim_width,
                a.center[0],
                a.center[1],
                a.center[2],
                a.outward[0],
                a.outward[2]
            ]
            .map(f64::to_bits),
            [
                b.radius,
                b.depth,
                b.rim_height,
                b.rim_width,
                b.center[0],
                b.center[1],
                b.center[2],
                b.outward[0],
                b.outward[2]
            ]
            .map(f64::to_bits),
            "t = {t}"
        );
    }
    std::fs::remove_file(path).ok();
}

#[test]
fn the_rigid_world_feels_the_crater_it_makes() {
    // the rock hits the ground at y = 60 with its centre half a metre above it and keeps going: the
    // surface sinks away under it, so it ends deeper than it could have on the flat ground
    let setup = Setup::default();
    let ev = setup.evaluator();
    let wanted = law(&setup, 100.2);
    let frame = ev.evaluate(5.5);
    let pose = node(&frame, "rock").pose3.expect("the rock is simulated");
    let y = pose[13];
    assert!(
        y > 60.0 - RADIUS + 0.4 * wanted.depth,
        "the rock ended at y = {y}, on the flat it could not pass {}",
        60.0 - RADIUS
    );
}

#[test]
fn a_crater_without_gravity_to_read_is_an_error() {
    let ev = Setup { gravity: 0.0, ..Setup::default() }.evaluator();
    let frame = ev.evaluate(1.0);
    assert!(frame.failures.iter().any(|f| f.contains("gravity")), "{:?}", frame.failures);
    // a gravity given on the crater is enough
    let ev = Setup { gravity: 0.0, extra: r#"gravity="9.8""#, ..Setup::default() }.evaluator();
    let frame = ev.evaluate(1.0);
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
}

#[test]
fn a_cache_that_predates_contacts_cannot_serve_a_crater_that_needs_them() {
    let setup = Setup::default();
    let mut v3 = Vec::new();
    v3.extend_from_slice(b"SRPHYS03");
    v3.extend_from_slice(&0.0083333333f64.to_le_bytes());
    v3.extend_from_slice(&0.0f64.to_le_bytes());
    for n in [2u64, 0, 2, 0, 1] {
        v3.extend_from_slice(&n.to_le_bytes());
    }
    for _ in 0..(2 * 3 + 2 * 7 + 2) {
        v3.extend_from_slice(&1.0f64.to_le_bytes());
    }
    let path = std::env::temp_dir().join(format!("sr-crater-old-{}.physics", std::process::id()));
    std::fs::write(&path, v3).unwrap();
    let xml = setup.xml().replace("<physics ", &format!("<physics cache=\"{}\" ", path.display()));
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let frame = Evaluator::new(&doc, &Default::default()).unwrap().evaluate(1.0);
    assert!(
        frame.failures.iter().any(|f| f.contains("contact recording")),
        "{:?} {:?}",
        frame.failures,
        frame.problems
    );
    std::fs::remove_file(path).ok();
}
