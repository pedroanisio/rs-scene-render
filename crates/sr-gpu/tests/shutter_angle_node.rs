//! `shutterAngle` on a node: its motion blur uses that angle instead of the project's, inherited from the nearest parent.

mod common;
use common::*;

fn doc(node: &str, group: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="160" height="40" fps="10" duration="2" background="#000000" motionBlur="true" shutterAngle="180" shutterPhase="0" motionBlurSamples="16"/>
        <composition><group id="g" width="160" height="40" {group}>
          <shape id="s" shape="rect" y="16" width="8" height="8" fill="#FFFFFF" {node}><animate property="x"><key time="0" value="10"/><key time="1" value="110"/></animate></shape>
        </group></composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: false, base_dir: None }).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// Width in pixels of the smear on the row through the square: columns with any ink.
fn smear(d: &sr_model::Document) -> Option<usize> {
    let r = render_sub(d, 0.3)?;
    Some((0..r.size[0]).filter(|&x| r.at(x, 20)[0] > 0.02).count())
}

#[test]
fn the_nodes_shutter_angle_replaces_the_projects() {
    let Some(project) = smear(&doc("", "")) else { return };
    let wide = smear(&doc(r#"shutterAngle="360""#, "")).unwrap();
    let none = smear(&doc(r#"shutterAngle="0""#, "")).unwrap();
    // 100 px per second at 10 fps: a 180 degree shutter smears about 5 px, 360 about 10, 0 nothing
    assert_eq!(none, 8, "a sharp square is its own width");
    assert!(project > 10 && project <= 16, "{project}");
    assert!(wide > project + 3, "360 degrees smears more than 180: {wide} vs {project}");
}

#[test]
fn a_parent_sets_it_for_its_children() {
    let Some(project) = smear(&doc("", "")) else { return };
    let from_group = smear(&doc("", r#"shutterAngle="360""#)).unwrap();
    assert!(from_group > project + 3, "{from_group} vs {project}");
    // the node's own angle beats its parent's
    let own = smear(&doc(r#"shutterAngle="0""#, r#"shutterAngle="360""#)).unwrap();
    assert_eq!(own, 8);
}
