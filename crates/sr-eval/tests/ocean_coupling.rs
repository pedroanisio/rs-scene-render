//! The ocean responds to a crater that deforms the seabed, with no authored impulse.
use sr_eval::{Evaluator, FrameGraph};

/// A 128 x 128 ocean of 2-unit cells over a 12-unit layer of water, and a seabed
/// plane at the same depth whose crater (radius 40, rim 3) grows from 0.5 s to 1.5 s.
fn scene(crater_depth: f64, extra: &str) -> String {
    format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
          <object3D id="seabed" primitive="plane" width="400" height="400" segments="80" y="12" rotationX="-90">
            <crater radius="40" depth="{crater_depth}" rimHeight="3" rimWidth="8" start="0.5" end="1.5"/>
          </object3D>
          <ocean id="sea" width="128" depth="128" cellSize="2" bottomDepth="12" dt="0.0416666666666667" boundary="closed" colliders="seabed">{extra}</ocean>
        </composition></scene>"#
    )
}
fn evaluator(xml: &str) -> Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    Evaluator::new(&doc, &Default::default()).unwrap()
}
fn sea(frame: &FrameGraph) -> &sr_eval::ocean::SimOcean {
    frame.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap()
}
/// Largest change of water depth from the 12-unit rest layer.
fn peak(frame: &FrameGraph) -> f64 {
    sea(frame).frame.cells.iter().map(|c| (c.depth - 12.0).abs()).fold(0.0, f64::max)
}

#[test]
fn a_crater_alone_makes_a_wave_and_conserves_the_water() {
    let ev = evaluator(&scene(8.0, ""));
    let before = ev.evaluate(0.3);
    assert!(before.problems.is_empty(), "{:?}", before.problems);
    // Nothing has moved yet: every column is at rest over the original bed.
    assert!(sea(&before).frame.cells.iter().all(|c| c.depth == 12.0));
    assert!(sea(&before).frame.bed.iter().all(|&y| y == 12.0));
    let after = ev.evaluate(2.5);
    assert!(after.problems.is_empty(), "{:?}", after.problems);
    let frame = &sea(&after).frame;
    let centre = 32 * 64 + 32;
    assert!(frame.bed[centre] > 15.0, "the crater lowers the bed under the centre: {}", frame.bed[centre]);
    assert!(peak(&after) > 0.05, "no wave: {}", peak(&after));
    let volume: f64 = frame.cells.iter().map(|c| c.depth).sum();
    assert!((volume - 12.0 * 4096.0).abs() < 1e-8 * volume, "water volume {volume}");
    assert!(frame.cells.iter().all(|c| c.depth >= 0.0));
}

#[test]
fn a_larger_crater_makes_a_larger_wave() {
    let heights: Vec<f64> = [4.0, 8.0, 16.0]
        .iter()
        .map(|&depth| {
            let frame = evaluator(&scene(depth, "")).evaluate(2.5);
            assert!(frame.problems.is_empty(), "{:?}", frame.problems);
            peak(&frame)
        })
        .collect();
    println!("WAVE peak depth change for crater depths 4, 8, 16: {heights:?}");
    assert!(heights[0] < heights[1] && heights[1] < heights[2], "{heights:?}");
    assert!(heights[2] > 1.3 * heights[1], "{heights:?}");
}

#[test]
fn replay_after_a_backward_seek_reproduces_the_frame_and_its_key() {
    let ev = evaluator(&scene(8.0, ""));
    let first = ev.evaluate(2.5);
    ev.evaluate(0.7);
    ev.evaluate(1.2);
    let again = ev.evaluate(2.5);
    assert!(again.problems.is_empty(), "{:?}", again.problems);
    assert_eq!(sea(&first).frame, sea(&again).frame);
    assert_eq!(sea(&first).key, sea(&again).key);
}

#[test]
fn foam_follows_the_moving_bed() {
    let extra = r#"<whitewater emissionRate="1" threshold="0.5" maxParticles="100000" lifetime="2"/>"#;
    let frame = evaluator(&scene(10.0, extra)).evaluate(2.5);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let foam = sea(&frame).whitewater.as_ref().unwrap();
    assert!(!foam.particles.is_empty(), "the collapsing crater makes no foam");
}

