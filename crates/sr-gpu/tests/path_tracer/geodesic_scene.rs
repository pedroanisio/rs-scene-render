//! A document with a black hole, its disk and a camera that traces geodesics: what the renderer draws from it.

use super::common;

/// A camera 60 units from the hole at `degrees` from the disk's axis (the scene's up is -y).
fn scene_at(degrees: f64, camera_attrs: &str, hole: &str, disk_attrs: &str, extra: &str) -> sr_model::Document {
    let i = degrees.to_radians();
    let xml = format!(
        r##"<scene version="1.3"><project width="128" height="80" fps="24" duration="10" background="#000000"/><composition>
  <camera id="eye" x="{:.4}" y="{:.4}" z="0" target="hole" fov="40" pathSamples="4" {camera_attrs}/>
  <blackHole id="hole" mass="1" {hole}/>
  <accretionDisk id="disk" blackHole="hole" outerRadius="16" temperatureScale="6000" {disk_attrs}/>
  {extra}
</composition></scene>"##,
        60.0 * i.sin(),
        -60.0 * i.cos()
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn scene(camera_attrs: &str, hole: &str, disk_attrs: &str, extra: &str) -> sr_model::Document {
    scene_at(75.0, camera_attrs, hole, disk_attrs, extra)
}

fn luma(p: [f32; 4]) -> f32 {
    p[0] + p[1] + p[2]
}

#[test]
fn a_black_hole_is_drawn_with_its_shadow_and_a_disk_round_it() {
    // seen almost along its axis, so that the disk's near side is not in front of the hole
    let doc = scene_at(5.0, r#"geodesics="true""#, "", "", "");
    let Some(r) = common::render_times(&doc, &[0.0]) else { return };
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    let (cx, cy) = (r.size[0] / 2, r.size[1] / 2);
    // the middle of the image is the shadow: black, and opaque
    let centre = r.at(cx, cy);
    assert!(luma(centre) < 1e-3 && centre[3] > 0.99, "the shadow: {centre:?}");
    // the disk is bright where the camera sees it, and the picture is not empty
    let bright = r.px.iter().filter(|p| luma(**p) > 0.05).count();
    assert!(bright > 500, "the disk covers part of the image: {bright} pixels");
    // the photon sphere's apparent radius: sqrt(27) M over the distance, in pixels of a 40 degree lens
    let focal = 128.0 / (2.0 * 20.0f32.to_radians().tan());
    let shadow = focal * (27.0f32.sqrt() / 60.0);
    let inside = (0..r.size[1])
        .flat_map(|y| (0..r.size[0]).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            ((x as f32 + 0.5 - cx as f32).powi(2) + (y as f32 + 0.5 - cy as f32).powi(2)).sqrt() < 0.8 * shadow
        })
        .filter(|&(x, y)| luma(r.at(x, y)) > 0.05)
        .count();
    println!("shadow radius {shadow:.1} px; {inside} bright pixels inside 0.8 of it");
    assert_eq!(inside, 0, "nothing bright inside the shadow");
}

#[test]
fn the_pattern_of_the_disk_turns_with_the_time_of_the_frame_and_a_smooth_disk_does_not_change() {
    let clumps = scene(r#"geodesics="true""#, "", r#"angularPattern="clumps" timeScale="8""#, "");
    let smooth = scene(r#"geodesics="true""#, "", r#"angularPattern="none" timeScale="8""#, "");
    let (Some(a), Some(b), Some(c), Some(d)) = (
        common::render_times(&clumps, &[0.0]),
        common::render_times(&clumps, &[3.0]),
        common::render_times(&smooth, &[0.0]),
        common::render_times(&smooth, &[3.0]),
    ) else {
        return;
    };
    let differ = |x: &common::Rendered, y: &common::Rendered| {
        x.px.iter().zip(&y.px).filter(|(p, q)| (luma(**p) - luma(**q)).abs() > 0.02 * luma(**p).max(0.05)).count()
    };
    println!("clumps differ in {} pixels, a smooth disk in {}", differ(&a, &b), differ(&c, &d));
    assert!(differ(&a, &b) > 300, "the clumps have moved");
    assert_eq!(differ(&c, &d), 0, "a smooth disk looks the same at any time");
}

#[test]
fn a_black_hole_seen_by_an_ordinary_camera_is_not_drawn_and_says_so() {
    let doc = scene("", "", "", "");
    let Some(r) = common::render_times(&doc, &[0.0]) else { return };
    assert!(r.px.iter().all(|p| luma(*p) < 1e-3), "nothing is drawn");
    assert!(
        r.stats.unsupported.iter().any(|m| m.starts_with("hole: ") && m.contains("geodesics")),
        "{:?}",
        r.stats.unsupported
    );
}
