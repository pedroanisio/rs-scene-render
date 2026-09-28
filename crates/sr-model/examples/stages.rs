//! Times each loading stage on a document: `cargo run --release --example stages -- FILE`.
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("usage: stages FILE");
    let xml = std::fs::read_to_string(&path).expect("read");
    sr_model::xsd::simple::warm_up();
    let t = Instant::now();
    let doc = roxmltree::Document::parse(&xml).expect("well-formed");
    let parse = t.elapsed();
    let mut out = Vec::new();
    let t = Instant::now();
    sr_model::xsd::structure::validate(&doc, &mut out);
    let structure = t.elapsed();
    let t = Instant::now();
    sr_model::rules::validate(&doc, &mut out);
    let rules = t.elapsed();
    let t = Instant::now();
    let loaded = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets());
    let total = t.elapsed();
    println!("parse {parse:?}  structure {structure:?}  rules {rules:?}  full load {total:?}  ok={}", loaded.is_ok());
}
