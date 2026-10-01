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

#[test]
fn image_size_warning_can_be_explained_denied_and_skipped() {
    let dir = render_fixture("image-size");
    let file = dir.join("r.scene.xml");
    let xml =
        std::fs::read_to_string(&file).unwrap().replace("width=\"16\" height=\"16\"", "width=\"32\" height=\"16\"");
    std::fs::write(&file, xml).unwrap();
    let file = file.to_str().unwrap();
    let o = run(&["validate", file, "--format", "json"]);
    assert_eq!(o.status.code(), Some(0));
    let r: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(r["files"][0]["diagnostics"].as_array().unwrap().iter().any(|d| d["code"] == "A07"));
    assert_eq!(run(&["validate", file, "--deny-warnings"]).status.code(), Some(1));
    assert_eq!(run(&["validate", file, "--no-assets", "--deny-warnings"]).status.code(), Some(0));
    let o = run(&["explain", "A07"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&o.stdout).contains("dimensions"));
}

#[test]
fn render_and_encode_surface_image_size_warnings() {
    let dir = render_fixture("image-size-render");
    let file = dir.join("r.scene.xml");
    let xml =
        std::fs::read_to_string(&file).unwrap().replace("width=\"16\" height=\"16\"", "width=\"32\" height=\"16\"");
    std::fs::write(&file, xml).unwrap();
    let file = file.to_str().unwrap();
    let png = dir.join("poster.png");
    let o = run(&["render", file, "-o", png.to_str().unwrap(), "--frame", "0"]);
    if no_gpu(&o) {
        return;
    }
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("warning[A07]"));
    let pattern = dir.join("encode_%03d.png");
    let o = run(&["encode", file, "-o", pattern.to_str().unwrap(), "--end", "0.1", "--json", "--strict"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let r: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("A07")));
}

fn no_gpu(o: &Output) -> bool {
    let e = String::from_utf8_lossy(&o.stderr);
    if o.status.code() == Some(2) && e.contains("no GPU adapter") {
        eprintln!("skipping: {e}");
        return true;
    }
    false
}

fn run_env(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_scene-render"));
    c.args(args).env("NO_COLOR", "1");
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().expect("binary runs")
}

#[test]
fn benchmark_rejects_unreadable_images_even_after_warmup() {
    let dir = render_fixture("bench-errors");
    std::fs::write(dir.join("bad.png"), b"not an image").unwrap();
    let scene = dir.join("bad.xml");
    std::fs::write(&scene, r#"<scene version="1.2"><project width="32" height="32" fps="1" duration="3"/><assets><image id="im" src="bad.png" width="32" height="32"/></assets><composition><layer id="bad" asset="im" start="2"/></composition></scene>"#).unwrap();
    for frames in ["0..3", "2..3"] {
        for pipelined in [false, true] {
            let mut args = vec!["render", scene.to_str().unwrap(), "--bench", "--frames", frames, "--strict"];
            if pipelined {
                args.push("--pipelined");
            }
            let out = run(&args);
            if no_gpu(&out) {
                return;
            }
            assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
            assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read image"));
        }
    }
}

#[test]
fn gpus_lists_adapters_and_marks_the_one_chosen() {
    let o = run(&["gpus"]);
    if no_gpu(&o) {
        return;
    }
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let text = String::from_utf8_lossy(&o.stdout);
    assert_eq!(text.lines().filter(|l| l.starts_with('*')).count(), 1, "{text}");
    let r: serde_json::Value = serde_json::from_slice(&run(&["gpus", "--json"]).stdout).unwrap();
    let list = r["adapters"].as_array().unwrap();
    assert!(!list.is_empty());
    assert_eq!(list.iter().filter(|a| a["chosen"] == true).count(), 1, "{r}");
    assert!(list.iter().all(|a| a["name"].is_string() && a["backend"].is_string() && a["software"].is_boolean()));
}

#[test]
fn an_unknown_adapter_name_is_an_error_that_lists_the_adapters() {
    let dir = render_fixture("adapter-name");
    let scene = dir.join("r.scene.xml").display().to_string();
    let o = run_env(&["render", &scene, "--bench", "--frames", "0..1"], &[("SR_GPU_ADAPTER", "no-such-gpu-xyz")]);
    if no_gpu(&o) {
        return;
    }
    assert_eq!(o.status.code(), Some(2));
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("no-such-gpu-xyz") && e.contains("available"), "{e}");
}

