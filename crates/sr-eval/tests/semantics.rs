//! Evaluation semantics, one behaviour per test.

use sr_eval::{EvalOptions, Evaluator, FrameGraph, FrameNode, Value};
use sr_model::values::{Length, LengthUnit};

const ASSETS: &str = r#"<assets>
  <image id="img" src="img.png" width="100" height="50"/>
  <video id="vid" src="clip.mp4" width="100" height="50" fps="10" duration="4"/>
  <text id="txt" text="Hello {{name}} #{{index}}: {{item.city}}" width="100" height="20" size="10"/>
</assets>"#;

fn doc(sections: &str, comp: &str) -> sr_model::Document {
    let xml = format!(
        r#"<scene version="1.1"><project width="1000" height="500" fps="10" duration="10" seed="1"/>{sections}{ASSETS}<composition>{comp}</composition></scene>"#
    );
    // sections before assets must precede it in schema order; callers pass only pre-asset sections
    match sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()) {
        Ok(d) => d,
        Err(e) => panic!("invalid test document: {e}\n{xml}"),
    }
}

fn doc_after(after_assets: &str, comp: &str, after: &str) -> sr_model::Document {
    let xml = format!(
        r#"<scene version="1.1"><project width="1000" height="500" fps="10" duration="10" seed="1"/>{ASSETS}{after_assets}<composition>{comp}</composition>{after}</scene>"#
    );
    match sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()) {
        Ok(d) => d,
        Err(e) => panic!("invalid test document: {e}\n{xml}"),
    }
}

fn eval(d: &sr_model::Document, t: f64) -> FrameGraph {
    ev(d, &EvalOptions::default()).evaluate(t)
}

fn ev(d: &sr_model::Document, o: &EvalOptions) -> Evaluator {
    match Evaluator::new(d, o) {
        Ok(e) => e,
        Err(r) => panic!("{r}"),
    }
}

fn node<'f>(f: &'f FrameGraph, id: &str) -> &'f FrameNode {
    f.nodes
        .iter()
        .find(|n| &*n.id == id)
        .unwrap_or_else(|| panic!("no node {id}; have {:?}", f.nodes.iter().map(|n| &n.id).collect::<Vec<_>>()))
}

fn has(f: &FrameGraph, id: &str) -> bool {
    f.nodes.iter().any(|n| &*n.id == id)
}

fn origin(n: &FrameNode) -> [f64; 2] {
    n.world.apply([0.0, 0.0])
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn codes(r: &sr_model::Report) -> Vec<&str> {
    let mut c: Vec<&str> = r.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.code.as_str()).collect();
    c.sort();
    c.dedup();
    c
}

fn err_codes(d: &sr_model::Document, o: &EvalOptions) -> Vec<String> {
    match Evaluator::new(d, o) {
        Ok(_) => Vec::new(),
        Err(r) => codes(&r).into_iter().map(str::to_string).collect(),
    }
}

// ------------------------------------------------------------------ transforms

#[test]
fn transforms_compose_through_groups_and_percentages() {
    let d = doc(
        "",
        r#"<group id="g" x="50%" y="25%" rotation="90" width="200" height="100">
        <layer id="l" asset="img" x="50%" y="10" anchorX="10" scaleX="2"/>
    </group>"#,
    );
    let f = eval(&d, 0.0);
    let g = node(&f, "g");
    assert_eq!(origin(g), [500.0, 125.0]);
    let l = node(&f, "l");
    // child x = 50% of the group box (200) = 100; anchor 10 scaled by 2; rotated 90° clockwise
    let p = l.world.apply([10.0, 0.0]);
    assert!(close(p[0], 500.0 - 10.0) && close(p[1], 125.0 + 100.0), "{p:?}");
    assert_eq!(l.parent, Some(f.nodes.iter().position(|n| &*n.id == "g").unwrap() as u32));
}

