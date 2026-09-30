//! Physics bodies, soft bodies and particles from documents.

mod common;
use common::*;

fn scene(body: &str, physics: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.1"><project width="64" height="64" fps="30" duration="4" background="#00000000"/>{ASSETS}<composition>{body}</composition>{physics}</scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn rows_covered(r: &Rendered) -> Vec<u32> {
    (0..64).filter(|y| (0..64).any(|x| r.at(x, *y)[3] > 0.5)).collect()
}

fn problems(r: &Rendered) -> Vec<String> {
    r.stats.unsupported.iter().chain(&r.stats.errors).cloned().collect()
}

#[test]
fn rigid_bodies_fall_onto_the_floor() {
    let d = scene(
        r#"<layer id="box" asset="red" x="24" y="0" scaleX="4" scaleY="4"><rigidBody/></layer>"#,
        r#"<physics bounds="floor" pixelsPerMeter="20"/>"#,
    );
    let Some(r0) = render_times(&d, &[0.0]) else { return };
    assert!(problems(&r0).is_empty(), "{:?}", problems(&r0));
    assert_eq!(rows_covered(&r0).first(), Some(&0), "starts at the top");
    let r2 = render_times(&d, &[2.0]).unwrap();
    let rows = rows_covered(&r2);
    assert!(*rows.last().unwrap() == 63 && rows.len() >= 15 && rows.len() <= 17, "rests on the floor: {rows:?}");
}

#[test]
fn particles_draw_and_replay() {
    let body = r#"<particleEmitter id="sparks" preset="sparks" x="32" y="40" seed="3"/>"#;
    let d = scene(body, "");
    let Some(a) = render_times(&d, &[1.0]) else { return };
    assert!(problems(&a).is_empty(), "{:?}", problems(&a));
    let lit = a.px.iter().filter(|p| p[3] > 0.05).count();
    assert!(lit > 40, "sparks cover pixels: {lit}");
    let warm = a.px.iter().filter(|p| p[3] > 0.2).all(|p| p[0] >= p[2]);
    assert!(warm, "sparks are warm (red over blue)");
    // a later frame first, then back: the same pixels as rendering directly
    let b = render_times(&d, &[2.5, 1.0]).unwrap();
    assert_eq!(a.px, b.px, "deterministic under seeking");
    // streaks and trails
    let d = scene(
        r#"<particleEmitter id="rain" preset="rain" x="32" y="0" emitterShape="line" emitterWidth="64" seed="1"/>"#,
        "",
    );
    let r = render_times(&d, &[1.0]).unwrap();
    assert!(r.px.iter().filter(|p| p[3] > 0.05).count() > 20);
}

