//! A body the solver cannot place contacts for, with a collider or a start position beyond 2^47 m, is refused by
//! name when the world is built instead of reaching the solver. sr-fuzz found a projectile of radius 1e308 that
//! made Rapier index past a contact manifold ("index out of bounds: the len is 58 but the index is
//! 18446744073709551615", main CI runs 37555473877 to 37754640851).

use sr_eval::Evaluator;

/// The failures and problems of `xml` evaluated where sr-fuzz evaluates it: at the start, the middle and the end.
fn said(xml: &str) -> Vec<String> {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let ev = Evaluator::new(&doc, &Default::default()).unwrap();
    let d = ev.program().duration;
    let mut out = Vec::new();
    for t in [0.0, d * 0.5, d] {
        let frame = ev.evaluate(t);
        out.extend(frame.problems.iter().chain(&frame.failures).cloned());
    }
    out
}

fn rock_over_ground(rock: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><composition>{rock}<object3D id="ground" primitive="plane" width="20" height="20" segments="40" y="2"><crater id="pit" source="rock" targetMaterial="softRock" repose="35"/><rigidBody type="static"/></object3D></composition><physics pixelsPerMeter="1"/></scene>"##
    )
}

#[test]
fn a_projectile_of_radius_1e308_is_refused_by_name_and_the_solver_never_sees_it() {
    // the input sr-fuzz wrote to fuzz-crashes/0000-document.txt (CI run 37755948658)
    let said = said(&rock_over_ground(
        r#"<object3D id="rock" primitive="sphere" radius="1e308" y="-8"><rigidBody mass="5"/></object3D>"#,
    ));
    assert!(said.iter().any(|m| m.starts_with("rock: rigidBody:") && m.contains("2^47 m")), "{said:?}");
}

#[test]
fn a_body_placed_beyond_the_solver_is_refused_by_name() {
    let said = said(&rock_over_ground(
        r#"<object3D id="rock" primitive="sphere" radius="1" y="1e300"><rigidBody mass="5"/></object3D>"#,
    ));
    assert!(said.iter().any(|m| m.starts_with("rock: rigidBody:") && m.contains("2^47 m")), "{said:?}");
}

#[test]
fn a_large_body_within_the_solver_is_not_refused() {
    // a rock a kilometre across over a plain static ground, at a metre a pixel
    let said = said(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="1"/><composition><object3D id="rock" primitive="sphere" radius="1000" y="-2000"><rigidBody mass="5"/></object3D><object3D id="ground" primitive="plane" width="20" height="20" y="2"><rigidBody type="static"/></object3D></composition><physics pixelsPerMeter="1"/></scene>"##,
    );
    assert!(!said.iter().any(|m| m.contains("2^47 m")), "{said:?}");
}
