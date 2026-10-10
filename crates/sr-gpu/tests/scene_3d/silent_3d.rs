//! Two 3D mistakes that render without any error, measured per frame for the render report: an `object3D` outside its
//! camera's view, and a metallic material with no environment to reflect. Neither changes the picture.

use super::common;
use common::*;

/// A 128×128 scene under the default camera (looking along +z from z ≈ −111, the frame's centre at x = y = 64).
fn scene(materials: &str, body: &str, lights: &str) -> sr_model::Document {
    let mats = if materials.is_empty() { String::new() } else { format!("<materials>{materials}</materials>") };
    let ls = if lights.is_empty() { String::new() } else { format!("<lights>{lights}</lights>") };
    let assets = ASSETS;
    let xml = format!(
        r##"<scene version="1.1"><project width="128" height="128" fps="10" duration="4" background="#00000000"/>{assets}{mats}<composition>{body}</composition>{ls}</scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn in_view(body: &str) -> Option<Vec<(String, bool)>> {
    render(&scene("", body, "")).map(|r| r.stats.objects3d_in_view)
}

#[test]
fn an_object_outside_the_camera_frustum_is_out_of_view() {
    let Some(seen) = in_view(r#"<object3D id="o" primitive="box" width="20" height="20" depth="20" x="64" y="64"/>"#)
    else {
        return;
    };
    assert_eq!(seen, [("o".to_string(), true)]);
    for (place, attrs) in [
        ("behind the camera", r#"x="64" y="64" z="-1000""#),
        ("far to the right", r#"x="5000" y="64""#),
        ("far above", r#"x="64" y="-5000""#),
        ("beyond the far plane", r#"x="64" y="64" z="20000""#),
    ] {
        let body = format!(r#"<object3D id="o" primitive="box" width="20" height="20" depth="20" {attrs}/>"#);
        assert_eq!(in_view(&body).unwrap(), [("o".to_string(), false)], "{place}");
    }
    // half in the frame is in view: only a box wholly beyond one side of the frustum is out
    let edge = r#"<object3D id="o" primitive="box" width="40" height="20" depth="20" x="130" y="64"/>"#;
    assert_eq!(in_view(edge).unwrap(), [("o".to_string(), true)]);
}

#[test]
fn the_camera_that_renders_decides() {
    // a camera turned away from the object (yaw 180°): the object at the frame's centre is behind it
    let cam = r#"<camera id="cam" x="64" y="64" z="-111" yaw="180"/>"#;
    let body = format!(r#"{cam}<object3D id="o" primitive="sphere" radius="10" x="64" y="64"/>"#);
    let Some(seen) = in_view(&body) else { return };
    assert_eq!(seen, [("o".to_string(), false)]);
}

const CHROME: &str = r##"<material id="chrome" baseColor="#D0D0D0" metallic="1" roughness="0.15"/>"##;

fn metal(materials: &str, material: &str, lights: &str) -> Option<Rendered> {
    let body = format!(r#"<object3D id="ball" primitive="sphere" radius="30" x="64" y="64" material="{material}"/>"#);
    render(&scene(materials, &body, lights))
}

#[test]
fn a_metal_with_no_environment_is_noted_and_drawn_as_before() {
    let Some(r) = metal(CHROME, "chrome", "") else { return };
    assert_eq!(r.stats.metal_without_environment, [("ball".to_string(), 1.0)]);
    // a directional light alone is no environment either
    let sun = r#"<light id="sun" type="directional" rotationX="40"/>"#;
    let lit = metal(CHROME, "chrome", sun).unwrap();
    assert_eq!(lit.stats.metal_without_environment.len(), 1);
    // nor is a dome without an image: it lights uniformly, as an ambient light
    let plain_dome = metal(CHROME, "chrome", r#"<light id="sky" type="dome"/>"#).unwrap();
    assert_eq!(plain_dome.stats.metal_without_environment.len(), 1);
}

#[test]
fn an_environment_image_or_a_dielectric_is_not_noted() {
    let Some(dome) = metal(CHROME, "chrome", r#"<light id="sky" type="dome" environment="sky.png"/>"#) else { return };
    assert!(dome.stats.metal_without_environment.is_empty(), "{:?}", dome.stats.metal_without_environment);
    // below the threshold of 0.5 the diffuse response dominates and the form reads under lights
    let satin = r##"<material id="satin" baseColor="#D0D0D0" metallic="0.4" roughness="0.15"/>"##;
    assert!(metal(satin, "satin", "").unwrap().stats.metal_without_environment.is_empty());
    let unlit = r##"<material id="flat" baseColor="#D0D0D0" metallic="1" unlit="true"/>"##;
    assert!(metal(unlit, "flat", "").unwrap().stats.metal_without_environment.is_empty());
}
