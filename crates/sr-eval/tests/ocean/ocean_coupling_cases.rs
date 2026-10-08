//! Cases around the crater-driven and body-driven ocean: mesh assets, an animated
//! ocean, other plane orientations and clocks that cannot be inverted.
use sr_eval::{Evaluator, FrameGraph};
use std::path::PathBuf;

struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn temp(name: &str) -> Temp {
    let dir = std::env::temp_dir().join(format!("sr-ocean-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Temp(dir)
}
fn evaluator(xml: &str, dir: Option<&Temp>) -> Evaluator {
    let options = match dir {
        Some(dir) => sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir.0.clone()) },
        None => sr_model::LoadOptions::without_assets(),
    };
    let doc = sr_model::load_str(xml, &options).unwrap();
    Evaluator::new(&doc, &Default::default()).unwrap()
}
fn sea(frame: &FrameGraph) -> &sr_eval::ocean::SimOcean {
    frame.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap()
}
fn depths(frame: &FrameGraph) -> Vec<f64> {
    sea(frame).frame.cells.iter().map(|c| c.depth).collect()
}
/// Largest change of water depth from the 12-unit layer.
fn peak(frame: &FrameGraph) -> f64 {
    depths(frame).iter().map(|d| (d - 12.0).abs()).fold(0.0, f64::max)
}
fn largest_difference(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max)
}

const OCEAN: &str = r#"<ocean id="sea" bedResponse="hydrostatic" width="128" depth="128" cellSize="2" bottomDepth="12" dt="0.0416666666666667" boundary="closed" colliders="seabed"/>"#;
const CRATER: &str = r#"<crater radius="40" depth="8" rimHeight="3" rimWidth="8" start="0.5" end="1.5"/>"#;
fn scene(objects: &str, ocean: &str) -> String {
    format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/>{{ASSETS}}<composition>{objects}{ocean}</composition></scene>"#
    )
}

/// A flat grid of `n` x `n` quads, `size` scene units wide, in the object's xy plane.
/// The importer reads OBJ units as metres and a scene unit as a centimetre.
fn grid_obj(n: usize, size: f64) -> String {
    let size = size / 100.0;
    let mut text = String::new();
    for j in 0..=n {
        for i in 0..=n {
            text += &format!("v {} {} 0\n", (i as f64 / n as f64 - 0.5) * size, (j as f64 / n as f64 - 0.5) * size);
        }
    }
    for j in 0..n {
        for i in 0..n {
            let a = j * (n + 1) + i + 1;
            text += &format!("f {} {} {}\nf {} {} {}\n", a, a + 1, a + n + 1, a + 1, a + n + 2, a + n + 1);
        }
    }
    text
}
/// A cube of 10 scene units (0.1 m in the file).
const CUBE_OBJ: &str = "v -0.05 -0.05 -0.05\nv 0.05 -0.05 -0.05\nv 0.05 0.05 -0.05\nv -0.05 0.05 -0.05\nv -0.05 -0.05 0.05\nv 0.05 -0.05 0.05\nv 0.05 0.05 0.05\nv -0.05 0.05 0.05\n\
f 1 3 2\nf 1 4 3\nf 5 6 7\nf 5 7 8\nf 1 2 6\nf 1 6 5\nf 3 4 8\nf 3 8 7\nf 2 3 7\nf 2 7 6\nf 1 5 8\nf 1 8 4\n";

