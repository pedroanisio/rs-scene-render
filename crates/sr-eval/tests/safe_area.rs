//! `safeArea enforce`: text and nodes tagged `cta` or `logo` must stay inside the safe region
//! (schema/scene-render-1.1.xsd, safeAreaType).

use sr_eval::safe_area::{audit, Finding};
use sr_eval::SafeEnforce;

/// 1080 x 1920 with youtube-shorts insets: top 8 %, right 12 %, bottom 25 %, left 5 %,
/// that is the region x 54..950, y 154..1440.
fn scene(enforce: &str, assets: &str, nodes: &str) -> sr_eval::Evaluator {
    let xml = format!(
        r##"<scene version="1.1"><project width="1080" height="1920" fps="24" duration="1" background="#000000" safeArea="sa"/>
        <safeAreas><safeArea id="sa" preset="youtube-shorts" {enforce}/></safeAreas>
        {assets}<composition>{nodes}</composition></scene>"##
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
}

fn findings(ev: &sr_eval::Evaluator, t: f64) -> Vec<Finding> {
    audit(ev.program(), &ev.evaluate(t))
}

const CTA_OUT: &str =
    r##"<shape id="cta1" shape="rect" x="0" y="1850" width="200" height="60" fill="#FF0000" tags="cta"/>"##;
const CTA_IN: &str =
    r##"<shape id="cta1" shape="rect" x="500" y="700" width="200" height="60" fill="#FF0000" tags="cta"/>"##;

#[test]
fn enforce_is_read_from_the_safe_area_the_project_names() {
    for (attr, want) in [
        (r#"enforce="error""#, SafeEnforce::Error),
        (r#"enforce="warn""#, SafeEnforce::Warn),
        (r#"enforce="off""#, SafeEnforce::Off),
        ("", SafeEnforce::Warn), // the schema default
    ] {
        assert_eq!(scene(attr, "", CTA_IN).program().safe_enforce, want, "{attr}");
    }
}

#[test]
fn a_cta_node_outside_the_region_is_a_finding() {
    let ev = scene(r#"enforce="error""#, "", CTA_OUT);
    let f = findings(&ev, 0.0);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].id, "cta1");
    assert_eq!(f[0].kind, "cta");
    assert_eq!(f[0].side, "bottom");
    assert!(f[0].overshoot > 400.0, "{f:?}"); // box bottom 1910 against the limit 1440
}

#[test]
fn a_logo_node_is_checked_like_a_cta() {
    let ev = scene(
        r#"enforce="warn""#,
        "",
        r##"<shape id="mark" shape="rect" x="0" y="0" width="80" height="80" fill="#FFFFFF" tags="brand logo"/>"##,
    );
    let f = findings(&ev, 0.0);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!((f[0].id.as_str(), f[0].kind), ("mark", "logo"));
}

#[test]
fn a_node_inside_the_region_is_clean() {
    assert!(findings(&scene(r#"enforce="error""#, "", CTA_IN), 0.0).is_empty());
}

#[test]
fn untagged_shapes_are_not_checked() {
    let ev = scene(
        r#"enforce="error""#,
        "",
        r##"<shape id="bg" shape="rect" x="0" y="1850" width="1080" height="70" fill="#202020"/>"##,
    );
    assert!(findings(&ev, 0.0).is_empty());
}

#[test]
fn enforce_off_reports_nothing() {
    assert!(findings(&scene(r#"enforce="off""#, "", CTA_OUT), 0.0).is_empty());
}

#[test]
fn a_document_without_a_safe_area_reports_nothing() {
    let xml = format!(
        r#"<scene version="1.1"><project width="1080" height="1920" fps="24" duration="1"/><composition>{CTA_OUT}</composition></scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    assert_eq!(ev.program().safe_enforce, SafeEnforce::Off);
    assert!(findings(&ev, 0.0).is_empty());
}

#[test]
fn a_text_layer_outside_the_region_is_a_finding_without_any_tag() {
    let ev = scene(
        r#"enforce="error""#,
        r##"<assets><text id="words" text="Hello" width="300" height="60" size="40" color="#FFFFFF"/></assets>"##,
        r##"<layer id="title" asset="words" x="500" y="1800"/>"##,
    );
    let f = findings(&ev, 0.0);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!((f[0].id.as_str(), f[0].kind), ("title", "text"));
}

#[test]
fn a_text_layer_inside_the_region_is_clean() {
    let ev = scene(
        r#"enforce="error""#,
        r##"<assets><text id="words" text="Hello" width="300" height="60" size="40" color="#FFFFFF"/></assets>"##,
        r##"<layer id="title" asset="words" x="500" y="700"/>"##,
    );
    assert!(findings(&ev, 0.0).is_empty());
}

#[test]
fn a_node_is_checked_where_it_is_at_that_time() {
    // enters the region from below: outside at 0 s, inside at 1 s
    let ev = scene(
        r#"enforce="error""#,
        "",
        r##"<shape id="cta1" shape="rect" x="500" y="1850" width="200" height="60" fill="#FF0000" tags="cta">
              <animate property="y"><key time="0" value="1850"/><key time="1" value="700"/></animate></shape>"##,
    );
    assert_eq!(findings(&ev, 0.0).len(), 1);
    assert!(findings(&ev, 1.0).is_empty());
}

#[test]
fn a_hidden_node_is_not_a_finding() {
    let ev = scene(
        r#"enforce="error""#,
        "",
        r##"<shape id="cta1" shape="rect" x="0" y="1850" width="200" height="60" fill="#FF0000" tags="cta" opacity="0"/>"##,
    );
    assert!(findings(&ev, 0.0).is_empty());
}

const NOTCH: &str = r##"<symbols><symbol id="notch" width="64" height="64" duration="1"><shape id="mark" shape="rect" width="64" height="64" fill="#FF8000" tags="logo"/></symbol></symbols>"##;
const NOTCH_FORCED: &str = r##"<symbols><symbol id="notch" width="64" height="64" duration="1" safeAreaForce="true"><shape id="mark" shape="rect" width="64" height="64" fill="#FF8000" tags="logo"/></symbol></symbols>"##;

#[test]
fn a_forced_node_is_the_one_exception_and_the_rest_is_still_checked() {
    let forced = r##"<shape id="cta1" shape="rect" x="0" y="1850" width="200" height="60" fill="#FF0000" tags="cta" safeAreaForce="true"/>"##;
    let other = r##"<shape id="cta2" shape="rect" x="0" y="1850" width="200" height="60" fill="#FF0000" tags="cta"/>"##;
    let f = findings(&scene(r#"enforce="error""#, "", &format!("{forced}{other}")), 0.0);
    assert_eq!(f.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["cta2"], "{f:?}");
    // the same node without the force is a finding
    assert_eq!(findings(&scene(r#"enforce="error""#, "", CTA_OUT), 0.0).len(), 1);
}

#[test]
fn a_forced_instance_covers_what_the_symbol_draws() {
    let at = |attr: &str| format!(r##"<instance id="n" symbol="notch" x="0" y="1850" {attr}/>"##);
    let f = findings(&scene(r#"enforce="error""#, NOTCH, &at("")), 0.0);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].id, "n/mark");
    assert!(findings(&scene(r#"enforce="error""#, NOTCH, &at(r#"safeAreaForce="true""#)), 0.0).is_empty());
}

#[test]
fn a_forced_symbol_covers_every_instance_of_it() {
    let two = r##"<instance id="a" symbol="notch" x="0" y="1850"/><instance id="b" symbol="notch" x="0" y="1800"/>"##;
    assert!(findings(&scene(r#"enforce="warn""#, NOTCH_FORCED, two), 0.0).is_empty());
    assert_eq!(findings(&scene(r#"enforce="warn""#, NOTCH, two), 0.0).len(), 2);
}

#[test]
fn a_forced_group_covers_its_children() {
    let g = |attr: &str| format!(r##"<group id="g" {attr}>{CTA_OUT}</group>"##);
    assert_eq!(findings(&scene(r#"enforce="error""#, "", &g("")), 0.0).len(), 1);
    assert!(findings(&scene(r#"enforce="error""#, "", &g(r#"safeAreaForce="true""#)), 0.0).is_empty());
}

#[test]
fn a_text_layer_next_to_a_forced_node_is_still_caught() {
    let ev = scene(
        r#"enforce="error""#,
        &format!(
            r##"<assets><text id="words" text="Hi" width="300" height="60" size="40" color="#FFFFFF"/></assets>{NOTCH}"##
        ),
        r##"<instance id="n" symbol="notch" x="0" y="1850" safeAreaForce="true"/><layer id="title" asset="words" x="500" y="1800"/>"##,
    );
    let f = findings(&ev, 0.0);
    assert_eq!(f.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["title"], "{f:?}");
}

#[test]
fn forced_nodes_are_listed_so_a_force_never_hides() {
    let ev = scene(
        r#"enforce="error""#,
        NOTCH,
        r##"<instance id="n" symbol="notch" x="0" y="1850" safeAreaForce="true"/><shape id="fine" shape="rect" x="500" y="700" width="20" height="20" fill="#FF0000" safeAreaForce="true"/>"##,
    );
    let mut ids = sr_eval::safe_area::forced(ev.program(), &ev.evaluate(0.0));
    ids.sort();
    assert_eq!(ids, ["fine", "n"]);
}

fn long_scene(nodes: &str) -> sr_eval::Evaluator {
    // 400 s at 24 fps: 9600 frames, far more than one audit evaluates
    let xml = format!(
        r##"<scene version="1.1"><project width="1080" height="1920" fps="24" duration="400" background="#000000" safeArea="sa"/>
        <safeAreas><safeArea id="sa" preset="youtube-shorts" enforce="error"/></safeAreas>
        <composition>{nodes}</composition></scene>"##
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
}

#[test]
fn a_long_timeline_is_sampled_and_says_so_but_a_keyed_instant_outside_is_still_caught() {
    // outside the region for exactly one key instant (t = 250 s), inside before and after
    let ev = long_scene(
        r##"<shape id="cta1" shape="rect" x="500" y="700" width="200" height="60" fill="#FF0000" tags="cta">
              <animate property="y">
                <key time="0" value="700" interpolation="hold"/><key time="250" value="1850" interpolation="hold"/><key time="250.0416667" value="700"/>
              </animate></shape>"##,
    );
    let s = sr_eval::safe_area::sample_times(ev.program(), 0.0, 400.0);
    assert_eq!(s.frames, 9600);
    assert!(s.times.len() < 600, "{}", s.times.len());
    assert_eq!(s.note().as_deref(), Some(format!("sampled {} of 9600 frames", s.times.len()).as_str()));
    let hit: Vec<_> = s
        .times
        .iter()
        .filter(|&&t| !sr_eval::safe_area::audit(ev.program(), &ev.evaluate_layout(t)).is_empty())
        .collect();
    // 250.0 s, and possibly the next frame, which sits on the instant the key returns inside
    assert!(!hit.is_empty() && hit.len() <= 2, "{hit:?}");
    assert!(hit.iter().all(|t| (**t - 250.0).abs() < 0.05), "{hit:?}");
}

#[test]
fn a_short_timeline_is_audited_frame_by_frame_with_no_note() {
    let ev = scene(r#"enforce="error""#, "", CTA_IN);
    let s = sr_eval::safe_area::sample_times(ev.program(), 0.0, 1.0);
    assert_eq!(s.times.len(), 24);
    assert!(s.note().is_none());
}

#[test]
fn a_static_document_with_nothing_to_check_costs_the_grid_only() {
    let ev = long_scene(r##"<shape id="a" shape="rect" width="10" height="10" fill="#FF0000"/>"##);
    let s = sr_eval::safe_area::sample_times(ev.program(), 0.0, 400.0);
    assert!(s.times.len() <= 70, "{}", s.times.len());
}
