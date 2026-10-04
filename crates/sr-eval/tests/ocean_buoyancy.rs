//! A rigid body in an ocean that couples to it floats: the weight of the water it displaces holds
//! it up, a drag on its vertical motion settles it, and a body too light for the step is reported.

use sr_eval::Evaluator;

const RHO: f64 = 1000.0;

struct Scene {
    body: &'static str,
    mass: f64,
    ocean: &'static str,
    y: f64,
    step: f64,
}

impl Default for Scene {
    fn default() -> Self {
        Scene {
            body: r#"primitive="sphere" radius="1" segments="24""#,
            mass: 2094.0,
            ocean: r#"bodyCoupling="buoyancy""#,
            y: -1.5,
            step: 1.0 / 120.0,
        }
    }
}

impl Scene {
    fn xml(&self) -> String {
        let shape = if self.body.contains("sphere") { "sphere" } else { "box" };
        format!(
            r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="4"/><composition>
              <object3D id="float" {body} y="{y}">
                <rigidBody shape="{shape}" mass="{mass}" linearDamping="0" angularDamping="0"/>
              </object3D>
              <ocean id="sea" width="16" depth="16" cellSize="1" bottomDepth="20" dt="0.05" boundary="closed" colliders="float" {ocean}/>
            </composition>
            <physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="{step}" bounds="none"/></scene>"##,
            body = self.body,
            y = self.y,
            mass = self.mass,
            ocean = self.ocean,
            step = self.step,
            shape = shape,
        )
    }

    fn evaluator(&self) -> Evaluator {
        let doc =
            sr_model::load_str(&self.xml(), &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
        Evaluator::new(&doc, &Default::default()).unwrap()
    }
}

/// Where the body's origin is at `t`, scene y.
fn height(ev: &Evaluator, t: f64) -> f64 {
    let frame = ev.evaluate(t);
    assert!(
        frame.problems.is_empty() && frame.failures.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.problems,
        frame.failures
    );
    frame.nodes.iter().find(|n| &*n.id == "float").unwrap().pose3.expect("simulated")[13]
}

/// The height of a ball's centre at which it displaces `mass` of water, by bisection on the cap volume.
fn draft(r: f64, mass: f64) -> f64 {
    use sr_sim::hydrostatics::{submerged_sphere, Surface};
    let (mut lo, mut hi) = (-r, r);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        let v = submerged_sphere([0.0, mid, 0.0], r, &Surface { offset: 0.0, slope: [0.0, 0.0] }).volume;
        if v > mass / RHO {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    0.5 * (lo + hi)
}

#[test]
fn a_ball_in_a_coupled_ocean_floats_at_the_draft_its_weight_needs() {
    let ev = Scene::default().evaluator();
    let want = draft(1.0, 2094.0);
    // quadratic drag takes the last of the motion out slowly
    let got = height(&ev, 80.0);
    assert!((got - want).abs() < 0.03, "at {got}, the displaced weight needs {want}");
}

#[test]
fn an_ocean_that_does_not_couple_leaves_the_body_to_fall_through() {
    // bodyCoupling none is the behaviour there always was
    for ocean in ["", r#"bodyCoupling="none""#] {
        let ev = Scene { ocean, ..Scene::default() }.evaluator();
        assert!(height(&ev, 4.0) > 20.0, "it sinks like a stone: {}", height(&ev, 4.0));
    }
}

#[test]
fn a_denser_ball_sinks_even_in_a_coupled_ocean() {
    let ev = Scene { mass: 6000.0, ..Scene::default() }.evaluator();
    let (a, b) = (height(&ev, 2.0), height(&ev, 4.0));
    assert!(a > 1.0 && b > a + 2.0, "{a} then {b}");
}

#[test]
fn without_drag_it_bobs_and_the_water_carries_the_motion_away() {
    // the ball reads the surface under it, which its own motion raises and lowers: the water takes the
    // energy of the bobbing with it, so with no drag of its own the ball settles
    let ev = Scene { ocean: r#"bodyCoupling="buoyancy" bodyDrag="0""#, ..Scene::default() }.evaluator();
    let want = draft(1.0, 2094.0);
    let swing: Vec<f64> = (0..80).map(|k| (height(&ev, 0.25 + 0.25 * k as f64) - want).abs()).collect();
    let most = |from: usize, to: usize| swing[from..to].iter().cloned().fold(0.0, f64::max);
    assert!(most(0, 80) < 1.6, "it does not grow: {swing:?}");
    assert!(most(0, 20) > 0.5, "it bobs at first: {swing:?}");
    assert!(most(60, 80) < 0.1, "and settles by 15 s: {swing:?}");
}

#[test]
fn a_box_floats_at_the_draft_its_weight_needs() {
    // a 2 x 1 x 2 box of 600 kg: draft 0.15 m, its bottom face at the centre + 0.5
    let ev =
        Scene { body: r#"primitive="box" width="2" height="1" depth="2""#, mass: 600.0, y: -0.2, ..Scene::default() }
            .evaluator();
    let got = height(&ev, 90.0);
    assert!((got + 0.5 - 0.15).abs() < 0.03, "centre at {got}");
}

#[test]
fn a_body_too_light_for_the_step_is_reported() {
    // a ball this light hovers where the waterline is small enough for the step, so use a box, whose
    // waterline is the same at any depth: 4 m2 on 0.1 kg is a spring of 392 kN/m on a tenth of a kilogram
    let light =
        Scene { body: r#"primitive="box" width="2" height="1" depth="2""#, mass: 0.1, y: -0.2, ..Scene::default() };
    let ev = light.evaluator();
    let frame = ev.evaluate(1.0);
    let said = frame.problems.iter().chain(&frame.failures).any(|m| m.contains("unstable") && m.contains("omega * dt"));
    assert!(said, "{:?} {:?}", frame.problems, frame.failures);
}

#[test]
fn any_order_of_frames_and_a_fresh_evaluator_give_the_same_motion_bit_for_bit() {
    let ev = Scene::default().evaluator();
    let times = [3.0, 0.5, 2.0, 0.0, 1.25, 3.0];
    let first: Vec<u64> = times.iter().map(|&t| height(&ev, t).to_bits()).collect();
    let fresh = Scene::default().evaluator();
    for (k, &t) in times.iter().enumerate() {
        assert_eq!(first[k], height(&fresh, t).to_bits(), "t = {t}");
    }
}

#[test]
fn such_a_document_cannot_be_baked_and_a_cache_cannot_serve_it() {
    let ev = Scene::default().evaluator();
    let error = ev.physics_cache().unwrap_err();
    assert!(error.contains("cannot be baked"), "{error}");
    // a cache baked from the same scene without the coupling does not stand in for the water
    let plain = Scene { ocean: "", ..Scene::default() };
    let bytes = plain.evaluator().physics_cache().unwrap();
    let path = std::env::temp_dir().join(format!("sr-buoyancy-{}.physics", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    let xml = Scene::default().xml().replace("<physics ", &format!("<physics cache=\"{}\" ", path.display()));
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let frame = Evaluator::new(&doc, &Default::default()).unwrap().evaluate(1.0);
    assert!(frame.failures.iter().any(|f| f.contains("physics cache cannot be combined")), "{:?}", frame.failures);
    std::fs::remove_file(path).ok();
}
