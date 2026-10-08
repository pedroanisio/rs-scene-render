//! A coupled group (an ocean and the rigid bodies it carries, and what reads them) may run on a clock that is not
//! the composition's if the whole group runs on the same one: a group that scales time shows the group's time
//! stretched, and everything in it, the world, the water and what drifts in it, is what the same scene is at
//! the time the clock gives, bit for bit. A member on another clock is an error that names both.

use sr_eval::{Evaluator, FrameGraph};

const BALL: &str = r#"<object3D id="float" primitive="sphere" radius="1" segments="24" y="-6">
    <rigidBody shape="sphere" mass="2094" linearDamping="0" angularDamping="0"/>
  </object3D>"#;
const SEA: &str = r#"<ocean id="sea" bedResponse="hydrostatic" width="16" depth="16" cellSize="1" bottomDepth="20" dt="0.05" boundary="closed" colliders="float" bodyCoupling="full"/>"#;
const DUST: &str = r#"<particles3D id="dust" x="0.2" y="-9" rate="0" gravityY="9.8" collisionRadius="0.1" bounce="0.3" dt="0.05" lifetime="6" colliders="float" seed="5"><burst time="0" count="6"/></particles3D>"#;

fn xml(body: &str, sea: &str, dust: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>{body}{sea}{dust}</composition>
        <physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"##
    )
}

fn evaluator(xml: &str) -> Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn at(ev: &Evaluator, t: f64) -> FrameGraph {
    let frame = ev.evaluate(t);
    assert!(
        frame.problems.is_empty() && frame.failures.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.problems,
        frame.failures
    );
    frame
}

/// Everything the group shows at `t`, as bits: the ball's pose, the water, the particles.
fn shown(ev: &Evaluator, t: f64) -> Vec<u64> {
    let frame = at(ev, t);
    let mut bits = Vec::new();
    for node in &frame.nodes {
        if &*node.id == "float" {
            bits.extend(node.pose3.expect("simulated").map(f64::to_bits));
        }
        if let Some(sea) = &node.sim_ocean {
            bits.push(sea.key);
            bits.extend(sea.frame.cells.iter().map(|c| c.depth.to_bits()));
        }
        if let Some(dust) = &node.particles3d {
            for p in &dust.frame.particles {
                bits.extend(p.position.map(f64::to_bits));
            }
        }
    }
    bits
}

