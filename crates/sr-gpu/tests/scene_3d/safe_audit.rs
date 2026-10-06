//! `safeArea enforce` over a render: nodes and burned captions, at the times being rendered.

use sr_gpu::safe_audit::audit;

fn ev(enforce: &str, track_attrs: &str, extra_safe_areas: &str) -> sr_eval::Evaluator {
    let xml = format!(
        r##"<scene version="1.1"><project width="1080" height="1920" fps="24" duration="4" background="#000000" safeArea="sa"/>
        <safeAreas><safeArea id="sa" preset="youtube-shorts" {enforce}/>{extra_safe_areas}</safeAreas>
        <composition>
          <shape id="cta1" shape="rect" x="500" y="700" width="200" height="60" fill="#FF0000" tags="cta">
            <animate property="y"><key time="0" value="700"/><key time="3" value="1850"/></animate></shape>
        </composition>
        <captions><captionTrack id="cc" language="en" {track_attrs}><cue start="0" end="2" text="Hello there"/></captionTrack></captions></scene>"##
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
}

fn times(n: usize) -> Vec<f64> {
    (0..n).map(|k| k as f64 / 24.0).collect()
}

#[test]
fn a_node_that_leaves_the_region_is_reported_once_at_the_first_frame_outside() {
    let e = ev(r#"enforce="error""#, r#"mode="sidecar""#, "");
    let got = audit(&e, &times(96));
    let cta: Vec<_> = got.iter().filter(|t| t.finding.id == "cta1").collect();
    assert_eq!(cta.len(), 1, "{got:?}");
    // y reaches 1440 - 60 = 1380 (box bottom on the limit) a little after 2 s of the 3 s move
    assert!(cta[0].t > 1.5 && cta[0].t < 2.2, "{:?}", cta[0]);
    assert!(cta[0].finding.overshoot > 0.0);
}

#[test]
fn a_node_that_stays_inside_is_clean() {
    let e = ev(r#"enforce="error""#, r#"mode="sidecar""#, "");
    assert!(audit(&e, &times(24)).is_empty());
}

#[test]
fn a_burned_caption_at_the_default_position_is_outside_a_shorts_region() {
    // the default y is 75 % = 1440 px, the region's bottom limit: the block hangs below it
    let e = ev(r#"enforce="error""#, r#"mode="burn""#, "");
    let got = audit(&e, &times(24));
    let cc: Vec<_> = got.iter().filter(|t| t.finding.kind == "caption").collect();
    assert_eq!(cc.len(), 1, "{got:?}");
    assert_eq!((cc[0].finding.id.as_str(), cc[0].finding.side), ("cc", "bottom"));
}

#[test]
fn a_burned_caption_placed_inside_the_region_is_clean() {
    let e = ev(r#"enforce="error""#, r#"mode="burn" y="40%""#, "");
    assert!(audit(&e, &times(24)).iter().all(|t| t.finding.kind != "caption"));
}

#[test]
fn a_sidecar_caption_is_not_drawn_so_not_checked() {
    let e = ev(r#"enforce="error""#, r#"mode="sidecar""#, "");
    assert!(audit(&e, &times(24)).iter().all(|t| t.finding.kind != "caption"));
}

#[test]
fn a_caption_track_can_name_its_own_safe_area() {
    // the project's area rejects the default caption; the track's own, with no insets, accepts it
    let own = r#"<safeArea id="own" top="0" right="0" bottom="0" left="0" enforce="error"/>"#;
    let e = ev(r#"enforce="error""#, r#"mode="burn" safeArea="own""#, own);
    assert!(audit(&e, &times(24)).iter().all(|t| t.finding.kind != "caption"));
}

#[test]
fn enforce_off_reports_nothing() {
    let e = ev(r#"enforce="off""#, r#"mode="burn""#, "");
    assert!(audit(&e, &times(96)).is_empty());
}
