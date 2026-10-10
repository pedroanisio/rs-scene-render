//! SREP 66: build-mode programs run when a document is loaded, and their output takes their place. The modules are
//! the sr-core kit's (`crates/sr-wasm/tests/modules`).

use std::path::PathBuf;

use sr_model::model::Node;
use sr_model::{load_str, validate_str, LoadOptions};

fn modules() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sr-wasm/tests/modules")
}

fn sha(name: &str) -> String {
    use sha2::Digest;
    let b = std::fs::read(modules().join(format!("{name}.wasm"))).unwrap();
    sha2::Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn opts() -> LoadOptions {
    // every test of this file runs without the on-disk cache (tests/program_cache.rs tests it)
    std::env::set_var("SR_PROGRAM_CACHE", "off");
    LoadOptions { verify_assets: true, base_dir: Some(modules()) }
}

fn program(name: &str, attrs: &str, inner: &str) -> String {
    format!(r#"<program id="p" src="{name}.wasm" sha256="{}"{attrs}>{inner}</program>"#, sha(name))
}

fn doc(body: &str) -> String {
    format!(
        r#"<scene version="1.6"><project width="640" height="360" fps="24" duration="1" seed="1"/><composition>{body}</composition></scene>"#
    )
}

fn codes(xml: &str) -> Vec<String> {
    match load_str(xml, &opts()) {
        Ok(_) => Vec::new(),
        Err(e) => e.report().map(|r| r.diagnostics.iter().map(|d| d.code.clone()).collect()).unwrap_or_default(),
    }
}

#[test]
fn a_build_program_becomes_a_group_holding_its_output() {
    let d = load_str(&doc(&program("srep66-rect", r#" x="100" y="40""#, "")), &opts()).expect("loads");
    let Some(Node::Group(g)) = d.node("p") else { panic!("p is a group: {:?}", d.node("p")) };
    assert_eq!(g.x.to_string(), "100");
    assert!(matches!(d.node("g1"), Some(Node::Shape(_))), "the emitted shape is in the document");
}

#[test]
fn the_seed_reaches_the_module() {
    // project seed 1: seed 4 gives the red fragment (x = 480), seed 1 the blue one (x = 160)
    for (seed, x) in [(4, "480"), (1, "160")] {
        let d = load_str(&doc(&program("srep66-seed", &format!(r#" seed="{seed}""#), "")), &opts()).unwrap();
        let Some(Node::Shape(s)) = d.node("g1") else { panic!() };
        assert_eq!(s.x.to_string(), x, "seed {seed}");
    }
}

#[test]
fn parameters_reach_the_module() {
    let d = load_str(&doc(&program("srep66-param", "", r#"<param name="side" value="1"/>"#)), &opts()).unwrap();
    let Some(Node::Shape(s)) = d.node("g1") else { panic!() };
    assert_eq!(s.x.to_string(), "480");
}

#[test]
fn a_data_program_feeds_a_repeat() {
    let xml = format!(
        r#"<scene version="1.6"><project width="640" height="360" fps="24" duration="1"/><parameters>{}</parameters><composition><repeat id="r" over="rows"><shape id="c" shape="rect" width="4" height="4"/></repeat></composition></scene>"#,
        program("srep66-rows", "", "").replace(r#"id="p""#, r#"id="rows""#)
    );
    let d = load_str(&xml, &opts()).expect("loads");
    let rows = d.data_source("rows").expect("the program became a data source");
    assert_eq!(rows.value.trim(), r#"[{"a":1},{"a":2}]"#);
}

#[test]
fn failures_carry_their_codes() {
    assert_eq!(codes(&doc(&program("srep66-loop", r#" fuel="100000""#, ""))), ["PRG12"]);
    assert_eq!(codes(&doc(&program("srep66-wasi", "", ""))), ["PRG11"]);
    // an invalid fragment can break several rules; each is reported as PRG14 at the program
    let bad = codes(&doc(&program("srep66-bad-output", "", "")));
    assert!(!bad.is_empty() && bad.iter().all(|c| c == "PRG14"), "{bad:?}");
    let collision = doc(&(program("srep66-rect", "", "") + r#"<shape id="g1" shape="rect" width="1" height="1"/>"#));
    assert!(codes(&collision).iter().all(|c| c == "PRG14") && !codes(&collision).is_empty(), "{:?}", codes(&collision));
    let pinned = program("srep66-rect", &format!(r#" outputSha256="{}""#, "1".repeat(64)), "");
    assert_eq!(codes(&doc(&pinned)), ["PRG15"]);
    let wrong_hash = doc(&program("srep66-rect", "", "")).replace(&sha("srep66-rect"), &"0".repeat(64));
    assert_eq!(codes(&wrong_hash), ["PRG10"]);
}

#[test]
fn a_matching_output_digest_loads() {
    use sha2::Digest;
    let out = sr_wasm::generate(
        &std::fs::read(modules().join("srep66-rect.wasm")).unwrap(),
        &sr_wasm::Inputs::default(),
        sr_wasm::Limits::default(),
    )
    .unwrap();
    let digest: String = sha2::Sha256::digest(&out).iter().map(|x| format!("{x:02x}")).collect();
    assert!(load_str(&doc(&program("srep66-rect", &format!(r#" outputSha256="{digest}""#), "")), &opts()).is_ok());
}

#[test]
fn validation_alone_does_not_run_programs() {
    let r = validate_str(&doc(&program("srep66-loop", r#" fuel="100000""#, "")), &opts());
    assert!(!r.has_errors(), "{r}");
}

#[test]
fn the_maze_example_loads_with_its_pinned_output() {
    // examples/program-maze: outputSha256 was computed from the module's output under V8 (Node 24); the same
    // digest here shows a second runtime and host computing the same maze
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/program-maze/maze.scene.xml");
    std::env::set_var("SR_PROGRAM_CACHE", "off");
    let d = sr_model::load_file(&path, &LoadOptions::default()).expect("the example loads");
    assert!(matches!(d.node("maze"), Some(Node::Group(_))));
    assert!(
        matches!(d.node("maze-walls"), Some(Node::Shape(_))) && matches!(d.node("maze-route"), Some(Node::Shape(_)))
    );
}