/// Cost of reading the crater into the bed at every step on a 720 x 720 ocean
/// under the authored 1000 x 1000 seabed of 96 segments. Run on request:
/// `cargo test --release -p sr-eval --test ocean_coupling -- --ignored --nocapture`.
#[test]
#[ignore = "timing measurement"]
fn bed_sampling_cost_per_canonical_step_at_720_by_720() {
    let hero = |colliders: &str| {
        format!(
            r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
              <object3D id="seabed" primitive="plane" width="1000" height="1000" segments="96" y="12" rotationX="-90">
                <crater radius="52" depth="25" rimHeight="7" rimWidth="10" start="1" end="2.5"/>
              </object3D>
              <ocean id="sea" width="1440" depth="1440" cellSize="2" bottomDepth="12" dt="0.0416666666666667" boundary="open" order="2" maxMemoryMiB="512" surfaceMemoryMiB="256" maxWork="100000000000" {colliders}/>
            </composition></scene>"#
        )
    };
    let run = |colliders: &str| {
        let ev = evaluator(&hero(colliders));
        ev.evaluate(1.0);
        let start = std::time::Instant::now();
        for k in 25..=48 {
            let frame = ev.evaluate(k as f64 / 24.0);
            assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        }
        start.elapsed().as_secs_f64() / 24.0
    };
    let still = run("");
    let moving = run(r#"colliders="seabed""#);
    println!("BEDCOST per canonical step: static {still:.4} s, with the crater driving the bed {moving:.4} s; sampling adds {:.4} s", moving - still);
}

/// A sphere that falls into the ocean, with no authored impulse and no crater.
fn falling_sphere(radius: f64, speed_scale: f64) -> String {
    let (y0, y1) = (-40.0, 6.0);
    // The fall takes 1 s at scale 1; a larger scale is a faster body.
    let fall = 1.0 / speed_scale;
    format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
          <object3D id="impactor" primitive="sphere" radius="{radius}" segments="24">
            <animate property="y"><key time="0" value="{y0}"/><key time="{fall}" value="{y1}"/></animate>
          </object3D>
          <ocean id="sea" width="128" depth="128" cellSize="2" bottomDepth="12" dt="0.0416666666666667" boundary="closed" colliders="impactor"/>
        </composition></scene>"#
    )
}

#[test]
fn a_body_falling_into_the_water_makes_a_wave_with_no_impulse_and_conserves_the_water() {
    let ev = evaluator(&falling_sphere(7.0, 1.0));
    let before = ev.evaluate(0.3);
    assert!(before.problems.is_empty(), "{:?}", before.problems);
    // The sphere is still in the air: the water has not been touched.
    assert!(sea(&before).frame.cells.iter().all(|c| c.depth == 12.0 && c.velocity == [0.0; 2]));
    assert!(sea(&before).frame.bed.iter().all(|&y| y == 12.0));
    let after = ev.evaluate(2.0);
    assert!(after.problems.is_empty(), "{:?}", after.problems);
    assert!(peak(&after) > 0.02, "no wave: {}", peak(&after));
    let volume: f64 = sea(&after).frame.cells.iter().map(|c| c.depth).sum();
    assert!((volume - 12.0 * 4096.0).abs() < 1e-8 * volume, "water volume {volume}");
}

/// Largest change of water depth between 20 and 30 units from the centre, outside
/// the footprint of any body in these tests, over 1 s to 4.5 s (before a reflection
/// from the walls can return): only the radiated wave is there.
fn far_wave(ev: &Evaluator) -> f64 {
    (4..=18)
        .map(|k| {
            let frame = ev.evaluate(k as f64 * 0.25);
            assert!(frame.problems.is_empty(), "{:?}", frame.problems);
            let cells = &sea(&frame).frame.cells;
            (0..cells.len())
                .filter(|&i| {
                    let (x, z) = ((i % 64) as f64 * 2.0 + 1.0 - 64.0, (i / 64) as f64 * 2.0 + 1.0 - 64.0);
                    (20.0..30.0).contains(&x.hypot(z))
                })
                .map(|i| (cells[i].depth - 12.0).abs())
                .fold(0.0, f64::max)
        })
        .fold(0.0, f64::max)
}

#[test]
fn a_larger_body_makes_a_larger_wave() {
    let heights: Vec<f64> =
        [4.0, 7.0, 10.0].iter().map(|&radius| far_wave(&evaluator(&falling_sphere(radius, 1.0)))).collect();
    println!("BODY far-field wave for sphere radii 4, 7, 10: {heights:?}");
    assert!(heights[0] > 1e-3, "the wave never reached the ring: {heights:?}");
    assert!(heights[0] < heights[1] && heights[1] < heights[2], "{heights:?}");
}

