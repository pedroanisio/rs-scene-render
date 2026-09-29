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
    assert!(bytes.starts_with(b"SRPHYS01"));
    use sha2::Digest;
    let sha: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    std::fs::write(fixtures().join("fall.physics"), &bytes).unwrap();
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