#[test]
fn soft_bodies_deform_their_layer() {
    let body =
        |soft: &str| format!(r#"<layer id="blob" asset="red" x="16" y="0" scaleX="8" scaleY="8">{soft}</layer>"#);
    let rest = render_times(&scene(&body(""), ""), &[0.0]);
    let Some(rest) = rest else { return };
    let d = scene(
        &body(r#"<softBody kind="jelly" stiffness="20" mass="2" rows="3" cols="3"/>"#),
        r#"<physics bounds="floor" pixelsPerMeter="40"/>"#,
    );
    let r = render_times(&d, &[2.0]).unwrap();
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    let rows = rows_covered(&r);
    assert!(rows.contains(&63), "it fell to the floor: {rows:?}");
    assert_ne!(rows_covered(&rest).len(), rows.len(), "and changed shape on landing");
}

#[test]
fn physics_problems_are_reported() {
    let d = scene(
        r#"<layer id="box" asset="red" x="24" y="0" scaleX="4" scaleY="4"><rigidBody/></layer><layer id="plain" asset="red"/>"#,
        r#"<physics><constraint id="c" type="distance" a="box" b="plain"/></physics>"#,
    );
    let Some(r) = render_times(&d, &[0.5]) else { return };
    assert!(problems(&r).iter().any(|m| m.contains("plain") && m.contains("rigidBody")), "{:?}", problems(&r));
}

#[test]
fn physics_cache_round_trip() {
    let d = scene(
        r#"<layer id="box" asset="red" x="24" y="0" scaleX="4" scaleY="4"><rigidBody/></layer>"#,
        r#"<physics bounds="floor" pixelsPerMeter="20"/>"#,
    );
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let bytes = ev.physics_cache().unwrap();
    assert!(bytes.starts_with(b"SRPHYS02"));
    use sha2::Digest;
    let sha: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    std::fs::write(fixtures().join("fall.physics"), &bytes).unwrap();
    // a version 1 file (no 3D bodies: no count after the soft-body sizes) reads the same
    let mut v1 = b"SRPHYS01".to_vec();
    v1.extend_from_slice(&bytes[8..40]);
    v1.extend_from_slice(&bytes[48..]);
    assert_eq!(u64::from_le_bytes(bytes[40..48].try_into().unwrap()), 0, "no 3D bodies");
    let sha1: String = sha2::Sha256::digest(&v1).iter().map(|b| format!("{b:02x}")).collect();
    std::fs::write(fixtures().join("fall-v1.physics"), &v1).unwrap();
    let old = scene(
        r#"<layer id="box" asset="red" x="24" y="0" scaleX="4" scaleY="4"><rigidBody/></layer>"#,
        &format!(r#"<physics bounds="floor" pixelsPerMeter="20" cache="fall-v1.physics" cacheSha256="{sha1}"/>"#),
    );
    let g1 = sr_eval::Evaluator::new(&old, &Default::default()).unwrap().evaluate(1.5);
    assert!(g1.problems.is_empty(), "{:?}", g1.problems);
    let simulated = ev.evaluate(1.5);
    let cached = scene(
        r#"<layer id="box" asset="red" x="24" y="0" scaleX="4" scaleY="4"><rigidBody/></layer>"#,
        &format!(r#"<physics bounds="floor" pixelsPerMeter="20" cache="fall.physics" cacheSha256="{sha}"/>"#),
    );
    let ev2 = sr_eval::Evaluator::new(&cached, &Default::default()).unwrap();
    let g = ev2.evaluate(1.5);
    assert!(g.problems.is_empty(), "{:?}", g.problems);
    let world = |g: &sr_eval::FrameGraph| g.nodes.iter().find(|n| &*n.id == "box").unwrap().world;
    assert_eq!(world(&g), world(&simulated), "the cache replays the simulation exactly");
    assert_eq!(world(&g1), world(&simulated), "and so does a version 1 cache");
    // a wrong digest fails validation (A02); unverified, the evaluator reports it and simulates instead
    let bad = format!(
        r##"<scene version="1.1"><project width="64" height="64" fps="30" duration="4" background="#00000000"/>{ASSETS}<composition><layer id="box" asset="red" x="24" y="0" scaleX="4" scaleY="4"><rigidBody/></layer></composition><physics bounds="floor" pixelsPerMeter="20" cache="fall.physics" cacheSha256="{}"/></scene>"##,
        "0".repeat(64)
    );
    let verified = sr_model::load_str(&bad, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) });
    assert!(format!("{verified:?}").contains("A02"));
    let d3 =
        sr_model::load_str(&bad, &sr_model::LoadOptions { verify_assets: false, base_dir: Some(fixtures()) }).unwrap();
    let g3 = sr_eval::Evaluator::new(&d3, &Default::default()).unwrap().evaluate(1.5);
    assert!(g3.problems.iter().any(|m| m.contains("SHA-256")), "{:?}", g3.problems);
    assert_eq!(world(&g3), world(&simulated));
    std::fs::remove_file(fixtures().join("fall.physics")).ok();
    std::fs::remove_file(fixtures().join("fall-v1.physics")).ok();
}

fn same(a: &Rendered, b: &Rendered) -> bool {
    a.px.iter().zip(&b.px).all(|(p, q)| (0..4).all(|c| (p[c] - q[c]).abs() < 1e-5))
}

#[test]
fn flocks_fluids_slime_and_erosion_draw_and_seek_deterministically() {
    // Flock, fluid, slime and erosion simulations: each draws, changes over time, and reaching a time directly
    // gives the same frame as playing up to it
    let cases = [
        r#"<flock id="s" width="64" height="64" count="80" seed="2" size="3" shape="disc"/>"#,
        r##"<fluid id="s" width="64" height="64" resolution="32" vorticity="1">
              <fluidSource x="32" y="56" radius="6" color="#FF8030" density="4" velocityY="-60"/></fluid>"##,
        r#"<slime id="s" width="64" height="64" resolution="64" agents="3000" seed="4" spawn="random"/>"#,
        r#"<erosion id="s" width="64" height="64" resolution="64" droplets="30000" seed="6"/>"#,
    ];
    for body in cases {
        let d = scene(body, "");
        let Some(early) = render_times(&d, &[0.5]) else { return };
        assert!(problems(&early).is_empty(), "{body}: {:?}", problems(&early));
        let direct = render_times(&d, &[2.0]).unwrap();
        let played = render_times(&d, &[0.5, 1.0, 1.5, 2.0]).unwrap();
        let back = render_times(&d, &[3.0, 2.0]).unwrap();
        assert!(direct.px.iter().any(|p| p[3] > 0.1), "{body}: nothing drawn");
        assert!(!same(&early, &direct), "{body}: no change over time");
        assert!(same(&direct, &played), "{body}: seeking and playing differ");
        assert!(same(&direct, &back), "{body}: seeking back differs");
    }
}

#[test]
fn simulated_content_inside_isolated_groups_follows_the_frame() {
    // an isolated group caches its content by hash: particles and simulation pictures change
    // without their nodes' attributes changing, so the hash must see the simulated content
    for inner in [
        r#"<particleEmitter id="p" preset="sparks" x="32" y="40" seed="3"/>"#,
        r#"<slime id="p" width="64" height="64" resolution="64" agents="3000" seed="4" spawn="random"/>"#,
    ] {
        let d = scene(&format!(r#"<group id="g" isolate="true">{inner}</group>"#), "");
        let Some(cold) = render_times(&d, &[1.5]) else { return };
        let warm = render_times(&d, &[0.5, 1.0, 1.5]).unwrap();
        assert!(same(&cold, &warm), "{inner}: a warm frame differs from a cold one");
    }
}

// ------------------------------------------------------------------ 3D bodies

fn pose3(g: &sr_eval::FrameGraph, id: &str) -> [f64; 16] {
    g.nodes.iter().find(|n| &*n.id == id).and_then(|n| n.pose3).unwrap_or_else(|| panic!("{id} has no 3D pose"))
}

fn scene3(objects: &str, physics: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="200" height="200" fps="30" duration="4" background="#000000"/>
          <materials><material id="red" baseColor="#FF0000" roughness="1"/></materials>
          <composition>{objects}</composition>
          <lights><light id="amb" type="ambient" intensity="1"/></lights>{physics}</scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

#[test]
fn objects_fall_and_feel_fields_in_3d() {
    // a sphere of radius 10 falls 1 s: ½ g t² = 4.903 m = 490 px at 100 px/m
    let d = scene3(
        r#"<object3D id="s" primitive="sphere" radius="10" x="100" y="0" z="0"><rigidBody linearDamping="0"/></object3D>"#,
        r#"<physics/>"#,
    );
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let g = ev.evaluate(1.0);
    assert!(g.problems.is_empty(), "{:?}", g.problems);
    let m = pose3(&g, "s");
    assert!((m[13] - 490.3).abs() < 3.0 && (m[12] - 100.0).abs() < 1e-6 && m[14].abs() < 1e-6, "{m:?}");
    // pushed toward the camera (forceZ, +z toward the viewer): scene z goes negative
    let d = scene3(
        r#"<object3D id="s" primitive="sphere" radius="10" x="100" y="100"><rigidBody linearDamping="0"/></object3D>"#,
        r#"<physics gravityY="0"><forceField id="f" type="directional" forceZ="2"/></physics>"#,
    );
    let g = sr_eval::Evaluator::new(&d, &Default::default()).unwrap().evaluate(1.0);
    let m = pose3(&g, "s");
    assert!((m[14] + 100.0).abs() < 3.0 && (m[13] - 100.0).abs() < 1e-6, "{m:?}");
    // gravityZ does the same
    let d = scene3(
        r#"<object3D id="s" primitive="sphere" radius="10" x="100" y="100"><rigidBody linearDamping="0"/></object3D>"#,
        r#"<physics gravityY="0" gravityZ="2"/>"#,
    );
    let g = sr_eval::Evaluator::new(&d, &Default::default()).unwrap().evaluate(1.0);
    assert!((pose3(&g, "s")[14] + 100.0).abs() < 3.0);
}

#[test]
fn boxes_stack_on_a_static_mesh_and_seek_deterministically() {
    // a static torus-free ground (a wide box as a triangle mesh) and two boxes dropped on it
    let objects = r#"
      <object3D id="ground" primitive="box" width="400" height="20" depth="400" x="100" y="190"><rigidBody type="static" shape="trimesh"/></object3D>
      <object3D id="a" primitive="box" width="30" height="30" depth="30" x="100" y="100" material="red"><rigidBody/></object3D>
      <object3D id="b" primitive="box" width="30" height="30" depth="30" x="102" y="40" rotationY="20" material="red"><rigidBody/></object3D>
      <object3D id="rider" primitive="sphere" radius="4" y="-19" parent="b"/>"#;
    let d = scene3(objects, r#"<physics/>"#);
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let g = ev.evaluate(3.0);
    assert!(g.problems.is_empty(), "{:?}", g.problems);
    let (a, b) = (pose3(&g, "a"), pose3(&g, "b"));
    assert!((a[13] - 165.0).abs() < 1.5, "a rests on the ground (top at 180): {a:?}");
    assert!((b[13] - 135.0).abs() < 1.5, "b rests on a: {b:?}");
    // seeking: a later time first, then back, gives the same pose bit for bit
    let fresh = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let late = fresh.evaluate(3.5);
    let _ = late;
    assert_eq!(pose3(&fresh.evaluate(1.2), "b"), pose3(&ev.evaluate(1.2), "b"));
    // on the GPU: the red boxes are drawn where the simulation put them; the rider follows b
    let Some(r) = render_times(&d, &[3.0]) else { return };
    let red = |x: u32, y: u32| {
        let c = r.at(x, y);
        c[0] > 0.2 && c[1] < 0.05
    };
    assert!(red(100, 165) && red(100, 135), "both boxes are drawn at rest");
    assert!(!red(100, 60), "nothing left in the air");
    assert!(r.at(102, 116)[0] > 0.1 || r.at(101, 116)[0] > 0.1, "the rider sits on b");
}

#[test]
fn ball_joints_swing_and_3d_caches_replay() {
    // a pendulum on a ball joint to a static hook: the bob keeps its distance from the hook
    let objects = r#"<object3D id="hook" primitive="sphere" radius="2" x="100" y="40"><rigidBody type="static" collisionGroup="1" collidesWith="1"/></object3D>
      <object3D id="bob" primitive="sphere" radius="8" x="160" y="40" z="0"><rigidBody/></object3D>"#;
    let physics =
        |extra: &str| format!(r#"<physics{extra}><constraint id="j" type="ball" a="bob" b="hook"/></physics>"#);
    let d = scene3(objects, &physics(""));
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    for t in [0.5, 1.0, 1.7] {
        let g = ev.evaluate(t);
        assert!(g.problems.is_empty(), "{:?}", g.problems);
        let m = pose3(&g, "bob");
        let r = ((m[12] - 100.0).powi(2) + (m[13] - 40.0).powi(2) + m[14].powi(2)).sqrt();
        assert!((r - 60.0).abs() < 1.0, "t = {t}: {r}");
    }
    assert!(pose3(&ev.evaluate(0.4), "bob")[13] > 60.0, "it swings down");
    // the cache stores 3D poses and replays them exactly
    let bytes = ev.physics_cache().unwrap();
    use sha2::Digest;
    let sha: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    std::fs::write(fixtures().join("bob.physics"), &bytes).unwrap();
    let cached = scene3(objects, &physics(&format!(r#" cache="bob.physics" cacheSha256="{sha}""#)));
    let ev2 = sr_eval::Evaluator::new(&cached, &Default::default()).unwrap();
    let g = ev2.evaluate(1.7);
    std::fs::remove_file(fixtures().join("bob.physics")).ok();
    assert!(g.problems.is_empty(), "{:?}", g.problems);
    assert_eq!(pose3(&g, "bob"), pose3(&ev.evaluate(1.7), "bob"));
}

/// Columns covered on row `y`.
fn width_at(r: &Rendered, y: u32) -> u32 {
    (0..r.size[0]).filter(|x| r.at(*x, y)[3] > 0.5).count() as u32
}

#[test]
fn preroll_emits_before_the_start_as_at_the_start() {
    // 2 s of preroll at 10 px/s: at time 0 the particles already reach 20 px from the emitter (8 px dots)
    let body = r#"<particleEmitter id="e" x="8" y="32" preroll="2" rate="40" lifetime="10" speed="10" direction="0" spread="0" size="8"/>"#;
    let Some(r) = render(&scene(body, "")) else { return };
    let w = width_at(&r, 32);
    assert!((26..=30).contains(&w), "covered {w} columns");
}

#[test]
fn force_fields_act_on_the_simulations_that_take_them() {
    // a directional push of 10 px/s² over a 2 s preroll moves the oldest particle 20 px: an emitter that
    // lists the field and one with no list take it; one with useForceFields="false" does not
    let body = r#"<particleEmitter id="listed" x="8" y="10" preroll="2" rate="40" lifetime="10" speed="0" size="8" forceFields="push"/>
        <particleEmitter id="all" x="8" y="30" preroll="2" rate="40" lifetime="10" speed="0" size="8"/>
        <particleEmitter id="none" x="8" y="50" preroll="2" rate="40" lifetime="10" speed="0" size="8" useForceFields="false"/>"#;
    let physics =
        r#"<physics gravityY="0"><forceField id="push" type="directional" forceX="0.1" start="-2"/></physics>"#;
    let Some(r) = render(&scene(body, physics)) else { return };
    let (listed, all, none) = (width_at(&r, 10), width_at(&r, 30), width_at(&r, 50));
    assert!((26..=30).contains(&listed) && (26..=30).contains(&all), "pushed: {listed}, {all}");
    assert!((7..=9).contains(&none), "not pushed: {none}");
}