/// The wave a body raises is set by the water it displaces, so it grows with the
/// entry speed only until the entry is quicker than the wave takes to cross the
/// body (about 1.3 s for this sphere). Measured 0.637, 0.661 and 0.660 for entry
/// speeds x0.5, x1 and x2: a slower entry gives a smaller wave, and a faster one
/// gives no more than the displaced volume allows.
#[test]
fn entry_speed_helps_the_wave_until_the_entry_is_faster_than_the_water_responds() {
    let heights: Vec<f64> =
        [0.5, 1.0, 2.0].iter().map(|&scale| far_wave(&evaluator(&falling_sphere(7.0, scale)))).collect();
    println!("BODY far-field wave for entry speeds x0.5, x1, x2: {heights:?}");
    assert!(heights[1] > 1.02 * heights[0], "{heights:?}");
    assert!(heights[2] > 0.97 * heights[1], "{heights:?}");
}

/// A half-submerged sphere sliding sideways drags the water with it.
fn sliding_sphere(speed: f64) -> String {
    format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
          <object3D id="barge" primitive="sphere" radius="6" segments="24" x="-40" y="4">
            <animate property="x"><key time="0" value="-40"/><key time="2" value="{}"/></animate>
          </object3D>
          <ocean id="sea" width="128" depth="128" cellSize="2" bottomDepth="12" dt="0.0416666666666667" boundary="closed" colliders="barge"/>
        </composition></scene>"#,
        -40.0 + 2.0 * speed
    )
}

#[test]
fn a_body_sliding_through_the_water_gives_it_momentum_along_its_motion() {
    let momentum: Vec<[f64; 2]> = [10.0, 20.0]
        .iter()
        .map(|&speed| {
            let frame = evaluator(&sliding_sphere(speed)).evaluate(1.5);
            assert!(frame.problems.is_empty(), "{:?}", frame.problems);
            let cells = &sea(&frame).frame.cells;
            [
                cells.iter().map(|c| c.depth * c.velocity[0]).sum::<f64>(),
                cells.iter().map(|c| c.depth * c.velocity[1]).sum::<f64>(),
            ]
        })
        .collect();
    println!("SLIDE water momentum for speeds 10 and 20: {momentum:?}");
    assert!(momentum[0][0] > 0.0 && momentum[1][0] > 1.3 * momentum[0][0], "{momentum:?}");
    // The body moves along x, so the sideways momentum is a small fraction of the forward one.
    assert!(momentum[1][1].abs() < 0.05 * momentum[1][0], "{momentum:?}");
}

/// The same fall driven by the rigid-body world instead of an animation.
fn falling_rigid_sphere() -> String {
    r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
      <object3D id="impactor" primitive="sphere" radius="7" segments="24" y="-40">
        <rigidBody velocityY="46" linearDamping="0" angularDamping="0"/>
      </object3D>
      <ocean id="sea" width="128" depth="128" cellSize="2" bottomDepth="12" dt="0.0416666666666667" boundary="closed" colliders="impactor"/>
    </composition><physics gravityY="0" bounds="none"/></scene>"#
        .to_string()
}

