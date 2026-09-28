use std::path::PathBuf;

use sr_eval::{EvalOptions, Evaluator, Value};

fn corpus(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus").join(p)
}

#[test]
fn kitchen_sink_evaluates() {
    let doc = sr_model::load_file(corpus("valid/kitchen-sink.scene.xml"), &Default::default()).unwrap();
    let ev = match Evaluator::new(&doc, &EvalOptions::default()) {
        Ok(e) => e,
        Err(r) => panic!("{r}"),
    };
    let f = ev.evaluate(1.0);
    for n in &f.nodes {
        eprintln!(
            "{:>3} {:<28} {:<16} depth {} draw {} t {:.3} world {:?} op {:.3} props {:?}",
            n.depth, n.id, n.kind, n.depth, n.draw, n.local_time, n.world.0, n.world_opacity, n.props.0
        );
    }
    eprintln!("{}", serde_json::to_string(&f.transitions).unwrap());
    eprintln!("{}", serde_json::to_string(&f.elements).unwrap());
    assert!(f.nodes.len() > 10);
    let _ = Value::Num(0.0);
}