#[test]
fn rendering_on_a_software_adapter_warns() {
    let dir = render_fixture("software");
    let scene = dir.join("r.scene.xml").display().to_string();
    let o = run_env(&["render", &scene, "--bench", "--frames", "0..2"], &[("SR_GPU_ADAPTER", "llvmpipe")]);
    if o.status.code() == Some(2) {
        eprintln!("skipping: no llvmpipe adapter: {}", String::from_utf8_lossy(&o.stderr));
        return;
    }
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.contains("warning:") && text.contains("software"), "{text}");
    let pattern = dir.join("sw_%03d.png");
    let o = run_env(
        &["encode", &scene, "-o", pattern.to_str().unwrap(), "--end", "0.2", "--json"],
        &[("SR_GPU_ADAPTER", "llvmpipe")],
    );
    let r: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(r["render_adapter"]["software"], true, "{r}");
    let o = run_env(
        &["encode", &scene, "-o", pattern.to_str().unwrap(), "--end", "0.2"],
        &[("SR_GPU_ADAPTER", "llvmpipe")],
    );
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.contains("warning:") && text.contains("software adapter"), "{text}");
}

#[test]
fn quality_overrides_render_drafts_at_half_size_and_encode_at_full_size() {
    // the fixture scene is 48×32
    let dir = render_fixture("quality");
    let scene = dir.join("r.scene.xml").display().to_string();
    let png = dir.join("draft.png");
    let o = run(&["render", &scene, "--frame", "3", "-o", png.to_str().unwrap(), "--quality", "draft"]);
    if no_gpu(&o) {
        return;
    }
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let img = image::open(&png).unwrap();
    assert_eq!((img.width(), img.height()), (24, 16), "a draft frame is half size");
    let pattern = dir.join("enc_%03d.png");
    let o = run(&["encode", &scene, "-o", pattern.to_str().unwrap(), "--end", "0.2", "--quality", "draft", "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let r: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(r["quality"], "draft", "{r}");
    let img = image::open(dir.join("enc_000.png")).unwrap();
    assert_eq!((img.width(), img.height()), (48, 32), "delivery scales a draft to the output size");
    let o = run(&["render", &scene, "--frame", "0", "-o", png.to_str().unwrap(), "--quality", "rough"]);
    assert_eq!(o.status.code(), Some(2), "an unknown quality is a usage error");
}

/// A 1 s, 10 fps scene: a shape for the whole second and one only in its second half.
fn two_halves(name: &str, project: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("sr-cli-changed-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scene = dir.join("h.scene.xml");
    std::fs::write(
        &scene,
        format!(
            r##"<scene version="1.1"><project width="32" height="24" fps="10" duration="1" background="#102030" {project}/>
<composition><shape id="a" shape="rect" x="2" y="2" width="10" height="10" fill="#FF0000"/>
<shape id="b" shape="ellipse" start="0.5" x="16" y="8" width="12" height="12" fill="#00FF00"/></composition></scene>"##
        ),
    )
    .unwrap();
    (dir, scene)
}

fn rendered(o: &Output) -> (usize, usize) {
    let text = String::from_utf8_lossy(&o.stdout);
    let line = text.lines().find(|l| l.starts_with("rendered ")).unwrap_or_else(|| panic!("{text}"));
    let w: Vec<&str> = line.split_whitespace().collect();
    (w[1].parse().unwrap(), w[3].parse().unwrap())
}

#[test]
fn changed_only_renders_the_frames_an_edit_changed() {
    let (dir, scene) = two_halves("second-half", "");
    let (scene, out) = (scene.display().to_string(), dir.join("f_%03d.png").display().to_string());
    let args = ["render", &scene, "--frames", "0..10", "-o", &out, "--changed-only"];
    let o = run(&args);
    if no_gpu(&o) {
        return;
    }
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(rendered(&o), (10, 10));
    assert_eq!(rendered(&run(&args)), (0, 10), "nothing changed");
    let early = std::fs::metadata(dir.join("f_002.png")).unwrap().modified().unwrap();
    // recolour the shape that only the second half shows
    let text = std::fs::read_to_string(&scene).unwrap().replace("#00FF00", "#0000FF");
    std::fs::write(&scene, text).unwrap();
    assert_eq!(rendered(&run(&args)), (5, 10), "only frames 5..9 show the edited shape");
    assert_eq!(std::fs::metadata(dir.join("f_002.png")).unwrap().modified().unwrap(), early);
    let img = image::open(dir.join("f_007.png")).unwrap().to_rgba8();
    assert!(img.get_pixel(22, 14)[2] > 200, "the edit is in the re-rendered frames");
    // a missing output is rendered again
    std::fs::remove_file(dir.join("f_001.png")).unwrap();
    assert_eq!(rendered(&run(&args)), (1, 10));
    // other settings are other pixels
    assert_eq!(rendered(&run(&[&args[..], &["--quality", "draft"]].concat())), (10, 10));
}

#[test]
fn with_motion_blur_an_edit_renders_every_frame() {
    let (dir, scene) = two_halves("blur", r#"motionBlur="true""#);
    let (scene, out) = (scene.display().to_string(), dir.join("f_%03d.png").display().to_string());
    let args = ["render", &scene, "--frames", "0..10", "-o", &out, "--changed-only"];
    let o = run(&args);
    if no_gpu(&o) {
        return;
    }
    assert_eq!(rendered(&o), (10, 10));
    let text = std::fs::read_to_string(&scene).unwrap().replace("#00FF00", "#0000FF");
    std::fs::write(&scene, text).unwrap();
    let o = run(&args);
    assert_eq!(rendered(&o), (10, 10));
    assert!(String::from_utf8_lossy(&o.stdout).contains("motion blur"), "{}", String::from_utf8_lossy(&o.stdout));
}

#[test]
fn watch_renders_again_what_an_edit_changed() {
    let (dir, scene) = two_halves("watch", "");
    let (scene_s, out) = (scene.display().to_string(), dir.join("w_%03d.png").display().to_string());
    let mut child = Command::new(env!("CARGO_BIN_EXE_scene-render"))
        .args(["watch", &scene_s, "--frames", "0..10", "-o", &out, "--interval", "100", "--max-runs", "2"])
        .env("NO_COLOR", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // wait for the first pass to write every frame
    let started = std::time::Instant::now();
    while !(0..10).all(|k| dir.join(format!("w_{k:03}.png")).exists()) {
        if let Some(status) = child.try_wait().unwrap() {
            let o = child.wait_with_output().unwrap();
            if String::from_utf8_lossy(&o.stderr).contains("no GPU adapter") {
                return;
            }
            panic!("watch exited early with {status}: {}", String::from_utf8_lossy(&o.stderr));
        }
        assert!(started.elapsed().as_secs() < 120, "the first pass did not finish");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    let text = std::fs::read_to_string(&scene).unwrap().replace("#00FF00", "#0000FF");
    std::fs::write(&scene, text).unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let text = String::from_utf8_lossy(&o.stdout);
    let runs: Vec<&str> = text.lines().filter(|l| l.starts_with("rendered ")).collect();
    assert_eq!(runs, ["rendered 10 of 10 frames (0 unchanged)", "rendered 5 of 10 frames (5 unchanged)"], "{text}");
}

#[test]
fn bench_and_stats_report_gpu_time_where_the_adapter_measures_it() {
    // llvmpipe has timestamp queries (as on CI); the GPU time is reported apart from the CPU's
    let dir = render_fixture("gpu-time");
    let scene = dir.join("r.scene.xml").display().to_string();
    let o = run_env(&["render", &scene, "--bench", "--stats", "--frames", "0..3"], &[("SR_GPU_ADAPTER", "llvmpipe")]);
    if o.status.code() == Some(2) {
        eprintln!("skipping: no llvmpipe adapter: {}", String::from_utf8_lossy(&o.stderr));
        return;
    }
    let text = String::from_utf8_lossy(&o.stdout);
    let line = text.lines().find(|l| l.starts_with("gpu:")).unwrap_or_else(|| panic!("no GPU time line: {text}"));
    assert!(line.contains("median") && line.contains("ms"), "{line}");
    let stats: Vec<serde_json::Value> = String::from_utf8_lossy(&o.stderr)
        .lines()
        .filter(|l| l.starts_with('{'))
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(stats.len(), 3);
    for s in &stats {
        assert!(s["gpu"]["frame_ms"].as_f64().is_some_and(|v| v > 0.0), "{s}");
        assert!(s["gpu"]["passes"].is_array(), "{s}");
    }
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
    // the shape renders: nothing is reported as not rendered yet
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
    for field in ["name", "backend", "device_type"] {
        assert!(!r["render_adapter"][field].as_str().unwrap_or("").is_empty(), "{r}");
    }
    assert!(r["unsupported"].as_array().is_none_or(|u| u.is_empty()), "{}", r["unsupported"]);
    assert!(std::fs::metadata(dir.join("clip.mp4")).unwrap().len() > 100);
    let seq = dir.join("f_%03d.png").display().to_string();
    let o = run(&["encode", &scene, "-o", &seq, "--end", "0.3"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.contains("(3 file(s), png)"), "{text}");
    assert!(text.contains("render adapter:") && text.contains("GPU/readback blocked"), "{text}");
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

#[test]
fn strict_parallel_encode_retains_findings_from_early_chunks() {
    let dir = render_fixture("parallel-findings");
    let scene = dir.join("scene.xml");
    std::fs::write(
        &scene,
        r##"<scene version="1.2">
      <project width="16" height="16" fps="1" duration="60" background="#000000"/>
      <output path="clip.mkv" codec="ffv1" audio="false"/>
      <composition><adjustment id="adj" start="0" end="1" effects="fx"/></composition>
      <effects><effect id="fx" type="echo"/></effects></scene>"##,
    )
    .unwrap();
    for workers in ["1", "2"] {
        let o =
            run(&["encode", scene.to_str().unwrap(), "--parallel", workers, "--hw", "software", "--strict", "--json"]);
        if no_gpu(&o) {
            return;
        }
        assert_eq!(o.status.code(), Some(1), "workers={workers}: {}", String::from_utf8_lossy(&o.stdout));
        let report: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        assert!(report["unsupported"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().contains("echo on an adjustment")));
    }
}

#[test]
fn strict_encode_rejects_unresolved_placeholders_and_reports_them() {
    let dir = render_fixture("strict-template");
    let scene = dir.join("r.scene.xml");
    std::fs::write(
        &scene,
        r#"<scene version="1.1"><project width="64" height="36" fps="10" duration="0.1"/>
      <assets><text id="txt" text="Hello {{missing}}" width="64" height="36" size="12"/></assets>
      <composition><layer id="title" asset="txt"/></composition></scene>"#,
    )
    .unwrap();
    let output = dir.join("f_%03d.png");
    let args = ["encode", scene.to_str().unwrap(), "-o", output.to_str().unwrap(), "--json"];
    let normal = run(&args);
    if no_gpu(&normal) {
        return;
    }
    assert_eq!(normal.status.code(), Some(0), "{}", String::from_utf8_lossy(&normal.stderr));
    let strict = run(&[args.as_slice(), &["--strict"]].concat());
    assert_eq!(strict.status.code(), Some(1), "{}", String::from_utf8_lossy(&strict.stdout));
    for o in [&normal, &strict] {
        let report: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        assert!(report["evaluation_warnings"].as_array().unwrap().iter().any(|d| d["code"] == "E16"));
    }
    let human = run(&["encode", scene.to_str().unwrap(), "-o", output.to_str().unwrap()]);
    assert!(String::from_utf8_lossy(&human.stdout).contains("warning[E16]"));
}

#[test]
fn regression_incremental_transitions_change_pixels() {
    let (dir, scene) = two_halves("transition-regression", "");
    let xml = r##"<scene version="1.2"><project width="64" height="64" fps="10" duration="4"/>
      <composition><shape id="a" shape="rect" width="64" height="64" fill="#FF0000" end="2"/>
      <shape id="b" shape="rect" width="64" height="64" fill="#0000FF" start="2"/>
      <transition type="crossfade" from="a" to="b" duration="1" curve="linear"/></composition></scene>"##;
    std::fs::write(&scene, xml).unwrap();
    let output = dir.join("transition.png");
    let args = ["render", scene.to_str().unwrap(), "--time", "1.75", "-o", output.to_str().unwrap(), "--changed-only"];
    let first = run(&args);
    if no_gpu(&first) {
        return;
    }
    assert!(first.status.success(), "{first:?}");
    let before = image::open(&output).unwrap().to_rgba8();
    std::fs::write(&scene, xml.replace("crossfade", "wipe")).unwrap();
    let second = run(&args);
    assert!(second.status.success(), "{second:?}");
    assert_eq!(rendered(&second), (1, 1));
    assert_ne!(image::open(&output).unwrap().to_rgba8(), before);
    // Static attributes are skipped by FrameTransition's serialization, but change pixels too.
    let wipe = image::open(&output).unwrap().to_rgba8();
    std::fs::write(
        &scene,
        xml.replace("crossfade", "wipe").replace("curve=\"linear\"", "curve=\"linear\" direction=\"right\""),
    )
    .unwrap();
    assert_eq!(rendered(&run(&args)), (1, 1));
    assert_ne!(image::open(&output).unwrap().to_rgba8(), wipe);
}

#[test]
fn regression_incremental_strict_repeats_shader_diagnostics() {
    let (dir, scene) = two_halves("strict-regression", "");
    std::fs::write(dir.join("broken.glsl"), "vec4 effect(vec2 uv) { return missingVariable; }").unwrap();
    std::fs::write(
        &scene,
        r##"<scene version="1.2"><project width="16" height="16" fps="10" duration="1"/>
      <composition><shape id="s" shape="rect" width="16" height="16" fill="#FF0000" effects="bad"/></composition>
      <effects><effect id="bad" type="shader" src="broken.glsl"/></effects></scene>"##,
    )
    .unwrap();
    let output = dir.join("strict.png");
    let args = ["render", scene.to_str().unwrap(), "-o", output.to_str().unwrap(), "--changed-only", "--strict"];
    for _ in 0..2 {
        let o = run(&args);
        if no_gpu(&o) {
            return;
        }
        assert_eq!(o.status.code(), Some(1), "{o:?}");
        assert!(String::from_utf8_lossy(&o.stdout).contains("GLSL"), "{o:?}");
    }
    // A non-strict render must not let a later strict run skip its diagnostics either.
    assert!(run(&args[..args.len() - 1]).status.success());
    assert_eq!(run(&args).status.code(), Some(1));
}

#[test]
fn regression_watch_reloads_changed_image_bytes() {
    use std::io::{BufRead, BufReader};
    let (dir, scene) = two_halves("watch-image-regression", "");
    let source = dir.join("source.png");
    image::RgbaImage::from_pixel(16, 16, image::Rgba([255, 0, 0, 255])).save(&source).unwrap();
    std::fs::write(
        &scene,
        r#"<scene version="1.2"><project width="16" height="16" fps="10" duration="1"/>
      <assets><image id="img" src="source.png" width="16" height="16"/></assets>
      <composition><layer id="l" asset="img"/></composition></scene>"#,
    )
    .unwrap();
    let output = dir.join("watch.png");
    let mut child = Command::new(env!("CARGO_BIN_EXE_scene-render"))
        .args([
            "watch",
            scene.to_str().unwrap(),
            "--frames",
            "0..1",
            "-o",
            output.to_str().unwrap(),
            "--interval",
            "50",
            "--max-runs",
            "2",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if line.unwrap().contains("watching") {
                let _ = tx.send(());
            }
        }
    });
    if rx.recv_timeout(std::time::Duration::from_secs(60)).is_err() {
        let _ = child.kill();
        let o = child.wait_with_output().unwrap();
        reader.join().unwrap();
        if no_gpu(&o) {
            return;
        }
        panic!("watch did not become ready: {o:?}");
    }
    image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 0, 255, 255])).save(&source).unwrap();
    let start = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed().as_secs() > 60 {
            let _ = child.kill();
            panic!("watch did not finish");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let o = child.wait_with_output().unwrap();
    reader.join().unwrap();
    assert!(o.status.success(), "{o:?}");
    assert_eq!(image::open(output).unwrap().to_rgba8().get_pixel(8, 8).0, [0, 0, 255, 255]);
}

#[test]
fn regression_sequence_edits_invalidate_incremental_frames() {
    let (dir, scene) = two_halves("sequence-input", "");
    let source = dir.join("f00.png");
    image::RgbaImage::from_pixel(16, 16, image::Rgba([255, 0, 0, 255])).save(&source).unwrap();
    std::fs::write(
        &scene,
        r#"<scene version="1.2"><project width="16" height="16" fps="10" duration="0.1"/>
      <assets><imageSequence id="seq" src="f%02d.png" width="16" height="16" fps="10" first="0" last="0"/></assets>
      <composition><layer id="l" asset="seq"/></composition></scene>"#,
    )
    .unwrap();
    let output = dir.join("out.png");
    let args = ["render", scene.to_str().unwrap(), "-o", output.to_str().unwrap(), "--changed-only"];
    let first = run(&args);
    if no_gpu(&first) {
        return;
    }
    assert!(first.status.success(), "{first:?}");
    assert_eq!(rendered(&run(&args)), (0, 1));
    image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 0, 255, 255])).save(&source).unwrap();
    let edited = run(&args);
    assert!(edited.status.success(), "{edited:?}");
    assert_eq!(rendered(&edited), (1, 1));
    assert_eq!(image::open(output).unwrap().to_rgba8().get_pixel(8, 8).0, [0, 0, 255, 255]);
}

#[test]
fn regression_transition_shader_edits_invalidate_incremental_frames() {
    let (dir, scene) = two_halves("shader-input", "");
    let shader = dir.join("transition.glsl");
    std::fs::write(&shader, "vec4 transition(vec2 uv) { return vec4(1.0,0.0,0.0,1.0); }").unwrap();
    std::fs::write(&scene, r##"<scene version="1.2"><project width="16" height="16" fps="10" duration="4"/>
      <composition><shape id="a" shape="rect" width="16" height="16" fill="#FF0000" end="2"/>
      <shape id="b" shape="rect" width="16" height="16" fill="#0000FF" start="2"/>
      <transition type="shader" shader="transition.glsl" from="a" to="b" duration="1" curve="linear"/></composition></scene>"##).unwrap();
    let output = dir.join("out.png");
    let args = [
        "render",
        scene.to_str().unwrap(),
        "--time",
        "1.75",
        "-o",
        output.to_str().unwrap(),
        "--changed-only",
        "--strict",
    ];
    let first = run(&args);
    if no_gpu(&first) {
        return;
    }
    assert!(first.status.success(), "{first:?}");
    std::fs::write(&shader, "vec4 transition(vec2 uv) { return vec4(0.0,0.0,1.0,1.0); }").unwrap();
    let edited = run(&args);
    assert!(edited.status.success(), "{edited:?}");
    assert_eq!(rendered(&edited), (1, 1));
    assert_eq!(image::open(output).unwrap().to_rgba8().get_pixel(8, 8).0, [0, 0, 255, 255]);
}

#[test]
fn regression_incremental_time_changes_update_captions_without_nodes() {
    let (dir, scene) = two_halves("caption-time", "");
    std::fs::write(
        &scene,
        r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="1" background="#000000"/>
      <styles><textStyle id="cs" size="18" color="#FFFFFF"/></styles><composition/>
      <captions><captionTrack id="cc" language="en" mode="burn" style="cs" x="32" y="2" width="56">
      <cue start="0" end="0.5" text="AB"/></captionTrack></captions></scene>"##,
    )
    .unwrap();
    let output = dir.join("out.png");
    let args = ["render", scene.to_str().unwrap(), "-o", output.to_str().unwrap(), "--changed-only"];
    let first = run(&[args.as_slice(), &["--time", "0"]].concat());
    if no_gpu(&first) {
        return;
    }
    assert!(first.status.success(), "{first:?}");
    assert!(image::open(&output).unwrap().to_rgba8().pixels().any(|p| p[0] > 0));
    let later = run(&[args.as_slice(), &["--time", "0.8"]].concat());
    assert!(later.status.success(), "{later:?}");
    assert_eq!(rendered(&later), (1, 1));
    assert!(image::open(output).unwrap().to_rgba8().pixels().all(|p| p.0 == [0, 0, 0, 255]));
}
