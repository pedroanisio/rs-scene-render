//! The typed model of a Schwarzschild black hole, its disk and the camera that traces geodesics: what a renderer reads
//! is in the model with the defaults the schema gives, and the rules refuse what the renderer cannot draw.

use sr_model::model::{AccretionDisk, BlackHole, Camera};

const SCENE: &str = r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition>
  <shape id="sky" shape="rect" x="0" y="0" width="64" height="64" fill="#000000"/>
  <camera id="eye" x="0" y="0" z="-60" geodesics="true"/>
  <blackHole id="hole" mass="1.5" x="1" y="2" z="3"/>
  <accretionDisk id="disk" blackHole="hole" outerRadius="30" temperatureScale="6000"/>
</composition></scene>"##;

fn find<T: 'static + Clone>(doc: &sr_model::Document) -> T {
    let mut found = None;
    sr_model::element::walk(&doc.scene, &mut |e| {
        if let Some(x) = e.as_any().downcast_ref::<T>() {
            found = Some(x.clone());
        }
    });
    found.expect("the element is in the model")
}

fn codes(xml: &str) -> Vec<String> {
    sr_model::validate_str(xml, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn the_model_carries_what_a_renderer_reads_with_the_schemas_defaults() {
    let doc = sr_model::load_str(SCENE, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let hole: BlackHole = find(&doc);
    assert_eq!((hole.mass.get(), hole.x, hole.y, hole.z), (1.5, 1.0, 2.0, 3.0));
    let disk: AccretionDisk = find(&doc);
    assert_eq!(disk.black_hole, "hole");
    assert_eq!((disk.outer_radius.get(), disk.temperature_scale.get()), (30.0, 6000.0));
    // the inner radius is the innermost stable circular orbit when absent: the renderer takes 6 mass
    assert!(disk.inner_radius.is_none());
    assert_eq!(disk.seed, 0);
    assert_eq!(disk.angular_pattern.as_str(), "clumps");
    assert_eq!((disk.contrast.get(), disk.intensity.get(), disk.time_scale.get()), (0.5, 1.0, 1.0));
    assert_eq!((disk.rotation_x, disk.rotation_y, disk.rotation), (0.0, 0.0, 0.0));
    // a disk has no place of its own: it is centred on its hole
    assert!(!sr_model::validate_str(
        &SCENE.replace("temperatureScale", "x=\"1\" temperatureScale"),
        &sr_model::LoadOptions::without_assets()
    )
    .diagnostics
    .is_empty());
    let camera: Camera = find(&doc);
    assert!(camera.geodesics);
}

#[test]
fn a_camera_without_geodesics_is_the_camera_there_always_was() {
    let xml = SCENE.replace(r#" geodesics="true""#, "");
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    assert!(!find::<Camera>(&doc).geodesics);
    // and the hole and the disk are warned about, never silently dropped
    assert_eq!(codes(&xml), ["W03", "W03"]);
}

#[test]
fn every_rule_names_what_it_refuses() {
    for (what, from, to, code) in [
        ("a version before 1.3", r#"version="1.3""#, r#"version="1.2""#, "BH1"),
        ("two holes", "<accretionDisk", r#"<blackHole id="b" mass="1"/><accretionDisk"#, "BH2"),
        ("a disk of nothing", r#"blackHole="hole""#, r#"blackHole="eye""#, "BH3"),
        // 8 is below the last stable orbit of a hole of mass 1.5, 6 * 1.5 = 9
        (
            "a disk inside the last stable orbit",
            r#"<accretionDisk id="disk""#,
            r#"<accretionDisk id="disk" innerRadius="8""#,
            "BH4",
        ),
        // the hole is at (1, 2, 3): this is 2 from it, inside 3 * 1.5
        ("a camera inside the photon sphere", r#"x="0" y="0" z="-60""#, r#"x="1" y="2" z="1""#, "BH7"),
        ("another object", "<blackHole", r#"<object3D id="ball" primitive="sphere" radius="1"/><blackHole"#, "BH6"),
        (
            "a second geodesic camera",
            "<blackHole",
            r#"<camera id="eye2" x="0" y="0" z="-90" geodesics="true"/><blackHole"#,
            "BH8",
        ),
    ] {
        let got = codes(&SCENE.replace(from, to));
        assert!(got.contains(&code.to_string()), "{what}: {got:?}");
    }
    let no_hole = SCENE
        .replace(r#"<blackHole id="hole" mass="1.5" x="1" y="2" z="3"/>"#, "")
        .replace(r#"<accretionDisk id="disk" blackHole="hole" outerRadius="30" temperatureScale="6000"/>"#, "");
    assert!(codes(&no_hole).contains(&"BH5".to_string()));
}

#[test]
fn the_distance_to_the_hole_is_the_distance_from_its_place() {
    // the camera 4.4 from a hole of mass 1.5 (3 mass = 4.5) is inside, 4.6 is outside
    let xml = |d: f64| SCENE.replace(r#"x="0" y="0" z="-60""#, &format!(r#"x="1" y="2" z="{}""#, 3.0 - d));
    assert!(codes(&xml(4.4)).contains(&"BH7".to_string()));
    assert!(codes(&xml(4.6)).is_empty(), "{:?}", codes(&xml(4.6)));
}

#[test]
fn a_scene_that_only_draws_the_hole_is_valid_and_the_disk_is_bounded_by_the_orbit() {
    assert!(codes(SCENE).is_empty(), "{:?}", codes(SCENE));
    // 9 is exactly 6 * 1.5: the last stable orbit is allowed; outer radius must be beyond it
    let at = SCENE.replace("<accretionDisk id=\"disk\"", "<accretionDisk id=\"disk\" innerRadius=\"9\"");
    assert!(codes(&at).is_empty(), "{:?}", codes(&at));
    assert!(codes(&at.replace(r#"outerRadius="30""#, r#"outerRadius="9""#)).contains(&"BH4".to_string()));
}