#[test]
fn parent_attribute_reparents_transforms() {
    let d = doc(
        "",
        r#"<layer id="a" asset="img" x="100" y="100" rotation="90"/><layer id="b" asset="img" x="10" parent="a"/>"#,
    );
    let f = eval(&d, 0.0);
    let o = origin(node(&f, "b"));
    assert!(close(o[0], 100.0) && close(o[1], 110.0), "{o:?}");
    let cyc = doc("", r#"<layer id="a" asset="img" parent="b"/><layer id="b" asset="img" parent="a"/>"#);
    assert_eq!(err_codes(&cyc, &EvalOptions::default()), ["E11"]);
}

// ------------------------------------------------------------------ keyframes

#[test]
fn keyframes_aliases_additive_and_time_bases() {
    let d = doc(
        "",
        r#"<layer id="l" asset="img" start="2" x="5">
        <animate property="opacity"><key time="2" value="0"/><key time="4" value="1"/></animate>
        <animate property="position"><key time="2" value="0,0%"/><key time="4" value="100,100%"/></animate>
        <animate property="rotation" timeBase="local"><key time="0" value="0"/><key time="1" value="90"/></animate>
        <animate property="scaleX" timeBase="normalized"><key time="0" value="1"/><key time="1" value="3"/></animate>
    </layer>
    <layer id="add" asset="img" x="100"><animate property="x" additive="true"><key time="0" value="0"/><key time="10" value="10"/></animate></layer>"#,
    );
    let f = eval(&d, 3.0);
    let l = node(&f, "l");
    assert_eq!(l.props.get("opacity"), Some(&Value::Num(0.5)));
    assert_eq!(
        l.props.get("position"),
        Some(&Value::Pair([Length::px(50.0), Length { value: 50.0, unit: LengthUnit::Percent }]))
    );
    assert!(close(origin(l)[1], 250.0), "50% of the 500 px frame");
    assert!(close(l.world_opacity, 0.5));
    let rot = l.props.get("rotation").unwrap().as_num().unwrap();
    assert!(close(rot, 90.0), "local time 1 → 90°, got {rot}");
    // normalized: the layer runs from 2 to the composition end (10)
    assert!(close(l.props.get("scaleX").unwrap().as_num().unwrap(), 1.0 + 2.0 * (1.0 / 8.0)));
    assert_eq!(node(&eval(&d, 5.0), "add").props.get("x"), Some(&Value::Len(Length::px(105.0))));
}

#[test]
fn values_clamp_to_their_schema_range() {
    let d = doc(
        "",
        r#"<layer id="l" asset="img"><animate property="opacity" defaultInterpolation="back-out"><key time="0" value="0"/><key time="1" value="1"/></animate></layer>"#,
    );
    let o = node(&eval(&d, 0.5), "l").props.get("opacity").unwrap().as_num().unwrap();
    assert!(o <= 1.0, "back-out overshoot is clamped to unitDecimal: {o}");
}

#[test]
fn animation_errors_are_reported() {
    let d =
        doc("", r#"<layer id="l" asset="img"><animate property="opacty"><key time="0" value="0"/></animate></layer>"#);
    let e = Evaluator::new(&d, &EvalOptions::default()).unwrap_err();
    assert_eq!(codes(&e), ["E02"]);
    assert!(e.diagnostics[0].message.contains("did you mean 'opacity'"), "{}", e.diagnostics[0].message);
    let d = doc(
        "",
        r#"<layer id="l" asset="img"><animate property="opacity"><key time="0" value="lots"/></animate></layer>"#,
    );
    assert_eq!(err_codes(&d, &EvalOptions::default()), ["E04"]);
}

// ------------------------------------------------------------------ expressions and links

#[test]
fn expressions_read_values_other_properties_and_params() {
    let d = doc(
        r#"<parameters><param id="k" type="number" default="3"/></parameters>"#,
        r#"<layer id="a" asset="img" x="10"><expression property="y">prop("b.x") * param("k")</expression></layer>
           <layer id="b" asset="img" x="1"><expression property="x">value + time * 10</expression></layer>
           <layer id="c" asset="img"><expression property="rotation">wiggle(2, 45)</expression>
             <expression property="opacity">clamp(random(), 0, 1)</expression></layer>"#,
    );
    let f = eval(&d, 2.0);
    assert_eq!(node(&f, "b").props.get("x"), Some(&Value::Len(Length::px(21.0))));
    assert_eq!(node(&f, "a").props.get("y"), Some(&Value::Len(Length::px(63.0))), "dependency evaluated first");
    let r1 = node(&f, "c").props.get("rotation").cloned();
    let r2 = node(&eval(&d, 2.0), "c").props.get("rotation").cloned();
    assert_eq!(r1, r2, "wiggle is a pure function of time");
    let o1 = node(&eval(&d, 2.0), "c").props.get("opacity").cloned();
    let o2 = node(&eval(&d, 2.1), "c").props.get("opacity").cloned();
    assert_ne!(o1, o2, "random changes per frame");
}

#[test]
fn expression_cycles_and_compile_errors() {
    let d = doc(
        "",
        r#"<layer id="a" asset="img"><expression property="x">prop("b.x")</expression></layer>
                       <layer id="b" asset="img"><expression property="x">prop("a.x") + 1</expression></layer>"#,
    );
    let e = Evaluator::new(&d, &EvalOptions::default()).unwrap_err();
    assert_eq!(codes(&e), ["E03"]);
    assert!(
        e.diagnostics[0].message.contains("a.x") && e.diagnostics[0].message.contains("b.x"),
        "{}",
        e.diagnostics[0].message
    );
    let d = doc("", r#"<layer id="a" asset="img"><expression property="x">time +* 2</expression></layer>"#);
    assert_eq!(err_codes(&d, &EvalOptions::default()), ["E01"]);
    let d = doc("", r#"<layer id="a" asset="img"><expression property="x">prop("nope.x")</expression></layer>"#);
    assert_eq!(err_codes(&d, &EvalOptions::default()), ["E01"]);
}

#[test]
fn links_scale_offset_clamp_delay_and_markers() {
    let d = doc(
        r#"<parameters><param id="p" type="number" default="4"/></parameters>"#,
        r#"<layer id="src" asset="img"><animate property="x"><key time="0" value="0"/><key time="10" value="100"/></animate></layer>
           <layer id="dst" asset="img"><link property="x" source="src.x" scale="2" offset="1" max="150" delay="1"/>
             <link property="rotation" source="param:p" scale="10"/>
             <link property="opacity" source="marker:m"/></layer>"#,
    );
    // markers come after composition in schema order; rebuild with markers
    let _ = d;
    let d = {
        let xml = format!(
            r#"<scene version="1.1"><project width="1000" height="500" fps="10" duration="10"/><parameters><param id="p" type="number" default="4"/></parameters>{ASSETS}<markers><marker id="m" time="5" duration="2"/></markers><composition>
           <layer id="src" asset="img"><animate property="x"><key time="0" value="0"/><key time="10" value="100"/></animate></layer>
           <layer id="dst" asset="img"><link property="x" source="src.x" scale="2" offset="1" max="150" delay="1"/>
             <link property="rotation" source="param:p" scale="10"/>
             <link property="opacity" source="marker:m"/></layer></composition></scene>"#
        );
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap()
    };
    let f = eval(&d, 4.0);
    let dst = node(&f, "dst");
    assert_eq!(dst.props.get("x"), Some(&Value::Len(Length::px(61.0))), "src.x at t−1 = 30 → 30·2+1");
    assert_eq!(dst.props.get("rotation"), Some(&Value::Num(40.0)));
    assert_eq!(dst.props.get("opacity"), Some(&Value::Num(0.0)));
    assert_eq!(node(&eval(&d, 6.0), "dst").props.get("opacity"), Some(&Value::Num(0.5)));
    assert_eq!(node(&eval(&d, 9.0), "dst").props.get("x"), Some(&Value::Len(Length::px(150.0))), "clamped to max");
}

// ------------------------------------------------------------------ timing

#[test]
fn sequences_place_children_end_to_end() {
    let d = doc(
        "",
        r#"<sequence id="s" timeGap="0.5" start="1">
        <layer id="a" asset="img" end="2"/>
        <layer id="b" asset="img" start="0.25" end="1.25"/>
        <layer id="c" asset="vid"/>
    </sequence>"#,
    );
    let ev = ev(&d, &EvalOptions::default());
    let p = ev.program();
    let span = |id: &str| {
        let n = p.nodes.iter().find(|n| &*n.id == id).unwrap();
        (n.start, n.end)
    };
    assert_eq!(span("a"), (1.0, Some(3.0)));
    assert_eq!(span("b"), (3.75, Some(4.75)));
    assert_eq!(span("c"), (5.25, Some(9.25)), "video length 4 s");
    assert!(has(&ev.evaluate(2.0), "a") && !has(&ev.evaluate(3.2), "a") && has(&ev.evaluate(4.0), "b"));
}

#[test]
fn layer_source_time_speed_reverse_loop_freeze_and_remap() {
    let d = doc(
        "",
        r#"<layer id="speed" asset="vid" start="1" speed="2" clipIn="0.5"/>
        <layer id="rev" asset="vid" reverse="true"/>
        <layer id="loop" asset="vid" loop="2"/>
        <layer id="freeze" asset="vid" freezeAt="1"/>
        <layer id="remap" asset="vid"><timeRemap><key time="0" value="3"/><key time="2" value="1"/></timeRemap></layer>"#,
    );
    let f = eval(&d, 2.0);
    assert_eq!(node(&f, "speed").source_time, Some(2.5));
    assert_eq!(node(&f, "rev").source_time, Some(2.0), "4 s clip reversed: 4 − 2");
    assert_eq!(node(&eval(&d, 5.0), "loop").source_time, Some(1.0), "second play");
    assert!(!has(&eval(&d, 12.5), "loop"));
    assert_eq!(node(&f, "freeze").source_time, Some(1.0));
    assert_eq!(node(&eval(&d, 1.0), "remap").source_time, Some(2.0));
    assert!(!has(&eval(&d, 1.9), "speed") || node(&eval(&d, 1.9), "speed").source_time.is_some());
    assert!(!has(&eval(&d, 2.8), "speed"), "3.5 s of source at 2× ends at t = 2.75");
}

#[test]
fn group_time_offset_and_scale_shift_children() {
    let d = doc(
        "",
        r#"<group id="g" timeOffset="1" timeScale="2">
        <layer id="l" asset="img"><animate property="x"><key time="0" value="0"/><key time="10" value="100"/></animate></layer>
    </group>"#,
    );
    let f = eval(&d, 3.0);
    // child time = (3 − 1) × 2 = 4
    assert_eq!(node(&f, "l").props.get("x"), Some(&Value::Len(Length::px(40.0))));
    assert_eq!(node(&f, "l").timeline_time, 4.0);
}

#[test]
fn markers_place_nodes_and_keys() {
    let xml = format!(
        r#"<scene version="1.1"><project width="10" height="10" fps="10" duration="10"/>{ASSETS}<markers><marker id="in" time="2"/><marker id="out" time="5"/><beatGrid bpm="120" offset="1"/></markers><composition>
        <layer id="l" asset="img" startMarker="in" start="0.5" endMarker="out"><animate property="x"><key time="0" value="0" marker="in"/><key time="1" value="10" marker="in"/></animate>
        <expression property="rotation">beat() + markerTime("bar.1")</expression></layer></composition></scene>"#
    );
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    assert!(!has(&eval(&d, 2.2), "l") && has(&eval(&d, 2.6), "l") && !has(&eval(&d, 5.0), "l"));
    let l = node(&eval(&d, 2.6), "l").clone();
    assert!(close(l.props.get("x").unwrap().as_num().unwrap(), 6.0));
    // beat at t = 2.6 with 120 bpm from offset 1 = 3.2 beats; bar.1 = 1 + 4 × 0.5 = 3
    assert!(close(l.props.get("rotation").unwrap().as_num().unwrap(), 6.2));
}

// ------------------------------------------------------------------ templating

#[test]
fn parameters_from_defaults_variants_rows_and_command_line() {
    let params = r#"<parameters>
        <param id="name" type="string" default="World"/>
        <param id="size" type="number" default="10" min="1" max="100"/>
        <param id="show" type="boolean" default="true"/>
        <data id="rows" format="csv">name,size
Ada,20
Linus,30</data>
        <variant id="big"><set param="size" value="80"/><override target="l" property="rotation" value="45"/></variant>
        <bind param="size" target="l" property="scaleX" map="80=4;20=2"/>
    </parameters>"#;
    let d = doc(
        params,
        r#"<layer id="l" asset="img" condition="param('show')"><expression property="x">param("size")</expression></layer>
                            <layer id="t" asset="txt"/>"#,
    );
    let x = |o: &EvalOptions| node(&ev(&d, o).evaluate(0.0), "l").props.get("x").cloned();
    assert_eq!(x(&EvalOptions::default()), Some(Value::Len(Length::px(10.0))));
    let big = EvalOptions { variant: Some("big".into()), ..Default::default() };
    assert_eq!(x(&big), Some(Value::Len(Length::px(80.0))));
    let f = ev(&d, &big).evaluate(0.0);
    let l = node(&f, "l");
    let (p, o) = (l.world.apply([1.0, 0.0]), origin(l));
    assert!(
        close(p[0] - o[0], 4.0 * 45f64.to_radians().cos()),
        "variant override rotates, bind maps 80 → scaleX 4: {p:?}"
    );
    let row = EvalOptions { row: Some((None, 1)), ..Default::default() };
    assert_eq!(x(&row), Some(Value::Len(Length::px(30.0))));
    assert_eq!(node(&ev(&d, &row).evaluate(0.0), "t").text.as_deref(), Some("Hello Linus #{{index}}: {{item.city}}"));
    let cli = EvalOptions {
        params: vec![("size".into(), "55".into()), ("show".into(), "false".into())],
        ..Default::default()
    };
    assert!(!has(&ev(&d, &cli).evaluate(0.0), "l"), "condition hides the node");
    let bad =
        EvalOptions { params: vec![("size".into(), "500".into()), ("nmae".into(), "x".into())], ..Default::default() };
    let e = Evaluator::new(&d, &bad).unwrap_err();
    assert_eq!(codes(&e), ["E07"]);
    assert!(e.diagnostics.iter().any(|x| x.message.contains("did you mean \"name\"")));
    assert_eq!(err_codes(&d, &EvalOptions { variant: Some("small".into()), ..Default::default() }), ["E10"]);
}

#[test]
fn required_parameters_and_patterns() {
    let d = doc(r#"<parameters><param id="code" type="string" required="true" pattern="[A-Z]{3}"/></parameters>"#, "");
    assert_eq!(err_codes(&d, &EvalOptions::default()), ["E08"]);
    let o = EvalOptions { params: vec![("code".into(), "abc".into())], ..Default::default() };
    assert_eq!(err_codes(&d, &o), ["E07"]);
    let o = EvalOptions { params: vec![("code".into(), "ABC".into())], ..Default::default() };
    assert!(err_codes(&d, &o).is_empty());
}

#[test]
fn repeats_generate_scoped_copies() {
    let d = doc(
        r#"<parameters><param id="cities" type="list" default="[{&quot;city&quot;:&quot;Lisbon&quot;},{&quot;city&quot;:&quot;Oslo&quot;}]"/></parameters>"#,
        r#"<repeat id="r" count="3" offsetX="10" opacityStep="0.25" timeStep="1">
             <layer id="l" asset="img"><expression property="y">index * 100 + count</expression>
               <animate property="rotation"><key time="0" value="0"/><key time="2" value="20"/></animate></layer>
           </repeat>
           <repeat id="c" over="cities" var="item"><layer id="t" asset="txt"/></repeat>"#,
    );
    let f = eval(&d, 2.0);
    let l2 = node(&f, "r[2]/l");
    assert_eq!(l2.props.get("y"), Some(&Value::Len(Length::px(203.0))));
    assert!(close(origin(l2)[0], 20.0));
    assert!(close(l2.world_opacity, 0.5));
    assert_eq!(l2.props.get("rotation"), Some(&Value::Num(0.0)), "staggered by 2 s");
    assert_eq!(node(&f, "r[1]/l").props.get("rotation"), Some(&Value::Num(10.0)));
    assert_eq!(node(&f, "c[1]/t").text.as_deref(), Some("Hello {{name}} #1: Oslo"));
    assert_eq!(node(&f, "c[1]/t").repeat, Some([1, 2]));
}

#[test]
fn object3d_instances_evaluate_once_per_index() {
    let d = doc(
        "",
        r#"<object3D id="p" primitive="plane" instances="4" y="5"><expression property="x">170 + index * 100 + count</expression></object3D>"#,
    );
    let f = eval(&d, 0.0);
    let copies: Vec<&FrameNode> = f.nodes.iter().filter(|n| n.kind == "object3D").collect();
    assert_eq!(copies.len(), 4);
    for (k, id) in ["p", "p[1]", "p[2]", "p[3]"].into_iter().enumerate() {
        let n = node(&f, id);
        let x = n.props.get("x").and_then(Value::as_num);
        assert_eq!(x, Some(174.0 + 100.0 * k as f64), "{id}");
        assert_eq!(n.repeat, Some([k as u32, 4]));
    }
}

#[test]
fn bare_assignment_declares_a_variable() {
    let d = doc("", r#"<layer id="a" asset="img"><expression property="x">t0 = [40, 50][0]; a = 50; t0 + 2 * a</expression></layer>"#);
    assert_eq!(node(&eval(&d, 0.0), "a").props.get("x"), Some(&Value::Len(Length::px(140.0))));
}

#[test]
fn instances_scope_ids_apply_overrides_and_run_on_their_own_clock() {
    let d = doc_after(
        r#"<symbols><symbol id="card" width="200" height="100" duration="2">
             <layer id="bg" asset="img" x="50%"><animate property="opacity"><key time="0" value="0"/><key time="2" value="1"/></animate></layer>
           </symbol></symbols>"#,
        r#"<instance id="a" symbol="card" start="1"><override target="bg" property="rotation" value="30"/></instance>
           <instance id="b" symbol="card" speed="0.5" loop="1"/>"#,
        "",
    );
    let f = eval(&d, 2.0);
    let a = node(&f, "a/bg");
    assert_eq!(a.props.get("opacity"), Some(&Value::Num(0.5)));
    assert!(close(origin(a)[0], 100.0), "50% of the symbol box");
    assert_eq!(node(&f, "b/bg").props.get("opacity"), Some(&Value::Num(0.5)));
    assert!(!has(&eval(&d, 3.5), "a/bg"), "a ends with its 2 s symbol");
    assert!(has(&eval(&d, 7.0), "b/bg"), "b plays twice at half speed: 8 s");
    let rec = doc_after(
        r#"<symbols><symbol id="s"><instance id="i" symbol="s"/></symbol></symbols>"#,
        r#"<instance id="top" symbol="s"/>"#,
        "",
    );
    assert_eq!(err_codes(&rec, &EvalOptions::default()), ["E13"]);
    let miss = doc_after(
        r#"<symbols><symbol id="s"><layer id="x" asset="img"/></symbol></symbols>"#,
        r#"<instance id="top" symbol="s"><override target="y" property="x" value="1"/></instance>"#,
        "",
    );
    assert_eq!(err_codes(&miss, &EvalOptions::default()), ["E05"]);
}

#[test]
fn transitions_and_mattes() {
    let d = doc(
        "",
        r#"<sequence id="s" transition="crossfade" transitionDuration="1">
        <layer id="a" asset="img" end="3"/><layer id="b" asset="img" end="3"/><layer id="c" asset="img" end="3"/>
        <transition type="wipe" from="b" to="c" duration="2" alignment="end" curve="linear"/>
    </sequence>
    <layer id="m" asset="img"/><layer id="u" asset="img" matte="m"/>"#,
    );
    let f = eval(&d, 3.0);
    assert_eq!(f.transitions.len(), 1);
    assert_eq!(&*f.transitions[0].kind, "crossfade");
    assert!(close(f.transitions[0].progress, 0.5));
    assert!(has(&f, "a") && has(&f, "b"), "handles keep both sides visible");
    let w = eval(&d, 5.5);
    assert_eq!(&*w.transitions[0].kind, "wipe");
    assert!(close(w.transitions[0].progress, 0.75));
    let m = node(&w, "m");
    assert!(m.is_matte && !m.draw);
    assert_eq!(node(&w, "u").matte, Some(w.nodes.iter().position(|n| &*n.id == "m").unwrap() as u32));
}

#[test]
fn motion_paths_and_camera_switching() {
    let d = doc(
        "",
        r#"<layer id="l" asset="img"><motionPath path="M0 0 L100 0 L100 100" start="0" end="2" autoOrient="true"/></layer>
        <camera id="c1" end="5"/><camera id="c2" start="5"/>"#,
    );
    let f = eval(&d, 1.5);
    let l = node(&f, "l");
    let p = origin(l);
    assert!(close(p[0], 100.0) && close(p[1], 50.0), "{p:?}");
    let dir = l.world.apply([1.0, 0.0]);
    assert!(close(dir[1] - p[1], 1.0), "oriented along +y");
    assert_eq!(f.nodes[f.camera.unwrap() as usize].id.as_ref(), "c1");
    let g = eval(&d, 6.0);
    assert_eq!(g.nodes[g.camera.unwrap() as usize].id.as_ref(), "c2");
}

#[test]
fn global_elements_animate_on_the_composition_clock() {
    let xml = format!(
        r##"<scene version="1.1"><project width="10" height="10" fps="10" duration="10"/>{ASSETS}<paints><linearGradient id="g"><stop offset="0" color="#000000"><animate property="color"><key time="0" value="#000000"/><key time="1" value="#ffffff"/></animate></stop><stop offset="1" color="#ffffff"/></linearGradient></paints><composition/><effects><effect id="blur" type="blur"><animate property="radius"><key time="0" value="0"/><key time="2" value="20"/></animate></effect></effects></scene>"##
    );
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let f = eval(&d, 0.5);
    let stop = f.elements.iter().find(|e| e.element == "stop").expect("stop");
    assert_eq!(stop.props.get("color"), Some(&Value::Color([0.5, 0.5, 0.5, 1.0])));
    let blur = f.elements.iter().find(|e| &*e.key == "blur").expect("effect");
    assert_eq!(blur.props.get("radius"), Some(&Value::Num(5.0)));
}

#[test]
fn frames_are_pure_functions_of_time() {
    let d = sr_model::load_file(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus/valid/kitchen-sink.scene.xml"),
        &Default::default(),
    )
    .unwrap();
    let e = ev(&d, &EvalOptions::default());
    let forward: Vec<String> = (0..30).map(|i| serde_json::to_string(&e.evaluate_frame(i)).unwrap()).collect();
    let backward: Vec<String> = (0..30).rev().map(|i| serde_json::to_string(&e.evaluate_frame(i)).unwrap()).collect();
    for (i, f) in forward.iter().enumerate() {
        assert_eq!(f, &backward[29 - i], "frame {i} depends on evaluation order");
    }
    let again = ev(&d, &EvalOptions::default());
    assert_eq!(serde_json::to_string(&again.evaluate_frame(17)).unwrap(), forward[17]);
    let _ = LengthUnit::Px;
}

#[test]
fn includes_pull_symbols_from_other_documents_under_a_namespace() {
    let dir = std::env::temp_dir().join(format!("sr-eval-include-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("lib.scene.xml"),
        r#"<scene version="1.1"><project width="400" height="200" fps="10" duration="3"/>
        <assets><image id="logo" src="logo.png" width="10" height="10"/></assets>
        <symbols><symbol id="badge" width="100" height="100"><layer id="mark" asset="logo" x="50%"/></symbol></symbols>
        <composition/></scene>"#,
    )
    .unwrap();
    let main = String::from(
        r#"<scene version="1.1"><project width="1000" height="500" fps="10" duration="10"/><composition>
        <include id="inc" src="lib.scene.xml" symbol="badge" start="1"><override target="mark" property="rotation" value="90"/></include>
        <include id="bad" src="lib.scene.xml" symbol="nope"/></composition></scene>"#,
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: Some(dir.clone()) };
    let d = sr_model::load_str(&main, &opts).unwrap();
    assert_eq!(err_codes(&d, &EvalOptions::default()), ["E12"]);
    let good = sr_model::load_str(&main.replace(r#"<include id="bad" src="lib.scene.xml" symbol="nope"/>"#, ""), &opts)
        .unwrap();
    let f = eval(&good, 2.0);
    let m = node(&f, "inc/mark");
    assert_eq!(m.asset.as_deref(), Some("inc/logo"));
    assert!(close(origin(m)[0], 50.0), "50% of the 100 px symbol");
    assert!(close(m.world.apply([1.0, 0.0])[1] - origin(m)[1], 1.0), "override rotated the mark");
    assert!(!has(&eval(&good, 4.5), "inc/mark"), "the included symbol lasts the library's 3 s");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn remaining_error_codes() {
    let bad_override = doc(
        r#"<parameters><variant id="v"><override target="l" property="opacity" value="2"/></variant></parameters>"#,
        r#"<layer id="l" asset="img"/>"#,
    );
    assert_eq!(err_codes(&bad_override, &EvalOptions { variant: Some("v".into()), ..Default::default() }), ["E06"]);
    let bad_data = doc(r#"<parameters><data id="d" format="json">[1, 2</data></parameters>"#, "");
    assert_eq!(err_codes(&bad_data, &EvalOptions::default()), ["E09"]);
    let row = doc(r#"<parameters><data id="d" format="json">[{"a":1}]</data></parameters>"#, "");
    assert_eq!(err_codes(&row, &EvalOptions { row: Some((None, 5)), ..Default::default() }), ["E09"]);
    let bad_link = doc("", r#"<layer id="l" asset="img"><link property="x" source="audio:music:bass"/></layer>"#);
    assert_eq!(err_codes(&bad_link, &EvalOptions::default()), ["E14"]);
    let bad_path = doc("", r#"<layer id="l" asset="img"><motionPath path="M0 0 Q 1"/></layer>"#);
    assert_eq!(err_codes(&bad_path, &EvalOptions::default()), ["E15"]);
    let placeholder = doc("", r#"<layer id="l" asset="txt"/>"#);
    let e = ev(&placeholder, &EvalOptions::default());
    assert!(e.warnings().iter().all(|w| w.code == "E16" && !w.is_error()) && !e.warnings().is_empty());
}

#[test]
fn audio_analysis_drives_expressions_and_links() {
    let d = doc(
        "",
        r#"<layer id="l" asset="img"><expression property="opacity">audioAmplitude("music", "low")</expression>
        <link property="rotation" source="audio:music" scale="90"/></layer>"#,
    );
    let mut a = sr_eval::Analysis { fps: 10.0, ..Default::default() };
    a.tracks.insert("music".into(), [vec![0.0, 1.0, 0.5], vec![0.2, 0.4, 0.6], vec![], vec![]]);
    let e = ev(&d, &EvalOptions { analysis: a, ..Default::default() });
    let f = e.evaluate(0.15);
    let l = node(&f, "l");
    assert!(
        close(l.props.get("opacity").unwrap().as_num().unwrap(), 0.5),
        "low band interpolated between frames 1 and 2"
    );
    assert!(close(l.props.get("rotation").unwrap().as_num().unwrap(), 67.5));
}

#[test]
fn flex_layout_alignment_and_fit_boxes() {
    let xml = format!(
        r#"<scene version="1.1"><project width="1000" height="500" fps="10" duration="10" safeArea="sa"/><safeAreas><safeArea id="sa" top="0.1" right="0.1" bottom="0.1" left="0.1"/></safeAreas>{ASSETS}<composition>{}</composition></scene>"#,
        r#"<group id="row" x="100" y="100" width="400" height="100" layout="row" gap="10" padding="5" justify="center" alignItems="center">
             <layer id="a" asset="img"/><layer id="b" asset="img" fit="cover" boxWidth="50" boxHeight="50"/>
           </group>
           <layer id="r" asset="img" alignX="right" alignY="bottom" alignTo="safe-area"/>
           <layer id="c" asset="img" alignX="center" alignY="middle" alignTo="frame"/>"#
    );
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let f = eval(&d, 0.0);
    // row content: 100 + 10 + 50 = 160 wide, centred in 390 → starts at 5 + 115
    let a = node(&f, "a");
    assert_eq!(origin(a), [100.0 + 120.0, 100.0 + 5.0 + 20.0]);
    let b = node(&f, "b");
    assert_eq!(origin(b), [100.0 + 230.0, 100.0 + 5.0 + 20.0]);
    assert_eq!(b.size, Some([50.0, 50.0]));
    assert_eq!(b.content.unwrap().uv, [0.25, 0.0, 0.75, 1.0], "cover crops the 2:1 image to its centre");
    assert_eq!(origin(node(&f, "r")), [900.0 - 100.0, 450.0 - 50.0]);
    assert_eq!(origin(node(&f, "c")), [450.0, 225.0]);
}
