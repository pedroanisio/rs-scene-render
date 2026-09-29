//! Cost of evaluating 5,000 animated properties with 500 expressions: the median frame
//! takes under 2 ms on one core (release build).

use std::fmt::Write;
use std::time::Instant;

use sr_eval::{EvalOptions, Evaluator};

const CURVES: [&str; 8] =
    ["linear", "ease-in-out", "cubic-in-out", "back-out", "elastic-out", "bounce-out", "catmull-rom", "spring"];

/// 1,000 layers × 5 keyframed properties = 5,000 animated properties, plus
/// 500 expressions (wiggle, prop() references, arithmetic).
pub fn benchmark_document() -> String {
    let mut s = String::from(
        r#"<scene version="1.1"><project width="1920" height="1080" fps="60" duration="10" seed="3"/>
<assets><image id="img" src="img.png" width="64" height="64"/></assets><composition>"#,
    );
    for g in 0..20 {
        write!(s, r#"<group id="g{g}" x="{}" y="{}">"#, g * 90, g * 50).unwrap();
        for l in 0..50 {
            let i = g * 50 + l;
            let c = |k: usize| CURVES[(i + k) % CURVES.len()];
            write!(s, r#"<layer id="l{i}" asset="img">"#).unwrap();
            for (k, (prop, a, b)) in [
                ("x", "0", "900"),
                ("y", "0", "500"),
                ("rotation", "0", "360"),
                ("opacity", "0", "1"),
                ("scaleX", "0.5", "2"),
            ]
            .iter()
            .enumerate()
            {
                write!(
                    s,
                    r#"<animate property="{prop}" defaultInterpolation="{}" extrapolateAfter="ping-pong"><key time="0" value="{a}"/><key time="{}" value="{b}"/><key time="{}" value="{a}"/><key time="6" value="{b}"/></animate>"#,
                    c(k),
                    1.0 + (i % 7) as f64 * 0.25,
                    3.0 + (i % 5) as f64 * 0.3
                )
                .unwrap();
            }
            if i % 2 == 0 {
                let e = match i % 3 {
                    0 => "value + wiggle(2, 5)".to_string(),
                    1 => format!("prop(\"l{}.x\") * 0.5 + time * 10", i + 1),
                    _ => "let a = Math.sin(time * 3 + index); clamp(value + a * 20, -90, 90)".to_string(),
                };
                write!(s, r#"<expression property="skewX">{e}</expression>"#).unwrap();
            }
            s.push_str("</layer>");
        }
        s.push_str("</group>");
    }
    s.push_str("</composition></scene>");
    s
}

#[test]
fn five_thousand_properties_and_500_expressions_under_2_ms() {
    let xml = benchmark_document();
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).expect("valid");
    let t = Instant::now();
    let ev = Evaluator::new(&doc, &EvalOptions::default()).unwrap_or_else(|r| panic!("{r}"));
    let compile_ms = t.elapsed().as_secs_f64() * 1e3;
    let (nodes, slots, channels, exprs) = ev.stats();
    assert_eq!(channels, 5000);
    assert_eq!(exprs, 500);
    assert!(slots >= 5500);
    // warm up, then take the median of 120 frames
    for i in 0..30 {
        std::hint::black_box(ev.evaluate_frame(i));
    }
    let mut times: Vec<f64> = (0..120)
        .map(|i| {
            let t = Instant::now();
            let f = ev.evaluate_frame(i * 5);
            std::hint::black_box(&f);
            t.elapsed().as_secs_f64() * 1e3
        })
        .collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (median, p95) = (times[times.len() / 2], times[times.len() * 95 / 100]);
    eprintln!("{nodes} nodes, {slots} slots, {channels} channels, {exprs} expressions: compile {compile_ms:.1} ms; frame median {median:.3} ms, p95 {p95:.3} ms");
    if !cfg!(debug_assertions) {
        assert!(median < 2.0, "median frame {median:.3} ms");
    }
}
