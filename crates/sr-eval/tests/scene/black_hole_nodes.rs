//! The black hole and its disk are nodes of the composition, with the attributes the renderer reads, and the camera
//! that traces geodesics says so.

use sr_eval::Evaluator;

const SCENE: &str = r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition>
  <shape id="sky" shape="rect" x="0" y="0" width="64" height="64" fill="#000000"/>
  <camera id="eye" x="0" y="0" z="-60" geodesics="true"/>
  <blackHole id="hole" mass="1.5" x="1" y="2" z="3"/>
  <accretionDisk id="disk" blackHole="hole" outerRadius="30" temperatureScale="6000" rotationX="10" seed="4"/>
</composition></scene>"##;

#[test]
fn the_hole_the_disk_and_the_camera_are_read_from_the_frame() {
    let doc = sr_model::load_str(SCENE, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let ev = Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"));
    let frame = ev.evaluate(0.5);
    assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
    let kinds: Vec<(&str, &str)> = frame.nodes.iter().map(|n| (&*n.id, n.kind)).collect();
    assert!(kinds.contains(&("hole", "blackHole")), "{kinds:?}");
    assert!(kinds.contains(&("disk", "accretionDisk")), "{kinds:?}");
    // what the renderer reads is on the node's element, with the defaults the schema gives
    let attr = |id: &str, name: &str| frame.nodes.iter().find(|n| &*n.id == id).unwrap().elem.get_attr(name);
    use sr_model::element::{
        AttrValue::{Bool, Num},
        Element,
    };
    assert_eq!(attr("hole", "mass"), Some(Num(1.5)));
    assert_eq!(attr("disk", "outerRadius"), Some(Num(30.0)));
    assert_eq!(attr("disk", "rotationX"), Some(Num(10.0)));
    assert_eq!(attr("disk", "contrast"), Some(Num(0.5)));
    assert_eq!(attr("eye", "geodesics"), Some(Bool(true)));
}
