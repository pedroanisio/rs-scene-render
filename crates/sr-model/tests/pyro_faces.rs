//! A source too close to an open face of the volume changes the flow of the cloud, so the
//! document is warned about it, but only where that can be told without simulating anything and
//! where the volume has room to put the source further in.

use sr_model::Severity;

fn warnings(pyro: &str, source: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><composition>
          <object3D id="rock" primitive="sphere" radius="1" y="-8"><rigidBody mass="5"/></object3D>
          <object3D id="ground" primitive="plane" width="100" height="100" segments="32" y="2">
            <crater id="pit" source="rock" targetMaterial="softRock"/><rigidBody type="static"/></object3D>
          <object3D id="cloud" primitive="volume"><pyro {pyro}>{source}</pyro></object3D>
        </composition><physics pixelsPerMeter="1"/></scene>"#
    );
    let report = sr_model::validate_str(&xml, &sr_model::LoadOptions::without_assets());
    assert_eq!(report.error_count(), 0, "{:?}", report.diagnostics);
    report.diagnostics.into_iter().filter(|d| d.severity == Severity::Warning).map(|d| d.code).collect()
}

/// The plume volume of the hero scene: 64 x 52 x 64 cells of 3 units, open.
const HERO: &str = r#"width="192" height="156" depth="192" voxelSize="3" boundary="open" dt="0.04""#;

#[test]
fn a_source_less_than_twelve_cells_from_an_open_face_is_warned_about() {
    // the hero scene's impulse: a sphere of radius 22 at y = 35, its lower edge 57 units down in a volume
    // 78 units either side of the middle, that is 7 cells from the bottom face
    let near = r#"<pyroImpulse shape="sphere" y="35" radius="22" time="1" density="0.8"/>"#;
    assert_eq!(warnings(HERO, near), ["W02"]);
    let near_source = r#"<pyroSource shape="sphere" y="30" radius="16" start="1" end="1.5" densityRate="4"/>"#;
    assert_eq!(warnings(HERO, near_source), ["W02"]);
    // any open face: a box against a side face, x from 40 to 90 in a volume 96 units either side
    let beside = r#"<pyroSource shape="box" x="65" width="50" height="20" depth="20" densityRate="1"/>"#;
    assert_eq!(warnings(HERO, beside), ["W02"]);
    // 12 cells, 36 units, is enough: edge at 78 - 36 = 42
    let enough = r#"<pyroImpulse shape="sphere" y="20" radius="22" time="1" density="0.8"/>"#;
    assert!(warnings(HERO, enough).is_empty(), "{:?}", warnings(HERO, enough));
}

#[test]
fn only_where_the_position_is_known_the_face_is_open_and_there_is_room() {
    let near = r#"<pyroImpulse shape="sphere" y="35" radius="22" time="1" density="0.8"/>"#;
    // closed faces are walls
    assert!(warnings(&HERO.replace(r#"boundary="open""#, r#"boundary="closed""#), near).is_empty());
    // a source from a crater has no position in the document
    assert!(warnings(HERO, r#"<pyroSource crater="pit"/>"#).is_empty());
    // an animated source moves
    let moving = r#"<pyroSource shape="sphere" y="35" radius="22" densityRate="1"><animate property="y"><key time="0" value="35"/><key time="1" value="0"/></animate></pyroSource>"#;
    assert!(warnings(HERO, moving).is_empty());
    // a volume with no room for twelve cells on each side of a source: 8 x 8 x 8 cells
    assert!(warnings(
        r#"width="8" height="8" depth="8" voxelSize="1" boundary="open""#,
        r#"<pyroSource radius="1" densityRate="1"/>"#
    )
    .is_empty());
}
