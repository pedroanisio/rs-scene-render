//! The axis of a crater and the speed it is made by are those of the surface that was hit, not
//! of the triangle's edge that a contact was reported against: the same ground in finer or
//! coarser triangles gives the same crater.

use sr_eval::Evaluator;

const GRAVITY: f64 = 9.80665;
const FLIGHT: f64 = 1.4888;

/// A rock of 2 m radius and 90 478 kg that arrives at the origin at `speed` m/s, `angle` degrees
/// above the ground (the launch is computed so that it arrives so), `offset` metres along x
/// from the origin, on `ground`.
fn scene(speed: f64, angle: f64, offset: f64, ground: &str) -> String {
    let (vx, vy) = (speed * angle.to_radians().cos(), speed * angle.to_radians().sin());
    let launch = vy - GRAVITY * FLIGHT;
    let (x, y) = (offset - vx * FLIGHT, -2.0 - launch * FLIGHT - 0.5 * GRAVITY * FLIGHT * FLIGHT);
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="4"/><composition>
          <object3D id="rock" primitive="sphere" radius="2" x="{x}" y="{y}">
            <rigidBody shape="sphere" mass="90478" velocityX="{vx}" velocityY="{launch}" restitution="0" linearDamping="0" angularDamping="0"/>
          </object3D>
          {ground}
        </composition>
        <physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"##
    )
}

fn plane(segments: u32) -> String {
    format!(
        r#"<object3D id="ground" primitive="plane" width="160" height="160" segments="{segments}" y="0" rotationX="-90">
             <crater source="rock" targetMaterial="softRock"/>
             <rigidBody type="static" shape="auto"/>
           </object3D>"#
    )
}

fn impact(xml: &str) -> (Vec<f64>, [f64; 3], f64, [f64; 3]) {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let ev = Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(2.5);
    assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
    let ground =
        frame.nodes.iter().find(|n| &*n.id != "rock" && n.crater_impact.is_some()).expect("a crater from the impact");
    let grown = ground.crater_impact.as_deref().unwrap();
    (vec![grown.impactor().normal_speed, grown.speed()], grown.spec.outward, grown.law().radius, grown.spec.center)
}

#[test]
fn the_same_ground_in_different_triangles_gives_the_same_axis_and_the_authored_normal_speed() {
    let authored = 100.0 * 60f64.to_radians().sin();
    for segments in [8, 16, 64, 160] {
        let (speeds, axis, _, _) = impact(&scene(100.0, 60.0, 0.0, &plane(segments)));
        println!("NORMAL {segments} segments: normal speed {:.2} (authored {authored:.2}), axis {axis:?}", speeds[0]);
        assert!(axis[2] < -1.0 + 1e-9, "{segments} segments: the ground is flat, the axis is its normal: {axis:?}");
        assert!(
            (speeds[0] - authored).abs() < 0.005 * authored,
            "{segments} segments: {} against {authored}",
            speeds[0]
        );
    }
}

#[test]
fn a_curved_ground_gives_the_normal_that_is_local_to_where_the_rock_hit() {
    // a planet of 60 m radius, hit on its flank
    let planet = r#"<object3D id="planet" primitive="sphere" radius="60" segments="96">
             <crater source="rock" targetMaterial="softRock"/>
             <rigidBody type="static" shape="trimesh"/>
           </object3D>"#;
    // the rock arrives from above at an offset of 25 m: the ground there is tilted by about 25 degrees
    let xml = scene(100.0, 90.0, 25.0, planet);
    let (_, axis, _, centre) = impact(&xml);
    let length = centre.iter().map(|c| c * c).sum::<f64>().sqrt();
    let radial = centre.map(|c| c / length);
    let tilt = axis.iter().zip(&radial).map(|(a, r)| a * r).sum::<f64>().abs().min(1.0).acos().to_degrees();
    println!("NORMAL planet: axis {axis:?}, radial {radial:?}, {tilt:.2} degrees apart");
    assert!(tilt < 1.5, "the axis is the planet's local normal: {tilt} degrees");
}