fn grouped(scale: f64, inner: &str) -> String {
    format!(r#"<group id="clock{}" timeScale="{scale}">{inner}</group>"#, scale.to_bits() % 1000)
}

/// The ball's height and how many particles there are, to see that something happens in the scene being compared.
fn happening(ev: &Evaluator, t: f64) -> (f64, usize) {
    let frame = at(ev, t);
    let height = frame.nodes.iter().find(|n| &*n.id == "float").and_then(|n| n.pose3).expect("simulated")[13];
    let particles = frame.nodes.iter().filter_map(|n| n.particles3d.as_ref()).map(|d| d.frame.particles.len()).sum();
    (height, particles)
}

/// The scene with all of it in a group of `scale`, and the same scene with no group.
fn pair(scale: f64, dust: &str) -> (Evaluator, Evaluator) {
    let inner = format!("{BALL}{SEA}{dust}");
    (evaluator(&xml(&grouped(scale, &inner), "", "")), evaluator(&xml(&inner, "", "")))
}

#[test]
fn a_group_that_slows_time_shows_the_scene_at_the_time_its_clock_gives() {
    let (slow, plain) = pair(0.5, "");
    // the ball falls from 6 units over the water: at 2 s of the group's time it has not fallen yet and at 4 s it has
    assert!(happening(&slow, 4.0).0 > happening(&slow, 0.0).0 + 5.0, "the ball does not fall");
    assert_ne!(shown(&slow, 1.0), shown(&slow, 4.0));
    for t in [1.0, 2.0, 3.0, 4.0] {
        assert_eq!(shown(&slow, t), shown(&plain, 0.5 * t), "t = {t}");
    }
    // the group does something the plain scene at the same time does not
    assert_ne!(shown(&slow, 3.0), shown(&plain, 3.0));
}

#[test]
fn a_group_that_speeds_time_up_shows_the_scene_at_the_time_its_clock_gives() {
    let (fast, plain) = pair(2.0, "");
    for t in [0.25, 0.5, 0.75, 1.0] {
        assert_eq!(shown(&fast, t), shown(&plain, 2.0 * t), "t = {t}");
    }
}

#[test]
fn what_drifts_in_the_group_reads_the_world_at_the_time_of_the_group() {
    let (slow, plain) = pair(0.5, DUST);
    assert_eq!(happening(&slow, 3.0).1, 6, "the burst is not there");
    for t in [1.0, 3.0, 4.0] {
        assert_eq!(shown(&slow, t), shown(&plain, 0.5 * t), "t = {t}");
    }
}

#[test]
fn nested_groups_with_the_same_total_scale_are_the_same_clock() {
    let inner = format!("{BALL}{SEA}");
    let one = evaluator(&xml(&grouped(0.5, &inner), "", ""));
    let nested = evaluator(&xml(
        &format!(r#"<group id="outer" timeScale="0.25"><group id="inner" timeScale="2">{inner}</group></group>"#),
        "",
        "",
    ));
    for t in [1.0, 2.0, 4.0] {
        assert_eq!(shown(&one, t), shown(&nested, t), "t = {t}");
    }
}

#[test]
fn any_order_of_instants_and_a_fresh_evaluator_give_the_same_bits() {
    let (slow, _) = pair(0.5, DUST);
    let times = [4.0, 1.0, 3.0, 2.0, 4.0, 0.5];
    let first: Vec<_> = times.iter().map(|&t| shown(&slow, t)).collect();
    let (fresh, _) = pair(0.5, DUST);
    for k in [3, 0, 5, 2, 1, 4] {
        assert_eq!(first[k], shown(&fresh, times[k]), "t = {}", times[k]);
    }
    assert_eq!(first[0], first[4]);
}

fn complaint(xml: &str) -> String {
    let frame = evaluator(xml).evaluate(1.0);
    frame.problems.iter().chain(&frame.failures).cloned().collect::<Vec<_>>().join("; ")
}

#[test]
fn a_member_on_another_clock_is_an_error_that_names_both() {
    // the water on a clock that the ball it carries is not on
    let outside = complaint(&xml(BALL, &grouped(0.5, SEA), ""));
    assert!(outside.contains("sea") && outside.contains("float"), "{outside:?}");
    // the ball and the water on different scales
    let different = complaint(&xml(&grouped(0.5, BALL), &grouped(2.0, SEA), ""));
    assert!(different.contains("sea") && different.contains("float"), "{different:?}");
    // what drifts in the group on another clock than the water and the bodies it collides with
    let reader = complaint(&xml(&grouped(0.5, &format!("{BALL}{SEA}")), "", &grouped(2.0, DUST)));
    assert!(reader.contains("dust") && reader.contains("sea"), "{reader:?}");
}

#[test]
fn a_body_of_the_world_that_is_not_in_the_group_clock_is_an_error_too() {
    // the world is one: a second body on the composition clock would see it at the wrong time
    let other = r#"<object3D id="stone" primitive="sphere" radius="1" segments="12" x="8" y="-20"><rigidBody shape="sphere" mass="100"/></object3D>"#;
    let bad = complaint(&xml(&format!("{}{other}", grouped(0.5, &format!("{BALL}{SEA}"))), "", ""));
    assert!(bad.contains("stone") && bad.contains("sea"), "{bad:?}");
}

fn with_clock(attributes: &str, inner: &str) -> Evaluator {
    evaluator(&xml(&format!(r#"<group id="clock" {attributes}>{inner}</group>"#), "", ""))
}

#[test]
fn a_scale_that_is_not_a_power_of_two_is_the_scene_at_the_time_its_clock_gives() {
    let inner = format!("{BALL}{SEA}{DUST}");
    let plain = evaluator(&xml(&inner, "", ""));
    for scale in [0.3, 1.5, 0.8] {
        let group = with_clock(&format!(r#"timeScale="{scale}""#), &inner);
        for t in [1.0, 2.5, 3.5] {
            assert_eq!(shown(&group, t), shown(&plain, scale * t), "scale {scale}, t = {t}");
        }
    }
}

#[test]
fn a_group_that_starts_late_shows_the_scene_from_the_start_of_the_world() {
    let inner = format!("{BALL}{SEA}{DUST}");
    let plain = evaluator(&xml(&inner, "", ""));
    // the group's time is the composition's less one second, and half as fast
    let group = with_clock(r#"timeOffset="1" timeScale="0.5""#, &inner);
    for t in [1.0, 2.0, 3.0, 5.0] {
        assert_eq!(shown(&group, t), shown(&plain, 0.5 * (t - 1.0)), "t = {t}");
    }
}

#[test]
fn the_world_of_a_group_starts_from_the_scene_at_the_start_of_the_group_clock() {
    // a body that animation had somewhere else before the world began is where the world finds it at its start,
    // on the group's clock: a group behind the composition by a second starts its world a second in
    let moved = BALL.replace(
        "<rigidBody",
        r#"<animate property="x"><key time="-1" value="-3"/><key time="0" value="0"/><key time="2" value="3"/></animate><rigidBody"#,
    );
    let inner = format!("{moved}{SEA}");
    let plain = evaluator(&xml(&inner, "", ""));
    let group = with_clock(r#"timeOffset="1""#, &inner);
    for t in [1.5, 2.5, 4.0] {
        assert_eq!(shown(&group, t), shown(&plain, t - 1.0), "t = {t}");
    }
}

#[test]
fn a_clock_that_puts_the_start_of_the_world_before_the_composition_is_an_error() {
    let inner = format!("{BALL}{SEA}");
    let complaint = complaint(&xml(&format!(r#"<group id="early" timeOffset="-1">{inner}</group>"#), "", ""));
    assert!(complaint.contains("sea") && complaint.contains("before the composition"), "{complaint:?}");
}
