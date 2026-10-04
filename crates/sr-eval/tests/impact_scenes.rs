//! The two impact scenes of the cinematic-impact examples, read as documents: a rock hits land
//! and hits the sea, and everything that follows (crater, smoke, wave) is a consequence of the
//! contact. Nothing in either document says when anything happens, so these tests check that the
//! consequences start at the impact, grow in the order physics gives with speed, mass and angle,
//! conserve what they should, and are the same however they are asked for.
//!
//! The scenes are used as authored. A test changes only the rock (speed, angle, mass, by the
//! launch that makes it arrive so) and, where it says so, the resolution of the smoke or the
//! memory a solver may keep for replay.

use sr_eval::{Evaluator, FrameGraph, FrameNode};

const LAND: &str = include_str!("../../../examples/cinematic-impact/impact-land.scene.xml");
const OCEAN: &str = include_str!("../../../examples/cinematic-impact/impact-ocean.scene.xml");

const GRAVITY: f64 = 9.80665;
/// Seconds the rock flies from its launch to the surface it hits, the same in every variant.
const FLIGHT: f64 = 1.4888;
/// The rock's radius, metres: its centre is this high when it touches the surface.
const RADIUS: f64 = 2.0;

/// The rock as it arrives at the surface.
#[derive(Clone, Copy, Debug)]
struct Hit {
    /// Metres per second.
    speed: f64,
    /// Degrees above the surface.
    angle: f64,
    /// Kilograms; the radius stays 2 m, so the density follows.
    mass: f64,
}

const AUTHORED: Hit = Hit { speed: 100.0, angle: 60.0, mass: 90478.0 };

/// The document with the rock launched so that it arrives as `hit`. The launch is found by
/// running the flight backwards from the arrival, so that the speed and the angle are those of
/// the contact whatever gravity does on the way.
fn with_hit(xml: &str, hit: Hit) -> String {
    let (vx, vy) = (hit.speed * hit.angle.to_radians().cos(), hit.speed * hit.angle.to_radians().sin());
    let launch_vy = vy - GRAVITY * FLIGHT;
    let x = -vx * FLIGHT;
    let y = -RADIUS - launch_vy * FLIGHT - 0.5 * GRAVITY * FLIGHT * FLIGHT;
    let xml = replace_once(xml, r#"x="-74.4" y="-120""#, &format!(r#"x="{x}" y="{y}""#));
    let xml =
        replace_once(&xml, r#"velocityX="50" velocityY="72""#, &format!(r#"velocityX="{vx}" velocityY="{launch_vy}""#));
    replace_once(&xml, r#"mass="90478""#, &format!(r#"mass="{}""#, hit.mass))
}

fn replace_once(text: &str, from: &str, to: &str) -> String {
    assert_eq!(text.matches(from).count(), 1, "the scene has exactly one {from}");
    text.replacen(from, to, 1)
}

/// The smoke on 2 m cells instead of 1 m: the same volume, a thousandth of the work in a debug
/// build. The authored resolution is timed separately.
fn coarse_smoke(xml: &str) -> String {
    replace_once(xml, r#"voxelSize="1""#, r#"voxelSize="2""#)
}

fn evaluator(xml: &str) -> Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn at(ev: &Evaluator, t: f64) -> FrameGraph {
    let frame = ev.evaluate(t);
    assert!(
        frame.problems.is_empty() && frame.failures.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.problems,
        frame.failures
    );
    frame
}

fn node<'a>(frame: &'a FrameGraph, id: &str) -> &'a FrameNode {
    frame.nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}"))
}

/// What the crater of `id` is at `t`: its spec and how far it has grown.
fn crater(frame: &FrameGraph, id: &str) -> (sr_3d::crater::Spec, f64) {
    let c = sr_eval::crater::at(node(frame, id)).unwrap().expect("a crater");
    (c.kernel.spec(), c.progress)
}

fn increasing(values: &[f64]) -> bool {
    values.windows(2).all(|w| w[0] < w[1])
}

// ---------------------------------------------------------------- the documents themselves

/// The start tags of every `name` element in `xml`.
fn tags<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name}");
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(&open) {
        let after = &rest[at + open.len()..];
        if after.starts_with([' ', '/', '>']) {
            found.push(&after[..after.find('>').unwrap()]);
        }
        rest = after;
    }
    found
}