#[test]
fn a_mesh_asset_with_a_crater_deforms_the_bed_like_the_plane_it_replaces() {
    let dir = temp("mesh-bed");
    std::fs::write(dir.0.join("grid.obj"), grid_obj(40, 40.0)).unwrap();
    let make =
        |seabed: &str| scene(seabed, OCEAN).replace("{ASSETS}", r#"<assets><mesh id="grid" src="grid.obj"/></assets>"#);
    let mesh = evaluator(
        &make(&format!(
            r#"<object3D id="seabed" primitive="mesh" mesh="grid" y="12" rotationX="-90">{CRATER}</object3D>"#
        )),
        Some(&dir),
    )
    .evaluate(2.5);
    let plane = evaluator(
        &make(&format!(r#"<object3D id="seabed" primitive="plane" width="40" height="40" segments="40" y="12" rotationX="-90">{CRATER}</object3D>"#)),
        Some(&dir),
    )
    .evaluate(2.5);
    assert!(mesh.problems.is_empty() && plane.problems.is_empty(), "{:?} {:?}", mesh.problems, plane.problems);
    let (a, b) = (depths(&mesh), depths(&plane));
    println!(
        "MESH peak {:.4} against plane {:.4}, largest difference {:.2e}",
        peak(&mesh),
        peak(&plane),
        largest_difference(&a, &b)
    );
    assert!(peak(&mesh) > 0.5, "the mesh bed made no wave");
    // The two tessellations share their vertices and differ only in how each quad is split.
    assert!(largest_difference(&a, &b) < 0.03 * peak(&plane), "{}", largest_difference(&a, &b));
}

#[test]
fn a_closed_mesh_asset_is_a_body_like_the_box_it_replaces() {
    let dir = temp("mesh-body");
    std::fs::write(dir.0.join("cube.obj"), CUBE_OBJ).unwrap();
    let fall = r#"<animate property="y"><key time="0" value="-40"/><key time="1" value="6"/></animate>"#;
    let make = |body: &str| {
        scene(&format!("<object3D id=\"seabed\" {body}>{fall}</object3D>"), OCEAN)
            .replace("{ASSETS}", r#"<assets><mesh id="cube" src="cube.obj"/></assets>"#)
    };
    let mesh = evaluator(&make(r#"primitive="mesh" mesh="cube""#), Some(&dir)).evaluate(2.0);
    let boxed = evaluator(&make(r#"primitive="box" width="10" height="10" depth="10""#), Some(&dir)).evaluate(2.0);
    assert!(mesh.problems.is_empty() && boxed.problems.is_empty(), "{:?} {:?}", mesh.problems, boxed.problems);
    println!(
        "MESHBODY peak {:.4}, difference to the box {:.2e}",
        peak(&mesh),
        largest_difference(&depths(&mesh), &depths(&boxed))
    );
    assert!(peak(&mesh) > 0.5);
    assert!(largest_difference(&depths(&mesh), &depths(&boxed)) < 1e-9);
}

#[test]
fn an_animated_ocean_sees_the_crater_slide_across_its_domain() {
    // The ocean drifts 20 units in +x while the crater stays in the world: in the ocean's
    // frame that is the crater drifting in -x, so it must equal the fixed ocean over a
    // seabed that drifts the other way.
    let drift = |axis: &str, to: f64| {
        format!(r#"<animate property="x"><key time="0" value="0"/><key time="6" value="{to}"/></animate>{axis}"#)
    };
    let seabed = |moving: &str| {
        format!(
            r#"<object3D id="seabed" primitive="plane" width="400" height="400" segments="80" y="12" rotationX="-90">{moving}{CRATER}</object3D>"#
        )
    };
    let moving_ocean = scene(
        &seabed(""),
        &OCEAN.replace("colliders=\"seabed\"/>", &format!("colliders=\"seabed\">{}</ocean>", drift("", 20.0))),
    )
    .replace("{ASSETS}", "");
    let moving_seabed = scene(&seabed(&drift("", -20.0)), OCEAN).replace("{ASSETS}", "");
    let a = evaluator(&moving_ocean, None).evaluate(3.0);
    let b = evaluator(&moving_seabed, None).evaluate(3.0);
    assert!(a.problems.is_empty() && b.problems.is_empty(), "{:?} {:?}", a.problems, b.problems);
    let still = evaluator(&scene(&seabed(""), OCEAN).replace("{ASSETS}", ""), None).evaluate(3.0);
    println!(
        "ANIMATED peak {:.4}; difference to the equivalent {:.2e}; difference to a still ocean {:.4}",
        peak(&a),
        largest_difference(&depths(&a), &depths(&b)),
        largest_difference(&depths(&a), &depths(&still))
    );
    assert!(peak(&a) > 0.5);
    assert!(largest_difference(&depths(&a), &depths(&b)) < 1e-6 * peak(&a));
    // The drift matters: it is not the still-ocean result.
    assert!(largest_difference(&depths(&a), &depths(&still)) > 0.05 * peak(&a));
}

#[test]
fn planes_in_other_orientations_move_the_bed_the_way_their_crater_points() {
    let bed_under_centre = |attributes: &str| {
        let seabed = format!(
            r#"<object3D id="seabed" primitive="plane" width="400" height="400" segments="80" {attributes}>{CRATER}</object3D>"#
        );
        let frame = evaluator(&scene(&seabed, OCEAN).replace("{ASSETS}", ""), None).evaluate(2.5);
        assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        (sea(&frame).frame.bed[32 * 64 + 32], peak(&frame))
    };
    // Facing down the crater digs (bed ordinate grows); facing up it builds a mound.
    let (down, dig) = bed_under_centre(r#"y="12" rotationX="-90""#);
    let (up, mound) = bed_under_centre(r#"y="12" rotationX="90""#);
    println!("PLANES centre bed: digging {down:.3}, mounding {up:.3}; peaks {dig:.3} and {mound:.3}");
    assert!(down > 12.0 + 4.0 && up < 12.0 - 4.0, "{down} {up}");
    // Tilted 30 degrees about x: still a surface over the columns, and a wave follows.
    let (_, tilted) = bed_under_centre(r#"y="12" rotationX="-60""#);
    assert!(tilted > 0.5, "a tilted plane made no wave: {tilted}");
    // A vertical plane has no horizontal extent: it covers no column, moves no bed, and
    // is not an error.
    let (wall, none) = bed_under_centre(r#"y="12" rotationX="0""#);
    assert_eq!((wall, none), (12.0, 0.0));
}

#[test]
fn a_crater_on_a_primitive_that_cannot_be_a_bed_is_rejected_when_the_scene_loads() {
    let sphere =
        scene(&format!(r#"<object3D id="seabed" primitive="sphere" radius="5" y="12">{CRATER}</object3D>"#), OCEAN)
            .replace("{ASSETS}", "");
    let Err(error) = sr_model::load_str(&sphere, &sr_model::LoadOptions::without_assets()) else {
        panic!("a sphere with a crater was accepted as a collider")
    };
    assert!(format!("{error:?}").contains("OCN6"), "{error:?}");
}

/// A symbol instance with `loop` has a clock that cannot be inverted in general,
/// so the ocean replays its history for every frame; inside the first cycle it must
/// give what the plain instance gives. The seabed is inside the symbol with the ocean.
#[test]
fn a_clock_that_cannot_be_inverted_gives_the_result_of_the_equivalent_linear_clock() {
    let make = |instance: &str| {
        format!(
            r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="8"/><symbols><symbol id="shot" width="64" height="64" duration="6"><object3D id="seabed" primitive="plane" width="400" height="400" segments="80" y="12" rotationX="-90">{CRATER}</object3D>{OCEAN}</symbol></symbols><composition><instance id="take" symbol="shot" {instance}/></composition></scene>"#
        )
    };
    let linear = evaluator(&make(""), None);
    let looped = evaluator(&make(r#"loop="1""#), None);
    let (a, b) = (linear.evaluate(3.0), looped.evaluate(3.0));
    println!("CLOCK linear problems {:?}, looped problems {:?}", a.problems, b.problems);
    assert!(a.problems.is_empty() && b.problems.is_empty(), "{:?} {:?}", a.problems, b.problems);
    let find =
        |frame: &FrameGraph| frame.nodes.iter().find(|n| n.id.ends_with("sea")).unwrap().sim_ocean.clone().unwrap();
    let (x, y) = (find(&a), find(&b));
    assert!(x.frame.cells.iter().any(|c| (c.depth - 12.0).abs() > 0.5), "the linear clock made no wave");
    assert_eq!(x.frame, y.frame);
    // Frame after frame, and after a backward seek.
    looped.evaluate(2.0);
    looped.evaluate(1.0);
    assert_eq!(find(&looped.evaluate(3.0)).frame, x.frame);
}
