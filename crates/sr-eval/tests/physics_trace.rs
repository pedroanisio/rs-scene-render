//! The physics trace: velocities and contacts of the 3D rigid bodies, the same from a live
//! simulation and from a baked cache, and a cache that belongs to another document is an
//! error rather than a silent re-simulation.

use sr_eval::Evaluator;

const BALL: &str = r##"<object3D id="ball" primitive="sphere" radius="10" y="-120">
    <rigidBody shape="sphere" mass="2" velocityX="30" velocityY="60" restitution="0.2" linearDamping="0" angularDamping="0"/></object3D>"##;
const FLOOR: &str = r##"<object3D id="floor" primitive="box" width="600" height="20" depth="600" y="60">
    <rigidBody type="static" shape="box" friction="0.5"/></object3D>"##;

fn scene(bodies: &str, physics: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="2"/>
        <composition>{bodies}</composition>{physics}</scene>"##
    )
}

fn physics(extra: &str) -> String {
    format!(r#"<physics gravityY="-9.80665" pixelsPerMeter="100" fixedStep="0.01" {extra}/>"#)
}

fn evaluator(xml: &str) -> Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    Evaluator::new(&doc, &Default::default()).unwrap()
}

struct TempFile(std::path::PathBuf);
impl TempFile {
    fn new(name: &str, bytes: &[u8]) -> TempFile {
        let path = std::env::temp_dir().join(format!("sr-trace-{}-{name}.bin", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        TempFile(path)
    }
}
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn baked(xml_without_cache: &str, bytes: &[u8], name: &str) -> (Evaluator, TempFile) {
    let file = TempFile::new(name, bytes);
    let xml = xml_without_cache.replace("<physics ", &format!("<physics cache=\"{}\" ", file.0.display()));
    (evaluator(&xml), file)
}

#[test]
fn the_trace_of_a_live_simulation_has_the_impact() {
    let ev = evaluator(&scene(&format!("{BALL}{FLOOR}"), &physics("")));
    let trace = ev.physics_trace().unwrap();
    assert_eq!(trace.step, 0.01);
    assert_eq!(trace.velocities.len() as u64, trace.frames);
    assert!(trace.velocities.iter().all(|v| v.len() == 2), "one entry per 3D body");
    // the ball falls at 60 + g t scene units a second, so it lands with speed ~ 60 + 981 t
    let first = trace.contacts.first().expect("the ball lands on the floor");
    assert_eq!(first.bodies, [Some(0), Some(1)]);
    assert!(first.normal[1] > 0.99, "from the ball down onto the floor: {:?}", first.normal);
    let closing = -first.relative_velocity[1];
    assert!(closing > 100.0, "{closing}");
    assert!(first.impulse > 2.0 * closing * 0.8, "impulse {} for closing speed {closing}", first.impulse);
    assert!(trace.contacts.windows(2).all(|w| w[0].step <= w[1].step));
}

#[test]
fn a_baked_cache_gives_the_same_trace_as_the_live_simulation() {
    let xml = scene(&format!("{BALL}{FLOOR}"), &physics(""));
    let live = evaluator(&xml);
    let want = live.physics_trace().unwrap();
    assert!(!want.contacts.is_empty());
    let bytes = live.physics_cache().unwrap();
    assert_eq!(&bytes[..8], b"SRPHYS04");
    let (cached, _file) = baked(&xml, &bytes, "same");
    let got = cached.physics_trace().unwrap();
    assert_eq!(got.contacts, want.contacts, "live = cache");
    assert_eq!(got.velocities, want.velocities);
    assert_eq!((got.start, got.step, got.frames), (want.start, want.step, want.frames));
    // and the poses come from the same file
    for t in [0.0, 0.5, 1.3, 2.0] {
        assert_eq!(cached.evaluate(t).nodes.len(), live.evaluate(t).nodes.len());
    }
}

#[test]
fn a_cache_of_another_document_is_an_error_and_nothing_is_simulated() {
    let original = scene(&format!("{BALL}{FLOOR}"), &physics(""));
    let bytes = evaluator(&original).physics_cache().unwrap();
    let heavier = original.replace("mass=\"2\"", "mass=\"3\"");
    let (stale, _file) = baked(&heavier, &bytes, "stale");
    let error = stale.physics_trace().unwrap_err();
    assert!(error.contains("digest"), "{error}");
    let frame = stale.evaluate(1.0);
    assert!(frame.failures.iter().any(|f| f.contains("digest")), "{:?}", frame.failures);
    assert!(frame.problems.iter().all(|p| !p.contains("simulating")), "{:?}", frame.problems);
}

#[test]
fn every_input_of_the_simulation_is_part_of_the_identity() {
    let animated = |keys: &str| {
        scene(
            &format!(
                r##"{BALL}<object3D id="platform" primitive="box" width="200" height="10" depth="200" y="40">
                <animate property="y" timeBase="composition">{keys}</animate>
                <rigidBody type="kinematic" shape="box"/></object3D>"##
            ),
            &physics(""),
        )
    };
    let original = animated(r#"<key time="0" value="40"/><key time="2" value="10"/>"#);
    let bytes = evaluator(&original).physics_cache().unwrap();
    let (same, _a) = baked(&original, &bytes, "kin-same");
    assert!(same.physics_trace().is_ok());
    // a different animation, gravity, step, shape, and a force field all change the identity
    let variants = [
        animated(r#"<key time="0" value="40"/><key time="2" value="20"/>"#),
        original.replace("gravityY=\"-9.80665\"", "gravityY=\"-9.8\""),
        original.replace("fixedStep=\"0.01\"", "fixedStep=\"0.005\""),
        original.replace("radius=\"10\"", "radius=\"11\""),
        original.replace("velocityX=\"30\"", "velocityX=\"31\""),
    ];
    for (k, xml) in variants.iter().enumerate() {
        let (other, _file) = baked(xml, &bytes, &format!("kin-{k}"));
        let error = other.physics_trace().unwrap_err();
        assert!(error.contains("digest"), "variant {k}: {error}");
    }
}

#[test]
fn a_cache_written_before_the_trace_existed_is_still_read() {
    // SRPHYS02 with one body (a 3D rigid body also has a place in the 2D world) and two frames
    let mut v2 = Vec::new();
    v2.extend_from_slice(b"SRPHYS02");
    v2.extend_from_slice(&0.01f64.to_le_bytes());
    v2.extend_from_slice(&0.0f64.to_le_bytes());
    for n in [1u64, 0, 1, 2] {
        v2.extend_from_slice(&n.to_le_bytes());
    }
    for k in 0..2 {
        for v in [0.0, 0.0, 0.0, 10.0 * k as f64, -120.0, 0.0, 0.0, 0.0, 0.0, 1.0] {
            v2.extend_from_slice(&v.to_le_bytes());
        }
    }
    let xml = scene(BALL, &physics(""));
    let (old, _file) = baked(&xml, &v2, "v2");
    let frame = old.evaluate(0.011);
    assert!(frame.problems.iter().all(|p| !p.contains("simulating")), "{:?}", frame.problems);
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let trace = old.physics_trace().unwrap();
    assert!(trace.contacts.is_empty(), "an old cache recorded no contacts");
    assert!(trace.velocities.is_empty(), "nor velocities");
}

#[test]
fn a_damaged_trace_section_is_an_error() {
    let xml = scene(&format!("{BALL}{FLOOR}"), &physics(""));
    let bytes = evaluator(&xml).physics_cache().unwrap();
    // a record count that promises more contacts than the file holds
    let (_, tail) = bytes.split_at(bytes.len() - 96);
    let truncated = &bytes[..bytes.len() - 40];
    assert_eq!(tail.len(), 96);
    let (cache, _file) = baked(&xml, truncated, "cut");
    let frame = cache.evaluate(1.0);
    assert!(frame.failures.iter().any(|f| f.contains("truncated")), "{:?}", frame.failures);
    // a contact that names a step the file does not have
    let mut bad = bytes.clone();
    let n = bad.len();
    bad[n - 96..n - 88].copy_from_slice(&u64::MAX.to_le_bytes());
    let (cache, _file) = baked(&xml, &bad, "step");
    let frame = cache.evaluate(1.0);
    assert!(frame.failures.iter().any(|f| f.contains("contact")), "{:?}", frame.failures);
}

#[test]
fn a_version_3_cache_is_still_read_and_its_flags_are_still_checked() {
    let v3 = |flag: f64| {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"SRPHYS03");
        bytes.extend_from_slice(&0.01f64.to_le_bytes());
        bytes.extend_from_slice(&0.0f64.to_le_bytes());
        for n in [1u64, 0, 1, 0, 2] {
            bytes.extend_from_slice(&n.to_le_bytes());
        }
        for k in 0..2 {
            for v in [0.0, 0.0, 0.0, 10.0 * k as f64, -120.0, 0.0, 0.0, 0.0, 0.0, 1.0, flag] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        bytes
    };
    let xml = scene(BALL, &physics(""));
    let (good, _a) = baked(&xml, &v3(1.0), "v3");
    let frame = good.evaluate(0.011);
    assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
    assert!(good.physics_trace().unwrap().contacts.is_empty());
    let (bad, _b) = baked(&xml, &v3(0.5), "v3-flag");
    let frame = bad.evaluate(0.011);
    assert!(frame.problems.iter().any(|p| p.contains("state flag")), "{:?}", frame.problems);
}

#[test]
fn the_identity_of_a_document_with_only_2d_bodies_covers_their_inputs_too() {
    let flat = |keys: &str, gravity: &str| {
        format!(
            r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="2"/><composition>
              <shape id="box" shape="rect" width="10" height="10" fill="#FF0000" x="20" y="0"><rigidBody velocityX="12"/></shape>
              <shape id="bar" shape="rect" width="40" height="4" fill="#00FF00" x="10" y="30">
                <animate property="y" timeBase="composition">{keys}</animate><rigidBody type="kinematic"/></shape>
            </composition><physics gravityY="{gravity}" pixelsPerMeter="20" fixedStep="0.01"/></scene>"##
        )
    };
    let original = flat(r#"<key time="0" value="30"/><key time="2" value="20"/>"#, "-9.8");
    let bytes = evaluator(&original).physics_cache().unwrap();
    let (same, _a) = baked(&original, &bytes, "flat-same");
    assert!(same.physics_trace().is_ok(), "{:?}", same.physics_trace());
    for (k, xml) in [
        flat(r#"<key time="0" value="30"/><key time="2" value="25"/>"#, "-9.8"),
        flat(r#"<key time="0" value="30"/><key time="2" value="20"/>"#, "-9.0"),
        original.replace("velocityX=\"12\"", "velocityX=\"13\""),
    ]
    .iter()
    .enumerate()
    {
        let (other, _file) = baked(xml, &bytes, &format!("flat-{k}"));
        assert!(other.physics_trace().unwrap_err().contains("digest"), "variant {k}");
    }
}
