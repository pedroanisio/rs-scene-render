//! The style engine against MapLibre's style-spec: the Protomaps "light" style
//! evaluated on real features of `baixa.pmtiles` (`tools/fixtures/make_style_expected.mjs`).

use std::path::PathBuf;

use serde_json::Value;
use sr_geo::style::{Ctx, Style, V};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0)
}

fn same(got: &V, want: &Value) -> bool {
    match (got, want) {
        (_, Value::Object(o)) if o.contains_key("color") => {
            let w: Vec<f64> = o["color"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
            got.color().is_some_and(|c| (0..4).all(|k| close(c[k], w[k])))
        }
        (_, Value::Object(o)) if o.contains_key("text") => got.text() == o["text"].as_str().unwrap(),
        (V::Num(a), Value::Number(b)) => close(*a, b.as_f64().unwrap()),
        (V::Str(a), Value::String(b)) => a == b,
        (V::Bool(a), Value::Bool(b)) => a == b,
        (V::Arr(a), Value::Array(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y)),
        (V::Null, Value::Null) => true,
        _ => false,
    }
}

#[test]
fn protomaps_light_evaluates_like_maplibre() {
    let style = Style::parse(
        &std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("styles/protomaps-light.json"))
            .unwrap(),
    )
    .unwrap();
    let e: Value = serde_json::from_str(&std::fs::read_to_string(fixture("style.json")).unwrap()).unwrap();
    let samples = e["samples"].as_array().unwrap();
    let (mut checked, mut failures) = (0usize, Vec::new());
    for r in e["results"].as_array().unwrap() {
        let s = &samples[r["sample"].as_u64().unwrap() as usize];
        let zoom = r["zoom"].as_f64().unwrap();
        let props = s["properties"].as_object().unwrap();
        let cx = Ctx { zoom, properties: props, geometry: s["type"].as_str().unwrap(), id: s["id"].as_u64() };
        let layer = s["layer"].as_str().unwrap();
        let got: Vec<&str> = style
            .layers
            .iter()
            .filter(|l| l.kind != "background" && l.source_layer.as_deref() == Some(layer) && l.accepts(&cx))
            .map(|l| l.id.as_str())
            .collect();
        let want: Vec<&str> = r["hits"].as_array().unwrap().iter().map(|h| h["layer"].as_str().unwrap()).collect();
        if got != want {
            failures.push(format!("{layer} {} z{zoom}: layers {got:?} vs {want:?}", s["index"]));
            continue;
        }
        for h in r["hits"].as_array().unwrap() {
            let l = style.layers.iter().find(|l| l.id == h["layer"]).unwrap();
            for (p, w) in h["values"].as_object().unwrap() {
                if w.get("error").is_some() {
                    continue;
                }
                let v = if l.paint.contains_key(p) { l.paint(p, &cx) } else { l.layout(p, &cx) };
                checked += 1;
                if !same(&v, w) {
                    failures.push(format!("{} {p} z{zoom} ({layer} {}): {v:?} vs {w}", l.id, s["index"]));
                }
            }
        }
    }
    assert!(checked > 2000, "only {checked} values compared");
    assert!(
        failures.is_empty(),
        "{} differ of {checked}:\n{}",
        failures.len(),
        failures[..failures.len().min(30)].join("\n")
    );
    // the background colour at three zooms
    let bg = style.layers.iter().find(|l| l.kind == "background").unwrap();
    let empty = serde_json::Map::new();
    for b in e["background"].as_array().unwrap() {
        let cx = Ctx { zoom: b["zoom"].as_f64().unwrap(), properties: &empty, geometry: "Polygon", id: None };
        let w = serde_json::json!({"color": b["color"]});
        assert!(same(&bg.paint("background-color", &cx), &w));
    }
}
