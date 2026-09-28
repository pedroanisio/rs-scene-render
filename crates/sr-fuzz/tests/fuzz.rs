//! A short deterministic fuzzing run on every test pass (the long run is `sr-fuzz --seconds N`).

#[test]
fn no_panics_in_a_short_campaign() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus"));
    let seeds = sr_fuzz::corpus(root);
    assert!(seeds.len() > 100, "corpus found: {}", seeds.len());
    std::panic::set_hook(Box::new(|_| {}));
    let c = sr_fuzz::fuzz(42, &seeds, |c| c.docs + c.exprs >= 3000);
    let _ = std::panic::take_hook();
    let report: Vec<String> = c
        .crashes
        .iter()
        .take(5)
        .map(|k| format!("{}: {}\n{}", k.target, k.message, &k.input[..k.input.len().min(400)]))
        .collect();
    assert!(c.crashes.is_empty(), "{} crashes:\n{}", c.crashes.len(), report.join("\n---\n"));
}
