//! A document material's `unevenness` attributes reach the 3D renderer.

mod common;
use common::*;

fn scene(material: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.1"><project width="128" height="128" fps="10" duration="2" background="#00000000"/>
        <materials><material id="clay" baseColor="#C48A5A" roughness="0.85" sheenColor="#FFE0C0" sheenRoughness="0.6" {material}/></materials>
        <composition><object3D id="s" primitive="sphere" radius="30" x="64" y="64" material="clay"/></composition>
        <lights><light id="sun" type="directional" intensity="3" yaw="45"/></lights></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

#[test]
fn unevenness_attributes_change_the_shading_and_off_does_not() {
    let Some(plain) = render(&scene("")) else { return };
    let Some(off) = render(&scene(r#"unevenness="0" unevennessScale="4" unevennessSeed="9""#)) else { return };
    let Some(uneven) = render(&scene(r#"unevenness="0.6" unevennessScale="6" unevennessSeed="2""#)) else { return };
    assert_eq!(plain.px, off.px, "off is the baseline");
    let differs =
        plain.px.iter().zip(&uneven.px).filter(|(a, b)| (a[0] + a[1] + a[2] - b[0] - b[1] - b[2]).abs() > 0.02).count();
    assert!(differs > 300, "{differs}");
    let problems: Vec<_> = uneven.stats.unsupported.iter().chain(&uneven.stats.errors).collect();
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn a_static_object_has_the_same_finish_in_every_frame() {
    // the noise is a function of the object's position and the seed only: no time, no frame, no history
    let doc = scene(r#"unevenness="0.6" unevennessScale="6" unevennessSeed="2""#);
    let Some(first) = render_times(&doc, &[0.0]) else { return };
    let Some(later) = render_times(&doc, &[0.0, 0.1, 0.7, 1.9]) else { return };
    assert_eq!(first.px, later.px);
}
