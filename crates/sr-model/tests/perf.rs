//! Load performance: a 10,000-node document validates and loads in under
//! 200 ms (release build). Debug builds report the time without asserting.

use std::fmt::Write;
use std::time::Instant;

use sr_model::{load_str, LoadOptions};

/// 100 groups × 99 animated layers + 100 group nodes = 10,000 composition nodes.
fn big_document() -> String {
    let mut s = String::from(
        "<scene version=\"1.1\">\n<project width=\"1920\" height=\"1080\" fps=\"60\" duration=\"60\"/>\n\
         <styles><token name=\"brand\" value=\"#ff8800\"/></styles>\n\
         <assets><image id=\"img\" src=\"img.png\" width=\"64\" height=\"64\"/></assets>\n\
         <paints><linearGradient id=\"g\"><stop offset=\"0\" color=\"#000000\"/><stop offset=\"1\" color=\"var(--brand)\"/></linearGradient></paints>\n\
         <markers><marker id=\"m\" time=\"1\"/></markers>\n<composition>\n",
    );
    for g in 0..100 {
        writeln!(s, "<group id=\"g{g}\" x=\"{g}%\" startMarker=\"m\">").unwrap();
        for l in 0..99 {
            writeln!(
                s,
                "  <layer id=\"l{g}_{l}\" asset=\"img\" x=\"{l}\" y=\"10vh\" opacity=\"0.5\" blend=\"screen\">\
                 <animate property=\"x\"><key time=\"0\" value=\"0\"/><key time=\"1\" value=\"100\" interpolation=\"cubic-bezier\" bezier=\"0.4,0,0.2,1\"/></animate></layer>"
            )
            .unwrap();
        }
        s.push_str("</group>\n");
    }
    s.push_str("</composition>\n</scene>\n");
    s
}

#[test]
fn ten_thousand_nodes_load_under_200_ms() {
    let xml = big_document();
    sr_model::xsd::simple::warm_up();
    let opts = LoadOptions::without_assets();
    let mut best = f64::MAX;
    for _ in 0..3 {
        let t = Instant::now();
        let doc = load_str(&xml, &opts).expect("valid");
        let ms = t.elapsed().as_secs_f64() * 1e3;
        best = best.min(ms);
        assert_eq!(doc.composition_nodes().len(), 10_000);
        assert!(doc.node("l99_98").is_some());
    }
    eprintln!("10,000-node document ({} KiB): validate + load in {best:.1} ms", xml.len() / 1024);
    if !cfg!(debug_assertions) {
        assert!(best < 200.0, "took {best:.1} ms");
    }
}