#[test]
fn a_body_of_the_rigid_world_moves_the_ocean_like_the_same_animated_body() {
    let rigid = evaluator(&falling_rigid_sphere());
    let before = rigid.evaluate(0.3);
    assert!(before.problems.is_empty(), "{:?}", before.problems);
    assert!(sea(&before).frame.cells.iter().all(|c| c.depth == 12.0));
    // The same straight fall at 46 units per second, through the bed and out of the water.
    let animated_xml = falling_rigid_sphere()
        .replace(
            r#"<rigidBody velocityY="46" linearDamping="0" angularDamping="0"/>"#,
            r#"<animate property="y"><key time="0" value="-40"/><key time="2" value="52"/></animate>"#,
        )
        .replace(r#"<physics gravityY="0" bounds="none"/>"#, "");
    let (animated, simulated) = (far_wave(&evaluator(&animated_xml)), far_wave(&rigid));
    println!("RIGID far-field wave: animated {animated:.4}, rigid world {simulated:.4}");
    assert!(simulated > 0.0 && (simulated - animated).abs() < 0.02 * animated, "{animated} vs {simulated}");
    // A backward seek replays the world and the water.
    let first = rigid.evaluate(2.0);
    rigid.evaluate(0.5);
    let again = rigid.evaluate(2.0);
    assert_eq!(sea(&first).frame, sea(&again).frame);
}

#[test]
fn colliders_that_are_neither_bed_nor_body_are_rejected_when_the_scene_loads() {
    let floor = r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
      <object3D id="floor" primitive="plane" width="100" height="100" y="12" rotationX="-90"/>
      <ocean id="sea" width="64" depth="64" cellSize="2" bottomDepth="12" colliders="floor"/>
    </composition></scene>"#;
    let Err(error) = sr_model::load_str(floor, &sr_model::LoadOptions::without_assets()) else {
        panic!("a plane without a crater was accepted");
    };
    assert!(format!("{error:?}").contains("OCN6"), "{error:?}");
}

/// Wall time of the ocean stage (solver, foam and surface) for a forward seek and a
/// backward one on the target-resolution ocean of the hero scene, alone, with and
/// without the cratered seabed driving its bed. Run on request:
/// `cargo test --release -p sr-eval --test ocean_coupling -- --ignored --nocapture ocean_replay`.
#[test]
#[ignore = "timing measurement"]
fn ocean_replay_seconds_at_target_resolution() {
    let full = include_str!("../../../examples/cinematic-impact/hero-hires.scene.xml");
    let a = full.find("<ocean").unwrap();
    let ocean = &full[a..full.find("</ocean>").unwrap() + "</ocean>".len()];
    let seabed = &full[full.find("<object3D id=\"seabed\"").unwrap()..];
    let seabed = &seabed[..seabed.find("</object3D>").unwrap() + "</object3D>".len()];
    let no_foam = {
        let w = ocean.find("<whitewater").unwrap();
        format!("{}{}", &ocean[..w], &ocean[ocean[w..].find("/>").unwrap() + w + 2..])
    };
    for (name, colliders, foam) in [
        ("static bed", "", true),
        ("static bed, no foam", "", false),
        ("cratered seabed", " colliders=\"seabed\"", true),
    ] {
        let ocean = if foam { ocean } else { no_foam.as_str() };
        let ocean = ocean.replacen("<ocean id=\"sea\"", &format!("<ocean id=\"sea\"{colliders}"), 1);
        let xml = format!(
            r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><materials><material id="water" baseColor="#06212C"/><material id="bed" baseColor="#756653"/></materials><composition>{seabed}{ocean}</composition></scene>"##
        );
        let ev = evaluator(&xml);
        let seconds = |t: f64| {
            let frame = ev.evaluate(t);
            assert!(frame.problems.is_empty(), "{:?}", frame.problems);
            frame.sim_seconds.ocean
        };
        let forward: f64 = (1..=72).map(|k| seconds(k as f64 / 24.0)).sum();
        let back = seconds(1.5);
        let again = seconds(3.0);
        println!(
            "REPLAY {name}: 72 sequential frames to 3.0 s took {forward:.1} s in the ocean stage; a backward seek to 1.5 s {back:.2} s; forward to 3.0 s again {again:.2} s"
        );
    }
}

#[test]
fn whitewater_checkpoint_budget_never_changes_the_result_of_a_replay() {
    let scene = |budget: &str| {
        format!(
            r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
              <ocean id="sea" width="64" depth="64" cellSize="1" bottomDepth="6" dt="0.0416666666666667">
                <waterImpulse time="0.2" radius="8" amplitude="-4"/>
                <whitewater emissionRate="20" threshold="0.05" maxParticles="100000" lifetime="2" {budget}/>
              </ocean></composition></scene>"#
        )
    };
    let run = |budget: &str| {
        let ev = evaluator(&scene(budget));
        let first = ev.evaluate(3.0);
        assert!(first.problems.is_empty(), "{:?}", first.problems);
        ev.evaluate(1.0);
        let again = ev.evaluate(3.0);
        assert!(again.problems.is_empty(), "{:?}", again.problems);
        let foam = sea(&again).whitewater.clone().unwrap();
        assert_eq!(sea(&first).whitewater.as_ref(), Some(&foam));
        (foam, sea(&again).key)
    };
    let (default, key) = run("");
    assert!(!default.particles.is_empty(), "the test needs foam to mean anything");
    assert_eq!(run(r#"checkpointMemoryMiB="0""#), (default.clone(), key));
    assert_eq!(run(r#"checkpointMemoryMiB="1""#), (default, key));
}
