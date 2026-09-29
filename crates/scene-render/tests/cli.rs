//! End-to-end tests of the `scene-render` binary: exit codes and output formats.

use std::path::PathBuf;
use std::process::{Command, Output};

fn corpus(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus").join(p)
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scene-render")).args(args).env("NO_COLOR", "1").output().expect("binary runs")
}

fn path(p: &str) -> String {
    corpus(p).display().to_string()
}

#[test]
fn valid_document_exits_zero() {
    let o = run(&["validate", &path("valid/kitchen-sink.scene.xml")]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stdout));
    assert!(String::from_utf8_lossy(&o.stdout).contains("valid"));
}

#[test]
fn invalid_document_exits_one_with_a_located_diagnostic() {
    let o = run(&["validate", &path("invalid/c21.scene.xml")]);
    assert_eq!(o.status.code(), Some(1));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("error[C21]: transition type=\"shader\" requires @shader."), "{out}");
    assert!(out.contains("c21.scene.xml:"), "{out}");
    assert!(out.contains('^'), "{out}");
}

#[test]
fn json_output_lists_codes_per_file() {
    let o = run(&["validate", "--format", "json", &path("invalid/r6.scene.xml"), &path("valid/minimal.scene.xml")]);
    assert_eq!(o.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("JSON");
    assert_eq!(v["valid"], false);
    assert_eq!(v["files"][0]["diagnostics"][0]["code"], "R6");
    assert_eq!(v["files"][1]["valid"], true);
}

#[test]
fn warnings_fail_only_when_denied() {
    let f = path("valid/a03-remote.scene.xml");
    assert_eq!(run(&["validate", &f]).status.code(), Some(0));
    assert_eq!(run(&["validate", "--deny-warnings", &f]).status.code(), Some(1));
}

#[test]
fn asset_checks_can_be_skipped() {
    let f = path("invalid/a02-hash-mismatch.scene.xml");
    assert_eq!(run(&["validate", &f]).status.code(), Some(1));
    assert_eq!(run(&["validate", "--no-assets", &f]).status.code(), Some(0));
}

#[test]
fn unreadable_file_exits_two() {
    let o = run(&["validate", "/nonexistent/scene.xml"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("cannot read"));
}

#[test]
fn inspect_summarises_the_model() {
    let o = run(&["inspect", &path("valid/kitchen-sink.scene.xml")]);
    assert_eq!(o.status.code(), Some(0));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("1920x1080 @ 30000/1001 fps"), "{out}");
    assert!(out.contains("375 frames"), "{out}");
    let o = run(&["inspect", "--json", &path("valid/minimal.scene.xml")]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("JSON");
    assert_eq!(v["project"]["fps"], "30");
    assert_eq!(v["project"]["workingColorSpace"], "srgb");
}

#[test]
fn explain_describes_codes() {
    let o = run(&["explain", "c21"]);
    assert_eq!(o.status.code(), Some(0));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("not(@type='shader') or @shader"), "{out}");
    assert_eq!(run(&["explain", "Z99"]).status.code(), Some(2));
    let all = String::from_utf8_lossy(&run(&["explain"]).stdout).lines().count();
    assert!(all > 100, "{all}");
}

#[test]
fn completions_are_generated() {
    let o = run(&["completions", "bash"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&o.stdout).contains("scene-render"));
}

#[test]
fn eval_prints_frames_as_summary_and_json() {
    let f = path("valid/kitchen-sink.scene.xml");
    let o = run(&["eval", &f, "--time", "5"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("t = 5.0000 s") && out.contains("card-1/card-logo"), "{out}");
    let o = run(&["eval", &f, "--frame", "150", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("JSON");
    assert_eq!(v["frame"], 150);
    assert!(v["nodes"].as_array().unwrap().iter().any(|n| n["id"] == "hero"));
}

#[test]
fn eval_reports_template_errors_and_benchmarks() {
    let f = path("valid/kitchen-sink.scene.xml");
    let o = run(&["eval", &f, "--variant", "light"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stdout).contains("error[E10]"));
    let o = run(&["eval", &f, "--param", "headline=Bonjour", "--time", "1"]);
    assert!(String::from_utf8_lossy(&o.stdout).contains("\"Bonjour\""));
    let o = run(&["eval", &f, "--bench"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&o.stdout).contains("375 frames: median"));
    assert!(String::from_utf8_lossy(&run(&["explain", "E03"]).stdout).contains("cycle"));
}

/// A scene next to a copy of the corpus logo, in a fresh directory.
fn render_fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sr-cli-render-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(corpus("media/logo.png"), dir.join("logo.png")).unwrap();
    let xml = r##"<scene version="1.1"><project width="48" height="32" fps="10" duration="1" background="#102030"/>
<assets><image id="logo" src="logo.png" width="16" height="16"/></assets>
<composition><layer id="l" asset="logo" x="4" y="8"><animate property="x"><key time="0" value="4"/><key time="1" value="28"/></animate></layer>
<shape id="s" shape="rect" width="4" height="4"/></composition></scene>"##;
    std::fs::write(dir.join("r.scene.xml"), xml).unwrap();
    dir
}

fn no_gpu(o: &Output) -> bool {
    let e = String::from_utf8_lossy(&o.stderr);
    if o.status.code() == Some(2) && e.contains("no GPU adapter") {
        eprintln!("skipping: {e}");
        return true;
    }
    false
}

#[test]
fn render_writes_png_frames() {
    let dir = render_fixture("png");
    let scene = dir.join("r.scene.xml").display().to_string();
    let out = dir.join("one.png").display().to_string();
    let o = run(&["render", &scene, "--frame", "5", "-o", &out]);
    if no_gpu(&o) {
        return;
    }
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let text = String::from_utf8_lossy(&o.stdout);
    // the shape renders since Batch 5: nothing is reported as not rendered yet
    assert!(text.contains("wrote") && !text.contains("not rendered yet"), "{text}");
    let img = image::open(dir.join("one.png")).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (48, 32));
    // the 4×4 shape (default white fill) over the background
    assert_eq!(img.get_pixel(1, 1).0, [255, 255, 255, 255]);
    assert_eq!(img.get_pixel(10, 10).0, [0x10, 0x20, 0x30, 255]);
    // a frame range needs a pattern; with one it writes every frame
    let o = run(&["render", &scene, "--frames", "0..3", "-o", &out]);
    assert_eq!(o.status.code(), Some(2));
    let pattern = dir.join("f_%03d.png").display().to_string();
    let o = run(&["render", &scene, "--frames", "0..=2", "-o", &pattern, "--bit-depth", "16"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    for k in 0..3 {
        let img = image::open(dir.join(format!("f_{k:03}.png"))).unwrap();
        assert_eq!(img.color(), image::ColorType::Rgba16);
    }
}

#[test]
fn render_bench_reports_timing() {
    let dir = render_fixture("bench");
    let scene = dir.join("r.scene.xml").display().to_string();
    let o = run(&["render", &scene, "--bench", "--stats"]);
    if no_gpu(&o) {
        return;
    }
    assert_eq!(o.status.code(), Some(0));
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.contains("10 frames on") && text.contains("fps"), "{text}");
    assert!(String::from_utf8_lossy(&o.stderr).lines().any(|l| l.contains("\"draws\"")));
}

#[test]
fn encode_writes_adhoc_outputs() {
    let dir = render_fixture("encode");
    let scene = dir.join("r.scene.xml").display().to_string();
    let o = run(&["encode", &scene]);
    if no_gpu(&o) {
        return;
    }
    assert_eq!(o.status.code(), Some(2), "no <output> in the document");
    assert!(String::from_utf8_lossy(&o.stderr).contains("defines no matching <output>"));
    let mp4 = dir.join("clip.mp4").display().to_string();
    let o = run(&["encode", &scene, "-o", &mp4, "--hw", "software", "--json"]);
    if no_gpu(&o) {
        return;
    }
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let r: serde_json::Value = serde_json::from_slice(&o.stdout).expect("JSON report");
    assert_eq!(r["frames"], 10);
    assert_eq!(r["size"], serde_json::json!([48, 32]));
    assert!(r["unsupported"].as_array().is_none_or(|u| u.is_empty()), "{}", r["unsupported"]);
    assert!(std::fs::metadata(dir.join("clip.mp4")).unwrap().len() > 100);
    let seq = dir.join("f_%03d.png").display().to_string();
    let o = run(&["encode", &scene, "-o", &seq, "--end", "0.3"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.contains("(3 file(s), png)"), "{text}");
    assert!(dir.join("f_002.png").is_file());
}

#[test]
fn simulate_writes_a_verified_physics_cache() {
    let dir = std::env::temp_dir().join(format!("sr-cli-sim-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scene = |sha: &str| {
        format!(
            r##"<scene version="1.1"><project width="64" height="64" fps="30" duration="1"/><composition><shape id="box" shape="rect" width="10" height="10" fill="#FF0000"><rigidBody/></shape></composition><physics bounds="floor" cache="box.physics"{sha}/></scene>"##
        )
    };
    let path = dir.join("sim.scene.xml");
    std::fs::write(&path, scene("")).unwrap();
    let out = run(&["simulate", path.to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let sha =
        text.split("cacheSha256=\"").nth(1).and_then(|s| s.split('"').next()).expect("digest printed").to_string();
    assert_eq!(sha.len(), 64);
    assert!(dir.join("box.physics").is_file());
    // the recorded digest validates
    std::fs::write(&path, scene(&format!(r#" cacheSha256="{sha}""#))).unwrap();
    let v = run(&["validate", path.to_str().unwrap()]);
    assert!(v.status.success(), "{}", String::from_utf8_lossy(&v.stdout));
    // without a cache attribute the output must be named
    std::fs::write(&path, scene("").replace(r#" cache="box.physics""#, "")).unwrap();
    assert_eq!(run(&["simulate", path.to_str().unwrap()]).status.code(), Some(2));
}

#[test]
fn strict_render_fails_on_shader_fallback() {
    if sr_gpu::Gpu::new().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("sr-strict-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("broken.glsl"), "vec4 effect(vec2 uv) { return this is not glsl; }").unwrap();
    let scene = dir.join("strict.scene.xml");
    std::fs::write(
        &scene,
        r##"<scene version="1.1"><project width="32" height="32" fps="1" duration="1" background="#202020"/><composition><shape id="s" shape="rect" width="32" height="32" fill="#FF0000" effects="fx"/></composition><effects><effect id="fx" type="shader" src="broken.glsl"/></effects></scene>"##,
    )
    .unwrap();
    let run = |extra: &[&str]| {
        let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_scene-render"));
        c.arg("render").arg(&scene).arg("-o").arg(dir.join("f.png")).args(extra);
        c.output().unwrap()
    };
    let lax = run(&[]);
    let out = String::from_utf8_lossy(&lax.stdout);
    assert!(lax.status.success(), "{out}");
    assert!(out.contains("not rendered yet") && out.contains("passed through"), "{out}");
    let strict = run(&["--strict"]);
    assert_eq!(strict.status.code(), Some(1), "{}", String::from_utf8_lossy(&strict.stdout));
    assert!(String::from_utf8_lossy(&strict.stderr).contains("--strict"));
}
