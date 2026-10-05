//! Ejecta thrown out of a crater that is still growing, with the crater's own ground as their
//! collider: they land on it, with no error, and the replay is identical.
use sr_eval::Evaluator;

fn scene(count: usize, segments: usize, radius: &str) -> String {
    format!(
        r#"<scene version="1.3"><project width="320" height="180" fps="24" duration="6"/><composition>
          <object3D id="impactor" primitive="sphere" radius="2" segments="24" x="-74.4" y="-120">
            <rigidBody shape="sphere" mass="90478" velocityX="50" velocityY="72" restitution="0" linearDamping="0" angularDamping="0"/>
          </object3D>
          <object3D id="ground" primitive="plane" width="160" height="160" segments="{segments}" y="0" rotationX="-90">
            <crater id="pit" source="impactor" targetMaterial="softRock"/>
            <rigidBody type="static" shape="auto"/>
          </object3D>
          <particles3D id="ejecta" shape="sphere" segments="6" rate="0" seed="20261004" size="0.34" gravityY="9.80665" drag="0" lifetime="6" dt="0.0416666666666667" maxParticles="4000" colliders="ground"{radius}>
            <burst crater="pit" count="{count}"/>
          </particles3D>
        </composition><physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"#
    )
}

fn evaluator(count: usize, segments: usize) -> Evaluator {
    with_radius(count, segments, "")
}

fn with_radius(count: usize, segments: usize, radius: &str) -> Evaluator {
    let doc = sr_model::load_str(&scene(count, segments, radius), &sr_model::LoadOptions::without_assets()).unwrap();
    Evaluator::new(&doc, &Default::default()).unwrap()
}

fn ejecta(ev: &Evaluator, t: f64) -> std::sync::Arc<sr_eval::particles3d::SimParticles3D> {
    let frame = ev.evaluate(t);
    assert!(
        frame.failures.is_empty() && frame.problems.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.failures,
        frame.problems
    );
    frame.nodes.iter().find(|n| &*n.id == "ejecta").unwrap().particles3d.clone().expect("particles")
}

#[test]
fn ejecta_of_a_growing_crater_collide_with_its_ground_without_error() {
    let ev = evaluator(1000, 80);
    // the rock lands near 1.49 s and the crater grows for a while after it
    for t in [1.6, 1.8, 2.0, 2.5, 3.0, 4.0, 5.5] {
        let p = ejecta(&ev, t);
        assert!(!p.frame.particles.is_empty() || t < 1.5);
    }
}

#[test]
fn none_of_them_ends_under_the_ground_outside_the_crater() {
    let ev = evaluator(1000, 80);
    let p = ejecta(&ev, 5.5);
    // y is downward and the original ground is y = 0, so a particle outside the crater (and its
    // rim) with y > 0 is buried
    let buried =
        p.frame.particles.iter().filter(|q| q.position[0].hypot(q.position[2]) > 12.0 && q.position[1] > 0.0).count();
    assert_eq!(buried, 0);
}

#[test]
fn scrubbing_gives_the_same_ejecta() {
    let ev = evaluator(1000, 80);
    let first = ejecta(&ev, 3.0);
    ejecta(&ev, 1.7);
    ejecta(&ev, 2.2);
    let again = ejecta(&ev, 3.0);
    assert_eq!(first.key, again.key);
    assert_eq!(first.frame, again.frame);
}

#[test]
fn the_whole_land_scene_with_the_ground_as_collider_runs_to_the_end() {
    // 4000 particles over a plane of 160 segments: the scene of the land impact
    let ev = evaluator(4000, 160);
    for t in [1.6, 1.8, 2.0, 2.5, 3.0, 4.0, 5.5, 5.9] {
        let p = ejecta(&ev, t);
        let buried = p
            .frame
            .particles
            .iter()
            .filter(|q| q.position[0].hypot(q.position[2]) > 9.0 && q.position[1] > 0.0)
            .count();
        assert_eq!(buried, 0, "buried at {t}");
        assert!(p.frame.particles.iter().all(|q| q.position[1] < 40.0), "none sinks through the ground at {t}");
    }
    // all of them are born, and by the end of the run none is more than a few metres above the ground
    // (they have landed; with no friction they slide about the bowl rather than come to rest)
    let end = ejecta(&ev, 5.9);
    assert_eq!(end.frame.particles.len(), 4000);
    assert!(end.frame.particles.iter().all(|q| q.position[1] > -5.0));
}

#[test]
fn ejecta_of_the_physical_size_of_the_rocks_do_not_go_through_the_ground() {
    // a collision radius of 0.17 m, the size of the rocks the 4000 particles stand for: some 700 of them went
    // through the ground when they were born at the height of the plane the rim of the growing crater had
    // since risen over (and 110 when born only at the nearest point of the original surface)
    let ev = with_radius(4000, 160, r#" collisionRadius="0.17""#);
    for t in [2.0, 3.0, 5.9] {
        let p = ejecta(&ev, t);
        let through = p.frame.particles.iter().filter(|q| q.position[1] > 3.3).count();
        assert_eq!(through, 0, "through the ground at {t}");
    }
}
