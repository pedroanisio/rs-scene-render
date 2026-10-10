//! SREP 73: with `orientToVelocity="false"` a flock's sprite keeps rotation 0 in the flock's space, and the node's own
//! rotation still turns it.
//!
//! The SREP's pixel cases `srep-0073-upright-sprite` and `srep-0073-upright-sprite-node-rotation`: one agent in a
//! 1 × 1 box at (320, 180) of a 640 × 360 frame on black, drawn as a 40 px sprite whose left half is red and right
//! half blue (here the fixture `wide.png`). The kit's regions, inset by 3 px for its 2 px tolerance.

mod common;
use common::*;

fn flock(attrs: &str) -> Option<Rendered> {
    let xml = format!(
        r##"<scene version="1.6"><project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>{ASSETS}
  <composition><flock id="f" count="1" seed="3" width="1" height="1" x="320" y="180" shape="sprite" sprite="wide" size="40" {attrs}/></composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    let d = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    let r = render_times(&d, &[0.0])?;
    assert!(
        r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
        "{:?} {:?}",
        r.stats.errors,
        r.stats.unsupported
    );
    Some(r)
}

fn red(p: [f32; 4]) -> bool {
    p[0] > 0.6 && p[2] < 0.3 && p[3] > 0.9
}

fn blue(p: [f32; 4]) -> bool {
    p[2] > 0.6 && p[0] < 0.3 && p[3] > 0.9
}

/// Every pixel of x0..=x1 × y0..=y1 passes `is`.
#[track_caller]
fn region(r: &Rendered, (x0, x1): (u32, u32), (y0, y1): (u32, u32), is: fn([f32; 4]) -> bool, what: &str) {
    for y in y0..=y1 {
        for x in x0..=x1 {
            assert!(is(r.at(x, y)), "({x}, {y}) is {:?}, not {what}", r.at(x, y));
        }
    }
}

#[test]
fn srep_0073_upright_sprite() {
    let Some(r) = flock(r#"orientToVelocity="false""#) else { return };
    // red: centre (310, 180), 20 × 40; blue: centre (330, 180), 20 × 40
    region(&r, (303, 317), (163, 197), red, "red");
    region(&r, (323, 337), (163, 197), blue, "blue");
}

#[test]
fn srep_0073_upright_sprite_node_rotation() {
    let Some(r) = flock(r#"orientToVelocity="false" rotation="90""#) else { return };
    // red: centre (320, 170), 40 × 20; blue: centre (320, 190), 40 × 20
    region(&r, (303, 337), (163, 177), red, "red");
    region(&r, (303, 337), (183, 197), blue, "blue");
}
