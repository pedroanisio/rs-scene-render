//! Ejecta thrown out of a crater that is still growing, with the crater's own ground as their
//! collider: they land on it, with no error, and the replay is identical.
use sr_eval::Evaluator;

fn scene(count: usize, segments: usize, radius: &str, mass: u32) -> String {
    format!(
        r#"<scene version="1.3"><project width="320" height="180" fps="24" duration="6"/><composition>
          <object3D id="impactor" primitive="sphere" radius="2" segments="24" x="-74.4" y="-120">
            <rigidBody shape="sphere" mass="{mass}" velocityX="50" velocityY="72" restitution="0" linearDamping="0" angularDamping="0"/>
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
    heavy(count, segments, radius, 90478)
}

fn heavy(count: usize, segments: usize, radius: &str, mass: u32) -> Evaluator {
    let doc =
        sr_model::load_str(&scene(count, segments, radius, mass), &sr_model::LoadOptions::without_assets()).unwrap();
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

#[test]
fn the_ejecta_of_the_biggest_crater_can_have_friction_and_land_on_a_surface_that_accelerates() {
    // a rock of 270000 kg, the ejecta with a restitution of 0.15 and a friction of 0.7: it stopped the solver at
    // 2 s with more than 16 collisions in a step. The particles that did it had been born under the rim that was
    // rising over them, and the sheet that pushed them down was caught up with, again and again, by the rim
    // passing; born on the surface the crater has by then they are not
    let ev = heavy(4000, 160, r#" bounce="0.15" friction="0.7""#, 270000);
    for t in [1.8, 2.0, 2.5, 3.0, 5.9] {
        let p = ejecta(&ev, t);
        assert!(p.frame.particles.iter().all(|q| q.position[1] < 40.0), "none sinks through the ground at {t}");
    }
    let again = ejecta(&ev, 2.0);
    ejecta(&ev, 1.7);
    assert_eq!(ejecta(&ev, 2.0).frame, again.frame, "scrubbing gives the same ejecta");
}

/// The same scene with the ejecta's crater given a mantle.
fn with_mantle(count: usize, segments: usize) -> Evaluator {
    let xml = scene(count, segments, "", 90478)
        .replace(r#"<crater id="pit" source="impactor""#, r#"<crater id="pit" mantle="true" source="impactor""#);
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    Evaluator::new(&doc, &Default::default()).unwrap()
}

#[test]
fn with_a_mantle_the_ejecta_that_come_to_rest_are_taken_out_and_the_replay_is_identical() {
    let (plain, mantle) = (evaluator(1000, 80), with_mantle(1000, 80));
    // in the air they are the same particles in number: nothing has landed before the rock does
    let (a, b) = (ejecta(&plain, 1.6).frame.particles.len(), ejecta(&mantle, 1.6).frame.particles.len());
    println!("EJECTA at 1.6 s: {a} without a mantle, {b} with one");
    assert!(b <= a);
    // later the ones that have come to rest on the ground have become the ground
    let (a, b) = (ejecta(&plain, 5.5).frame.particles.len(), ejecta(&mantle, 5.5).frame.particles.len());
    println!("EJECTA at 5.5 s: {a} without a mantle, {b} with one");
    assert!(b < a, "{b} against {a}");
    // almost none of those left is at rest on the ground: one caught at the top of a hop a few centimetres high is
    // slow and near it without having settled
    let resting = ejecta(&mantle, 5.5)
        .frame
        .particles
        .iter()
        .filter(|q| q.velocity.iter().map(|c| c * c).sum::<f64>().sqrt() < 0.5 && q.position[1] > -0.01)
        .count();
    println!("EJECTA slow and on the ground, of {b}: {resting}");
    assert!(resting <= 5, "{resting}");
    // scrubbing gives the same ones
    let first = ejecta(&mantle, 3.0);
    ejecta(&mantle, 1.7);
    ejecta(&mantle, 4.4);
    let again = ejecta(&mantle, 3.0);
    assert_eq!(first.key, again.key);
    assert_eq!(first.frame, again.frame);
}

/// The same scene with the ejecta's crater given an angle of repose for what settles.
fn with_repose(count: usize, segments: usize) -> Evaluator {
    let xml = scene(count, segments, "", 90478)
        .replace(r#"<crater id="pit" source="impactor""#, r#"<crater id="pit" repose="35" source="impactor""#);
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    Evaluator::new(&doc, &Default::default()).unwrap()
}

/// The mass of the particles in a frame, kilograms.
fn mass_of(ev: &Evaluator, t: f64) -> f64 {
    ejecta(ev, t).frame.particles.iter().map(|q| q.mass).sum()
}

/// The deposit that lies on the ground at `t`: none for a crater that has not got one.
fn deposit_of(ev: &Evaluator, t: f64) -> Option<sr_3d::crater::Deposit> {
    let frame = ev.evaluate(t);
    assert!(
        frame.failures.is_empty() && frame.problems.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.failures,
        frame.problems
    );
    let node = frame.nodes.iter().find(|n| &*n.id == "ground").unwrap();
    sr_eval::crater::at(node).unwrap().expect("a crater").kernel.deposit().cloned()
}

#[test]
fn the_mass_of_the_ejecta_that_settles_is_in_the_ground_as_a_deposit_and_nowhere_else() {
    let (plain, bed) = (evaluator(1000, 80), with_repose(1000, 80));
    // the whole of what is thrown out, from the scene without the attribute, in which none is removed
    let thrown = mass_of(&plain, 5.5);
    let left = mass_of(&bed, 5.5);
    let settled = thrown - left;
    println!("DEBRIS thrown {thrown:.1} kg, still moving {left:.1} kg, settled {settled:.1} kg");
    assert!(settled > 0.1 * thrown && left > 0.0, "{settled} of {thrown}");
    // in the scene's units, the crater of soft rock of 2100 kg/m3 at one scene unit a metre
    let deposit = deposit_of(&bed, 5.5).expect("a deposit on the ground");
    println!("DEBRIS deposit {:.6} against {:.6}", deposit.volume(), settled / 2100.0);
    assert!(
        (deposit.volume() - settled / 2100.0).abs() < 1e-9 * (settled / 2100.0),
        "{} against {}",
        deposit.volume(),
        settled / 2100.0
    );
    // without the attribute nothing is put on the ground
    assert!(deposit_of(&plain, 5.5).is_none());
    // before anything has settled there is none: at 1.6 s the rock has just landed
    assert!(deposit_of(&bed, 1.6).is_none_or(|d| d.volume() < 0.05 * settled / 2100.0));
}

#[test]
fn the_deposit_grows_as_the_ejecta_settle_and_is_the_same_whatever_order_the_frames_are_asked_in() {
    let ev = with_repose(1000, 80);
    let started = std::time::Instant::now();
    let volumes: Vec<f64> =
        [2.0, 3.0, 4.0, 5.5].iter().map(|&t| deposit_of(&ev, t).map_or(0.0, |d| d.volume())).collect();
    println!("DEBRIS volume at 2, 3, 4, 5.5 s: {volumes:.3?}; {:.2} s for the four", started.elapsed().as_secs_f64());
    // the same four frames of the scene without the deposit, for what the deposit costs
    let plain = evaluator(1000, 80);
    let started = std::time::Instant::now();
    for t in [2.0, 3.0, 4.0, 5.5] {
        ejecta(&plain, t);
    }
    println!("DEBRIS the same frames without the angle of repose: {:.2} s", started.elapsed().as_secs_f64());
    assert!(volumes.windows(2).all(|w| w[1] >= w[0]), "{volumes:?}");
    assert!(volumes[3] > volumes[0]);
    // going back and forward gives the same, and so does an evaluator that has seen none of it
    let forward = deposit_of(&ev, 5.5).unwrap();
    let early = deposit_of(&ev, 3.0).unwrap();
    let again = deposit_of(&ev, 5.5).unwrap();
    assert_eq!(forward, again);
    let fresh = with_repose(1000, 80);
    assert_eq!(deposit_of(&fresh, 3.0).unwrap(), early);
    assert_eq!(deposit_of(&fresh, 5.5).unwrap(), forward);
}