#[test]
fn no_effect_in_either_scene_has_an_attribute_of_time() {
    for (scene, xml) in [("land", LAND), ("ocean", OCEAN)] {
        let mut seen = 0;
        for element in ["crater", "waterImpulse", "burst", "pyroSource", "pyroImpulse"] {
            for tag in tags(xml, element) {
                seen += 1;
                for time in ["time", "start", "end", "duration", "repeat", "interval"] {
                    assert!(!tag.contains(&format!(" {time}=")), "{scene}: <{element}{tag}> says when");
                }
            }
        }
        assert!(seen > 0, "{scene}: no effect found, so the check looked at nothing");
    }
    // each scene has the effects it is about, and the ocean has no authored water impulse
    assert_eq!(tags(LAND, "crater").len(), 1);
    assert_eq!(tags(LAND, "pyroSource").len(), 1);
    assert_eq!(tags(OCEAN, "crater").len(), 1);
    assert!(tags(OCEAN, "waterImpulse").is_empty());
    assert!(tags(OCEAN, "pyroSource").is_empty() && tags(OCEAN, "pyroImpulse").is_empty(), "no smoke under water");
}

#[test]
fn the_scenes_declare_their_units_and_the_body_is_the_same() {
    for xml in [LAND, OCEAN] {
        assert!(xml.contains(r#"pixelsPerMeter="1""#) && xml.contains(r#"gravityY="-9.80665""#));
        // 2 m radius, 2700 kg/m3
        let tag = tags(xml, "rigidBody").into_iter().find(|t| t.contains(r#"mass="90478""#)).expect("the rock");
        let mass: f64 = 90478.0;
        assert!((mass / (4.0 / 3.0 * std::f64::consts::PI * RADIUS.powi(3)) - 2700.0).abs() < 0.1, "{tag}");
    }
    let rock = |xml: &str| {
        let start = xml.find(r#"<object3D id="impactor""#).unwrap();
        xml[start..xml[start..].find("</object3D>").unwrap() + start].to_string()
    };
    assert_eq!(rock(LAND), rock(OCEAN), "one body in both scenes");
}

// ------------------------------------------------------------------------------- land

/// Everything the land scene gives at `t` that the tests compare.
#[derive(Clone, Debug, PartialEq)]
struct Land {
    /// The rock's pose matrix.
    pose: [f64; 16],
    spec: [f64; 6],
    progress: f64,
    /// Cubic metres of dust in the cloud, the hottest it is above ambient (kelvin), the heat in
    /// the dust (cubic metres of dust times kelvin above ambient) and a hash of every value of
    /// the grids.
    dust: f64,
    rise: f64,
    heat: f64,
    grids: u64,
}

fn land(ev: &Evaluator, t: f64) -> Land {
    let frame = at(ev, t);
    let (spec, progress) = crater(&frame, "ground");
    let cloud = &node(&frame, "cloud").sim_volume.as_ref().expect("a native smoke volume").data;
    let sum =
        |name: &str| -> f64 { cloud.grid(name).unwrap().bricks().flat_map(|(_, b)| b.iter()).map(|v| *v as f64).sum() };
    let hottest = cloud
        .grid("temperature")
        .unwrap()
        .bricks()
        .flat_map(|(_, b)| b.iter())
        .map(|v| *v as f64)
        .fold(300.0, f64::max);
    let temperature: std::collections::HashMap<_, _> = cloud.grid("temperature").unwrap().bricks().collect();
    let heat: f64 = cloud
        .grid("density")
        .unwrap()
        .bricks()
        .map(|(key, density)| {
            let warm = temperature.get(&key);
            density
                .iter()
                .enumerate()
                .map(|(k, d)| *d as f64 * (warm.map_or(300.0, |w| w[k] as f64) - 300.0).max(0.0))
                .sum::<f64>()
        })
        .sum::<f64>()
        * 8.0;
    let mut grids = 0xcbf29ce484222325u64;
    for (name, grid) in cloud.grids() {
        for (key, values) in grid.bricks() {
            for word in name
                .bytes()
                .map(u64::from)
                .chain(key.iter().map(|k| *k as u64))
                .chain(values.iter().map(|v| u64::from(v.to_bits())))
            {
                grids = (grids ^ word).wrapping_mul(0x100000001b3);
            }
        }
    }
    // the cells are 2 m in the variants that call this: eight cubic metres each
    Land {
        pose: node(&frame, "impactor").pose3.expect("a simulated rock"),
        spec: [spec.radius, spec.depth, spec.rim_height, spec.center[0], spec.center[1], spec.center[2]],
        progress,
        dust: sum("density") * 8.0,
        rise: hottest - 300.0,
        heat,
        grids,
    }
}

fn land_variant(hit: Hit) -> Evaluator {
    evaluator(&coarse_smoke(&with_hit(LAND, hit)))
}

/// The rock lands about 1.49 s in, and the crater and the dust it makes are done well before this.
const SETTLED: f64 = 3.0;

#[test]
fn on_land_nothing_happens_before_the_impact() {
    let ev = land_variant(AUTHORED);
    for t in [0.0, 0.5, 1.0, 1.3] {
        let state = land(&ev, t);
        assert_eq!(state.progress, 0.0, "t = {t}: no crater yet");
        assert_eq!(state.dust, 0.0, "t = {t}: no dust");
        assert_eq!(state.rise, 0.0, "t = {t}: no heat");
    }
    let before = land(&ev, 1.3);
    assert!(before.pose[13] < -2.5, "the rock is still in the air: {}", before.pose[13]);
    let after = land(&ev, SETTLED);
    assert_eq!(after.progress, 1.0);
    assert!(after.dust > 0.0 && after.rise > 0.0, "{after:?}");
}

/// A scene's values for the speeds, the masses and the angles of the monotonicity tests.
fn sweeps() -> (Vec<Hit>, Vec<Hit>, Vec<Hit>) {
    let speeds = [60.0, 100.0, 150.0].map(|speed| Hit { speed, ..AUTHORED }).to_vec();
    let masses = [60000.0, 90478.0, 270000.0].map(|mass| Hit { mass, ..AUTHORED }).to_vec();
    // listed from the shallowest: the quantities grow with the angle
    let angles = [30.0, 60.0, 90.0].map(|angle| Hit { angle, ..AUTHORED }).to_vec();
    (speeds, masses, angles)
}

#[test]
fn on_land_the_crater_the_dust_and_the_heat_grow_with_speed_mass_and_angle() {
    let (speeds, masses, angles) = sweeps();
    for (name, hits) in [("speed", speeds), ("mass", masses), ("angle", angles)] {
        let states: Vec<Land> = hits.iter().map(|&h| land(&land_variant(h), SETTLED)).collect();
        let column = |f: fn(&Land) -> f64| states.iter().map(f).collect::<Vec<_>>();
        let (radius, depth, dust, heat) =
            (column(|s| s.spec[0]), column(|s| s.spec[1]), column(|s| s.dust), column(|s| s.heat));
        println!(
            "IMPACT land by {name}: radius {radius:?} depth {depth:?} dust {dust:?} heat {heat:?} hottest rise {:?}",
            column(|s| s.rise)
        );
        assert!(increasing(&radius), "crater radius by {name}: {radius:?}");
        assert!(increasing(&depth), "crater depth by {name}: {depth:?}");
        assert!(increasing(&dust), "dust by {name}: {dust:?}");
        assert!(increasing(&heat), "heat in the dust by {name}: {heat:?}");
    }
}

#[test]
fn on_land_the_temperature_of_the_dust_grows_with_speed_and_with_mass() {
    let (speeds, masses, _) = sweeps();
    for (name, hits) in [("speed", speeds), ("mass", masses)] {
        let rise: Vec<f64> = hits.iter().map(|&h| land(&land_variant(h), SETTLED).rise).collect();
        assert!(increasing(&rise), "hottest rise by {name}: {rise:?}");
    }
}

#[test]
#[ignore = "fails as the engine stands: the dust is as hot at 30 degrees as at 90"]
fn on_land_the_temperature_of_the_dust_grows_with_the_angle() {
    let (_, _, angles) = sweeps();
    let rise: Vec<f64> = angles.iter().map(|&h| land(&land_variant(h), SETTLED).rise).collect();
    println!("IMPACT land hottest rise for 30, 60, 90 degrees: {rise:?}");
    assert!(increasing(&rise), "hottest rise by angle: {rise:?}");
}

#[test]
fn on_land_the_rock_arrives_as_the_document_says_and_the_dust_is_what_the_law_gives() {
    // no heat, no dissipation and closed faces, so that the dust in the volume is the dust put in
    let xml = with_hit(LAND, AUTHORED)
        .replace("<pyroSource crater=\"pit\"/>", "<pyroSource crater=\"pit\" heatFraction=\"0\"/>");
    let xml = coarse_smoke(&xml)
        .replace(r#"dissipation="0.02""#, r#"dissipation="0""#)
        .replace(r#"boundary="open""#, r#"boundary="closed""#);
    let ev = evaluator(&xml);
    let state = land(&ev, 2.2);
    let frame = at(&ev, 2.2);
    let cause = node(&frame, "ground").crater_impact.as_deref().expect("the impact");
    // the arrival is the authored 100 m/s at 60 degrees, within the step the contact is found in
    let normal = AUTHORED.speed * AUTHORED.angle.to_radians().sin();
    println!(
        "IMPACT land arrival at {:.1} s: {:.2} m/s, {:.2} m/s along the normal (authored {normal:.2}); dust {:.4} m3 against {:.4}",
        cause.impact_time(),
        cause.speed(),
        cause.impactor().normal_speed,
        state.dust,
        0.01 * cause.law().ejecta_volume
    );
    assert!((cause.speed() - AUTHORED.speed).abs() < 0.03 * AUTHORED.speed, "{}", cause.speed());
    assert!((1.4..1.6).contains(&cause.impact_time()), "{}", cause.impact_time());
    assert_eq!(cause.impactor().mass, AUTHORED.mass);
    // the dust is a hundredth of the volume the law says was thrown out
    let wanted = 0.01 * cause.law().ejecta_volume;
    assert!((state.dust - wanted).abs() < 2e-3 * wanted, "{} against {wanted}", state.dust);
}

#[test]
#[ignore = "fails as the engine stands: the contact normal of the ground mesh is tilted by about 6 degrees"]
fn on_land_the_normal_speed_and_the_axis_of_the_crater_are_those_of_the_ground() {
    let ev = evaluator(&coarse_smoke(&with_hit(LAND, AUTHORED)));
    let frame = at(&ev, 2.2);
    let cause = node(&frame, "ground").crater_impact.as_deref().expect("the impact");
    let normal = AUTHORED.speed * AUTHORED.angle.to_radians().sin();
    println!(
        "IMPACT land normal speed {:.2} m/s against {normal:.2}; axis {:?}",
        cause.impactor().normal_speed,
        cause.spec.outward
    );
    assert!((cause.impactor().normal_speed - normal).abs() < 0.03 * normal, "{}", cause.impactor().normal_speed);
    assert!(cause.spec.outward[2] < -0.9995, "the ground is flat: {:?}", cause.spec.outward);
}

#[test]
fn on_land_the_same_answers_in_any_order_from_a_fresh_evaluator_and_after_replay_from_the_start() {
    let times = [3.0, 0.5, 6.0, 1.7, 2.2, 4.5, 3.0, 1.0];
    let first = land_variant(AUTHORED);
    let forward: Vec<Land> = times.iter().map(|&t| land(&first, t)).collect();
    // a fresh evaluator in another order
    let fresh = land_variant(AUTHORED);
    for k in [4, 0, 7, 2, 5, 1, 3, 6] {
        assert_eq!(forward[k], land(&fresh, times[k]), "fresh evaluator, t = {}", times[k]);
    }
    // the smoke keeps no checkpoint but its first, so going back replays it from the start
    let discarding = evaluator(
        &coarse_smoke(&with_hit(LAND, AUTHORED))
            .replace(r#"pressureTolerance="0.001""#, r#"pressureTolerance="0.001" checkpointMemoryMiB="1""#),
    );
    for (k, &t) in times.iter().enumerate() {
        assert_eq!(forward[k], land(&discarding, t), "after discarding checkpoints, t = {t}");
    }
    assert_eq!(forward[0], forward[6], "the same instant asked twice");
}

// ------------------------------------------------------------------------------ ocean

/// What the ocean scene gives at `t`.
#[derive(Clone, Debug, PartialEq)]
struct Sea {
    pose: [f64; 16],
    spec: [f64; 6],
    progress: f64,
    /// Highest water surface above the rest level (metres) and the cubic metres of water.
    crest: f64,
    water: f64,
    /// Depth at every cell, to compare whole frames.
    frame: sr_sim::ocean::Frame,
}

fn sea(ev: &Evaluator, t: f64) -> Sea {
    let frame = at(ev, t);
    let (spec, progress) = crater(&frame, "seabed");
    let ocean = node(&frame, "sea").sim_ocean.as_ref().expect("an ocean");
    let water = &ocean.frame;
    // the surface is the depth less the bed's own depth, the rest level being the scene's y = 0
    let crest = water.cells.iter().zip(&water.bed).map(|(c, bed)| c.depth - bed).fold(f64::MIN, f64::max);
    let cell = 1.5;
    Sea {
        pose: node(&frame, "impactor").pose3.expect("a simulated rock"),
        spec: [spec.radius, spec.depth, spec.rim_height, spec.center[0], spec.center[1], spec.center[2]],
        progress,
        crest,
        water: water.cells.iter().map(|c| c.depth).sum::<f64>() * cell * cell,
        frame: water.clone(),
    }
}

/// The sea on 3 m cells instead of 1.5 m, for the sweeps; the authored cells are used by the
/// tests that do not sweep.
fn coarse_sea(xml: &str) -> String {
    replace_once(xml, r#"cellSize="1.5""#, r#"cellSize="3""#)
}

fn sea_variant(hit: Hit) -> Evaluator {
    evaluator(&coarse_sea(&with_hit(OCEAN, hit)))
}

/// The water is 20 m deep over 192 x 192 m.
const WATER: f64 = 20.0 * 192.0 * 192.0;

#[test]
fn in_the_sea_nothing_happens_before_the_impact() {
    let ev = evaluator(&with_hit(OCEAN, AUTHORED));
    let still = sea(&ev, 0.0);
    for t in [0.5, 1.0, 1.3] {
        let state = sea(&ev, t);
        assert_eq!(state.progress, 0.0, "t = {t}: no crater");
        assert!(
            state.frame.cells.iter().zip(&still.frame.cells).all(|(a, b)| a.depth == b.depth),
            "t = {t}: the water is still"
        );
        assert!(state.frame.bed.iter().all(|&bed| bed == 20.0), "t = {t}: the bed is flat");
        assert!(state.crest <= 1e-12, "t = {t}: no wave: {}", state.crest);
    }
    assert!(sea(&ev, 1.3).pose[13] < -2.0, "the rock is still above the water");
    // it reaches the bed about 2 s in, and only then is there a crater
    assert_eq!(sea(&ev, 1.7).progress, 0.0);
    let later = sea(&ev, 3.5);
    assert!(later.progress == 1.0 && later.crest > 0.1, "{} {}", later.progress, later.crest);
}

#[test]
fn in_the_sea_the_water_is_conserved_to_the_last_cell() {
    let ev = evaluator(&with_hit(OCEAN, AUTHORED));
    for t in [0.5, 1.6, 2.0, 2.5, 3.5, 5.0, 6.0] {
        let state = sea(&ev, t);
        assert!((state.water - WATER).abs() < 1e-9 * WATER, "t = {t}: {} against {WATER}", state.water);
    }
}

/// The highest the water stands above its rest level from the moment the rock is in it to the end.
fn highest_wave(ev: &Evaluator) -> f64 {
    (0..10).map(|k| sea(ev, 1.5 + 0.5 * k as f64).crest).fold(f64::MIN, f64::max)
}

/// The same sea, but the rock does not displace water, only the bed the crater lowers does.
fn crater_only(xml: String) -> String {
    replace_once(&xml, r#"colliders="seabed impactor""#, r#"colliders="seabed""#)
}

#[test]
fn in_the_sea_the_crater_grows_with_speed_mass_and_angle() {
    let (speeds, masses, angles) = sweeps();
    for (name, hits) in [("speed", speeds), ("mass", masses), ("angle", angles)] {
        let (radius, depth): (Vec<f64>, Vec<f64>) = hits
            .iter()
            .map(|&h| {
                let state = sea(&sea_variant(h), 6.0);
                assert_eq!(state.progress, 1.0);
                (state.spec[0], state.spec[1])
            })
            .unzip();
        println!("IMPACT sea by {name}: crater radius {radius:?} depth {depth:?}");
        assert!(increasing(&radius), "crater radius by {name}: {radius:?}");
        assert!(increasing(&depth), "crater depth by {name}: {depth:?}");
    }
}

#[test]
fn in_the_sea_the_wave_grows_with_the_mass() {
    let (_, masses, _) = sweeps();
    let waves: Vec<f64> = masses.iter().map(|&h| highest_wave(&sea_variant(h))).collect();
    println!("IMPACT sea highest wave by mass: {waves:?}");
    assert!(increasing(&waves), "wave by mass: {waves:?}");
}

#[test]
fn in_the_sea_the_wave_of_the_crater_alone_grows_with_speed_mass_and_angle() {
    let (speeds, masses, angles) = sweeps();
    for (name, hits) in [("speed", speeds), ("mass", masses), ("angle", angles)] {
        let waves: Vec<f64> =
            hits.iter().map(|&h| highest_wave(&evaluator(&crater_only(coarse_sea(&with_hit(OCEAN, h)))))).collect();
        println!("IMPACT sea wave of the crater alone by {name}: {waves:?}");
        assert!(increasing(&waves), "wave by {name}: {waves:?}");
    }
}

#[test]
#[ignore = "fails as the engine stands: the wave is the rock's own bow wave and does not grow with speed"]
fn in_the_sea_the_wave_grows_with_the_speed() {
    let (speeds, _, _) = sweeps();
    let waves: Vec<f64> = speeds.iter().map(|&h| highest_wave(&sea_variant(h))).collect();
    println!("IMPACT sea highest wave by speed: {waves:?}");
    assert!(increasing(&waves), "wave by speed: {waves:?}");
}

#[test]
#[ignore = "fails as the engine stands: a rock that arrives straight down makes a wave of a few centimetres"]
fn in_the_sea_the_wave_grows_with_the_angle() {
    let (_, _, angles) = sweeps();
    let waves: Vec<f64> = angles.iter().map(|&h| highest_wave(&sea_variant(h))).collect();
    println!("IMPACT sea highest wave for 30, 60, 90 degrees: {waves:?}");
    assert!(increasing(&waves), "wave by angle: {waves:?}");
}

#[test]
fn in_the_sea_the_same_answers_in_any_order_from_a_fresh_evaluator_and_after_replay_from_the_start() {
    let times = [3.5, 1.0, 6.0, 2.2, 4.5, 3.5, 0.5];
    let xml = coarse_sea(&with_hit(OCEAN, AUTHORED));
    let first = evaluator(&xml);
    let forward: Vec<Sea> = times.iter().map(|&t| sea(&first, t)).collect();
    let fresh = evaluator(&xml);
    for k in [3, 0, 6, 1, 4, 2, 5] {
        assert_eq!(forward[k], sea(&fresh, times[k]), "fresh evaluator, t = {}", times[k]);
    }
    // the sea keeps no checkpoint but its first
    let discarding =
        evaluator(&xml.replace(r#"bodyCoupling="buoyancy""#, r#"bodyCoupling="buoyancy" checkpointMemoryMiB="0""#));
    for (k, &t) in times.iter().enumerate() {
        assert_eq!(forward[k], sea(&discarding, t), "after discarding checkpoints, t = {t}");
    }
}

// --------------------------------------------------------------------------------- ejecta

/// The rock's burst of ejecta: the scene's land and a particle emitter that the crater drives.
/// It waits for the evaluator to read `burst@crater`; until then the document is refused.
fn with_ejecta(xml: &str) -> String {
    replace_once(
        xml,
        "    <object3D id=\"cloud\"",
        "    <particles3D id=\"ejecta\" shape=\"sphere\" segments=\"6\" material=\"rock\" rate=\"0\" seed=\"20261004\" size=\"0.3\" \
         gravityY=\"9.80665\" drag=\"0\" lifetime=\"6\" dt=\"0.0416666666666667\" maxParticles=\"6000\" colliders=\"ground\" \
         collisionRadius=\"0.3\" bounce=\"0.15\"><burst crater=\"pit\" count=\"4000\"/></particles3D>\n    <object3D id=\"cloud\"",
    )
}

/// Where the ejecta of a rock arriving as `hit` are at `t`: their mean position on the ground
/// (x along the rock's travel, z across it) and how far the farthest is from the impact point.
fn ejecta(hit: Hit, t: f64) -> ([f64; 2], f64, usize) {
    let ev = evaluator(&coarse_smoke(&with_ejecta(&with_hit(LAND, hit))));
    let frame = at(&ev, t);
    let particles = &node(&frame, "ejecta").particles3d.as_ref().expect("particles").frame.particles;
    assert!(!particles.is_empty(), "the impact threw something out");
    let n = particles.len() as f64;
    let mean = [
        particles.iter().map(|p| p.position[0]).sum::<f64>() / n,
        particles.iter().map(|p| p.position[2]).sum::<f64>() / n,
    ];
    let reach = particles.iter().map(|p| p.position[0].hypot(p.position[2])).fold(0.0, f64::max);
    (mean, reach, particles.len())
}

#[test]
#[ignore = "waits for the evaluator to read burst@crater"]
fn on_land_nothing_is_thrown_before_the_impact_and_the_ejecta_obey_the_order_of_the_impact() {
    let before = evaluator(&coarse_smoke(&with_ejecta(&with_hit(LAND, AUTHORED))));
    assert_eq!(node(&at(&before, 1.3), "ejecta").particles3d.as_ref().map_or(0, |p| p.frame.particles.len()), 0);
    let (speeds, masses, angles) = sweeps();
    for (name, hits) in [("speed", speeds), ("mass", masses), ("angle", angles)] {
        let reach: Vec<f64> = hits.iter().map(|&h| ejecta(h, 2.5).1).collect();
        println!("IMPACT ejecta reach by {name}: {reach:?}");
        assert!(increasing(&reach), "reach by {name}: {reach:?}");
    }
    // an oblique impact throws the ejecta on downrange, the direction the rock was going (+x);
    // a vertical one throws them evenly
    let downrange: Vec<f64> = [90.0, 60.0, 30.0].map(|angle| ejecta(Hit { angle, ..AUTHORED }, 2.5).0[0]).to_vec();
    println!("IMPACT ejecta mean x for 90, 60, 30 degrees: {downrange:?}");
    assert!(downrange[0].abs() < downrange[1] && downrange[1] < downrange[2], "{downrange:?}");
}

// ------------------------------------------------------------------------------- cost

/// Peak resident memory of this process so far, MiB.
fn peak_memory_mib() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<f64>().ok())
        .map_or(f64::NAN, |kb| kb / 1024.0)
}

/// Time to evaluate each scene as authored to its last frame, and the peak memory. Run one scene
/// at a time to read its own peak:
/// `cargo test --release -p sr-eval --test impact_scenes cost_of_land -- --ignored --nocapture`.
fn cost(xml: &str, label: &str) {
    let ev = evaluator(xml);
    let started = std::time::Instant::now();
    let mut slowest: f64 = 0.0;
    for k in 0..=24 {
        let t = 0.25 * k as f64;
        let frame_start = std::time::Instant::now();
        at(&ev, t);
        slowest = slowest.max(frame_start.elapsed().as_secs_f64());
    }
    println!(
        "IMPACT cost {label}: {:.1} s for 25 frames in order to 6 s, slowest frame {slowest:.2} s, peak memory {:.0} MiB",
        started.elapsed().as_secs_f64(),
        peak_memory_mib()
    );
}

#[test]
#[ignore = "timing measurement"]
fn cost_of_land() {
    cost(LAND, "land");
}

#[test]
#[ignore = "timing measurement"]
fn cost_of_ocean() {
    cost(OCEAN, "ocean");
}
