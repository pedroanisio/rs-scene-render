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
fn huge_sequences_are_rejected_before_dependency_expansion() {
    let (dir, scene) = two_halves("huge-sequence-dependencies", "");
    std::fs::write(&scene, r#"<scene version="1.2"><project width="16" height="16" fps="1" duration="1"/>
      <assets><imageSequence id="seq" src="f_%d.png" first="0" last="9223372036854775807" fps="1" width="16" height="16"/></assets><composition/></scene>"#).unwrap();
    for command in ["watch", "render"] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_scene-render"))
            .args([command, scene.to_str().unwrap(), "-o", dir.join("out.png").to_str().unwrap()])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let start = std::time::Instant::now();
        while child.try_wait().unwrap().is_none() && start.elapsed().as_secs() < 2 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("{command} did not bound image-sequence dependency expansion");
        }
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success(), "{output:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("image sequence") && error.contains("limit"), "{command}: {output:?}");
    }
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
fn changed_only_invalidates_persistent_shader_history() {
    let (dir, scene) = two_halves("persistent-history", "");
    std::fs::write(
        dir.join("history.fs"),
        r#"/*{"INPUTS":[{"NAME":"inputImage","TYPE":"image"}],"PASSES":[{"TARGET":"acc","PERSISTENT":true},{}]}*/
void main() {
    if (PASSINDEX == 0) {
        vec3 old = IMG_NORM_PIXEL(acc, isf_FragNormCoord).rgb;
        vec3 now = IMG_NORM_PIXEL(inputImage, isf_FragNormCoord).rgb;
        gl_FragColor = vec4(max(old, now), 1.0);
    } else { gl_FragColor = IMG_NORM_PIXEL(acc, isf_FragNormCoord); }
}"#,
    )
    .unwrap();
    let xml = r##"<scene version="1.3"><project width="32" height="32" fps="10" duration="0.3" background="#000000"/>
    <composition><shape id="early" shape="rect" width="32" height="32" fill="#FF0000" end="0.1"/><adjustment id="history" effects="feedback"/></composition>
    <effects><effect id="feedback" type="shader" src="history.fs" space="raw"/></effects></scene>"##;
    std::fs::write(&scene, xml).unwrap();
    let out = dir.join("f_%03d.png");
    let args = [
        "render",
        scene.to_str().unwrap(),
        "--frames",
        "0..3",
        "-o",
        out.to_str().unwrap(),
        "--changed-only",
        "--strict",
    ];
    let first = run(&args);
    if no_gpu(&first) {
        return;
    }
    assert!(first.status.success(), "{first:?}");
    assert_eq!(rendered(&first), (3, 3));
    assert_eq!(rendered(&run(&args)), (0, 3));
    std::fs::write(&scene, xml.replace("#FF0000", "#00FF00")).unwrap();
    let edited = run(&args);
    assert!(edited.status.success(), "{edited:?}");
    assert_eq!(rendered(&edited), (3, 3), "all frames depend on the edited first frame");
    let fresh = dir.join("fresh_%03d.png");
    let control =
        run(&["render", scene.to_str().unwrap(), "--frames", "0..3", "-o", fresh.to_str().unwrap(), "--strict"]);
    assert!(control.status.success(), "{control:?}");
    for frame in 0..3 {
        let actual = image::open(dir.join(format!("f_{frame:03}.png"))).unwrap().to_rgba8();
        let expected = image::open(dir.join(format!("fresh_{frame:03}.png"))).unwrap().to_rgba8();
        assert_eq!(actual, expected);
        assert!(actual.get_pixel(16, 16)[1] > 250);
    }
    std::fs::remove_dir_all(dir).unwrap();
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
fn bake_volume_exports_native_fields_without_replacing_existing_output() {
    let dir = std::env::temp_dir().join(format!("sr-cli-bake-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let scene = dir.join("source.scene.xml");
    let output = dir.join("take");
    std::fs::write(&scene,r#"<scene version="1.3"><project width="16" height="16" fps="10" duration="0.3"/><composition><object3D id="cloud" primitive="volume"><pyro width="4" height="4" depth="4" voxelSize="1" dt="0.1"><pyroSource radius="1.5" densityRate="1"/></pyro></object3D></composition></scene>"#).unwrap();
    let args = ["bake-volume", scene.to_str().unwrap(), "--object", "cloud", "-o", output.to_str().unwrap(), "--json"];
    let result = run(&args);
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["frames"], 3);
    assert_eq!(report["sha256"].as_str().unwrap().len(), 64);
    assert!(output.join("manifest.srvseq").is_file());
    let before = std::fs::read(output.join("manifest.srvseq")).unwrap();
    assert!(!run(&args).status.success());
    assert_eq!(std::fs::read(output.join("manifest.srvseq")).unwrap(), before);
    let replacement = format!(
        r#"<scene version="1.3"><project width="16" height="16" fps="10" duration="0.3"/><assets><volume id="cache" src="take/manifest.srvseq" format="srvseq" sha256="{}" temperatureGrid="temperature"/></assets><composition><object3D id="cloud" primitive="volume" volume="cache"/></composition></scene>"#,
        report["sha256"].as_str().unwrap()
    );
    let baked = dir.join("baked.scene.xml");
    std::fs::write(&baked, replacement).unwrap();
    let valid = run(&["validate", baked.to_str().unwrap()]);
    assert!(valid.status.success(), "{}", String::from_utf8_lossy(&valid.stdout));
    let frame = std::fs::read_dir(&output)
        .unwrap()
        .map(|p| p.unwrap().path())
        .find(|p| p.extension().is_some_and(|s| s == "srvol"))
        .unwrap();
    let mut bytes = std::fs::read(&frame).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&frame, bytes).unwrap();
    let invalid = run(&["validate", baked.to_str().unwrap()]);
    assert!(!invalid.status.success(), "modified cache was accepted");
    std::fs::remove_dir_all(dir).unwrap();
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
    watch_reloads_image_bytes("watch-image-regression", "", "source.png");
}

#[test]
fn watch_reloads_bound_image_bytes() {
    watch_reloads_image_bytes(
        "watch-bound-image",
        r#"<parameters><param id="picture" type="string" default="source.png"/><bind param="picture" target="img" property="src"/></parameters>"#,
        "original.png",
    );
}

fn watch_reloads_image_bytes(name: &str, parameters: &str, authored_src: &str) {
    use std::io::{BufRead, BufReader};
    let (dir, scene) = two_halves(name, "");
    let source = dir.join("source.png");
    image::RgbaImage::from_pixel(16, 16, image::Rgba(GREEN)).save(dir.join("original.png")).unwrap();
    image::RgbaImage::from_pixel(16, 16, image::Rgba([255, 0, 0, 255])).save(&source).unwrap();
    std::fs::write(
        &scene,
        format!(
            r#"<scene version="1.2"><project width="16" height="16" fps="10" duration="1"/>{parameters}
      <assets><image id="img" src="{authored_src}" width="16" height="16"/></assets>
      <composition><layer id="l" asset="img"/></composition></scene>"#
        ),
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
    assert_eq!(centre(output.clone()), RED);
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
fn mesh_sequence_material_edits_invalidate_incremental_pixels() {
    let (dir, scene) = two_halves("mesh-sequence-material", "");
    std::fs::write(dir.join("frame-0.obj"), "mtllib surface.mtl\nusemtl surface\nv -0.3 -0.3 0\nv 0.3 -0.3 0\nv 0.3 0.3 0\nv -0.3 0.3 0\nf 1 2 3\nf 1 3 4\n").unwrap();
    let material = dir.join("surface.mtl");
    std::fs::write(&material, "newmtl surface\nKd 1 0 0\n").unwrap();
    std::fs::write(&scene, r##"<scene version="1.3"><project width="32" height="32" fps="1" duration="1" background="#000000"/><assets><meshSequence id="frames" src="frame-%d.obj" first="0" last="0" fps="1"/></assets><composition><camera id="cam" x="16" y="16" z="-200" projection="orthographic" orthoHeight="32"/><object3D id="cache" primitive="mesh" mesh="frames" x="16" y="16"/></composition></scene>"##).unwrap();
    let output = dir.join("out.png");
    let args = ["render", scene.to_str().unwrap(), "-o", output.to_str().unwrap(), "--changed-only", "--strict"];
    let first = run(&args);
    if no_gpu(&first) {
        return;
    }
    assert!(first.status.success(), "{first:?}");
    let red = image::open(&output).unwrap().to_rgba8().get_pixel(16, 16).0;
    assert!(red[0] > red[2], "red material must be visible: {red:?}");
    assert_eq!(rendered(&run(&args)), (0, 1));
    std::fs::write(material, "newmtl surface\nKd 0 0 1\n").unwrap();
    let edited = run(&args);
    assert!(edited.status.success(), "{edited:?}");
    assert_eq!(rendered(&edited), (1, 1));
    let blue = image::open(&output).unwrap().to_rgba8().get_pixel(16, 16).0;
    assert!(blue[2] > blue[0], "updated material must change pixels: {blue:?}");
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

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// A document that includes `parts/child.scene.xml`, which shows `parts/logo.png` (red).
fn including(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let (dir, scene) = two_halves(name, "");
    std::fs::create_dir_all(dir.join("parts")).unwrap();
    let logo = dir.join("parts/logo.png");
    image::RgbaImage::from_pixel(16, 16, image::Rgba(RED)).save(&logo).unwrap();
    std::fs::write(
        dir.join("parts/child.scene.xml"),
        r#"<scene version="1.2"><project width="16" height="16" fps="10" duration="1"/>
      <assets><image id="logo" src="logo.png" width="16" height="16"/></assets>
      <composition><layer id="l" asset="logo"/></composition></scene>"#,
    )
    .unwrap();
    std::fs::write(
        &scene,
        r#"<scene version="1.2"><project width="16" height="16" fps="10" duration="1"/>
      <composition><include id="inc" src="parts/child.scene.xml"/></composition></scene>"#,
    )
    .unwrap();
    (dir, scene, logo)
}

fn centre(path: PathBuf) -> [u8; 4] {
    image::open(path).unwrap().to_rgba8().get_pixel(8, 8).0
}

#[test]
fn an_included_documents_asset_invalidates_incremental_frames() {
    let (dir, scene, logo) = including("include-asset");
    let out = dir.join("f_%03d.png");
    let args = ["render", scene.to_str().unwrap(), "--frames", "0..2", "-o", out.to_str().unwrap(), "--changed-only"];
    let first = run(&args);
    if no_gpu(&first) {
        return;
    }
    assert!(first.status.success(), "{first:?}");
    assert_eq!(centre(dir.join("f_001.png")), RED);
    assert_eq!(rendered(&run(&args)), (0, 2));
    image::RgbaImage::from_pixel(16, 16, image::Rgba(BLUE)).save(&logo).unwrap();
    assert_eq!(rendered(&run(&args)), (2, 2));
    assert_eq!(centre(dir.join("f_001.png")), BLUE);
}

#[test]
fn watch_notices_an_included_documents_asset() {
    use std::io::{BufRead, BufReader};
    let (dir, scene, logo) = including("include-watch");
    let output = dir.join("watch.png");
    let mut child = Command::new(env!("CARGO_BIN_EXE_scene-render"))
        .args(["watch", scene.to_str().unwrap(), "--frames", "0..1", "-o", output.to_str().unwrap()])
        .args(["--interval", "50", "--max-runs", "2"])
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
    assert_eq!(centre(output.clone()), RED);
    image::RgbaImage::from_pixel(16, 16, image::Rgba(BLUE)).save(&logo).unwrap();
    let start = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed().as_secs() > 20 {
            let _ = child.kill();
            panic!("watch did not notice the included document's image");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let o = child.wait_with_output().unwrap();
    reader.join().unwrap();
    assert!(o.status.success(), "{o:?}");
    // the renderer kept from the first run does not hold on to the old image
    assert_eq!(centre(output), BLUE);
}

#[test]
fn every_render_keeps_the_recorded_fingerprints_true() {
    let (dir, scene) = two_halves("sidecar-sync", "");
    std::fs::write(dir.join("bad.png"), b"not an image").unwrap();
    let doc = |fill: &str, bad: bool| {
        let (asset, layer) = if bad {
            (
                r#"<assets><image id="im" src="bad.png" width="16" height="16"/></assets>"#,
                r#"<layer id="bad" asset="im" start="0.2"/>"#,
            )
        } else {
            ("", "")
        };
        let xml = format!(
            r##"<scene version="1.2"><project width="16" height="16" fps="10" duration="0.3"/>{asset}
      <composition><shape id="s" shape="rect" width="16" height="16" fill="{fill}"/>{layer}</composition></scene>"##
        );
        std::fs::write(&scene, xml).unwrap();
    };
    let out = dir.join("f_%03d.png");
    let plain = ["render", scene.to_str().unwrap(), "--frames", "0..3", "-o", out.to_str().unwrap()];
    let changed = [plain.as_slice(), &["--changed-only"]].concat();
    let frame = |k: u32| centre(dir.join(format!("f_{k:03}.png")));
    doc("#FF0000", false);
    let first = run(&changed);
    if no_gpu(&first) {
        return;
    }
    assert_eq!(rendered(&first), (3, 3));
    // a render without the flag writes the same files
    doc("#00FF00", false);
    assert!(run(&plain).status.success());
    assert_eq!(frame(0), GREEN);
    doc("#FF0000", false);
    assert_eq!(rendered(&run(&changed)), (3, 3), "the files are not what the fingerprints say");
    assert_eq!(frame(0), RED);
    // a run that stops at a frame it cannot render has written the ones before it
    doc("#00FF00", true);
    let stopped = run(&changed);
    assert_eq!(stopped.status.code(), Some(1), "{stopped:?}");
    assert_eq!((frame(0), frame(1), frame(2)), (GREEN, GREEN, RED));
    doc("#FF0000", false);
    assert_eq!(rendered(&run(&changed)), (2, 3));
    assert_eq!((frame(0), frame(1), frame(2)), (RED, RED, RED));
}

#[test]
fn a_percent_sign_in_a_directory_is_not_a_frame_pattern() {
    let (dir, scene) = two_halves("percent-dir", "");
    let scene = scene.to_str().unwrap();
    std::fs::create_dir_all(dir.join("50%")).unwrap();
    let frames = dir.join("50%/f_%03d.png");
    let o = run(&["render", scene, "--frames", "0..2", "-o", frames.to_str().unwrap()]);
    if no_gpu(&o) {
        return;
    }
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(dir.join("50%/f_000.png").is_file() && dir.join("50%/f_001.png").is_file());
    let one = dir.join("50%/still.png");
    assert_eq!(run(&["render", scene, "-o", one.to_str().unwrap()]).status.code(), Some(0));
    assert!(one.is_file());
    let o = run(&["render", scene, "--frames", "0..2", "-o", one.to_str().unwrap()]);
    assert!(String::from_utf8_lossy(&o.stderr).contains("names one file"), "{o:?}");
    // encode takes the sequence codec from the file name, and writes the frames
    let encoded = dir.join("75%/e_%03d.png");
    let o = run(&["encode", scene, "-o", encoded.to_str().unwrap(), "--end", "0.2"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("(2 file(s), png)"), "{o:?}");
    assert!(dir.join("75%/e_000.png").is_file() && dir.join("75%/e_001.png").is_file());
}

#[test]
fn watch_reloads_edited_caption_text() {
    use std::io::{BufRead, BufReader};
    let (dir, scene) = two_halves("watch-caption-reload", "");
    let xml = r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="1" background="#000000"/>
      <styles><textStyle id="cs" size="18" color="#FFFFFF"/></styles><composition/>
      <captions><captionTrack id="cc" language="en" mode="burn" style="cs" x="32" y="2" width="56">
      <cue start="0" end="1" text="ABC"/></captionTrack></captions></scene>"##;
    std::fs::write(&scene, xml).unwrap();
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
    let before = image::open(&output).unwrap().to_rgba8();
    std::fs::write(&scene, xml.replace("ABC", "XYZ")).unwrap();
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
    let fresh = dir.join("fresh.png");
    let o = run(&["render", scene.to_str().unwrap(), "-o", fresh.to_str().unwrap()]);
    assert!(o.status.success(), "{o:?}");
    let expected = image::open(fresh).unwrap().to_rgba8();
    let actual = image::open(output).unwrap().to_rgba8();
    assert!(before != expected, "the edited text changes the image");
    assert!(actual == expected, "watch must render the edited captions like a fresh renderer");
}

#[test]
fn bound_assets_invalidate_changed_frames() {
    let (dir, scene) = two_halves("bound-assets", "");
    let image =
        |name: &str, color| image::RgbaImage::from_pixel(16, 16, image::Rgba(color)).save(dir.join(name)).unwrap();
    image("base.png", GREEN);
    image("actual.png", RED);
    let xml = r#"<scene version="1.2"><project width="16" height="16" fps="1" duration="1"/>
      <parameters><param id="picture" type="string" default="base.png"/><bind param="picture" target="img" property="src"/></parameters>
      <assets><image id="img" src="base.png" width="16" height="16"/></assets><composition><layer id="l" asset="img"/></composition></scene>"#;
    std::fs::write(&scene, xml).unwrap();
    let output = dir.join("bound.png");
    let args = [
        "render",
        scene.to_str().unwrap(),
        "--param",
        "picture=actual.png",
        "--changed-only",
        "-o",
        output.to_str().unwrap(),
    ];
    let first = run(&args);
    if no_gpu(&first) {
        return;
    }
    assert_eq!(rendered(&first), (1, 1));
    assert_eq!(centre(output.clone()), RED);
    assert_eq!(rendered(&run(&args)), (0, 1));
    image("actual.png", BLUE);
    assert_eq!(rendered(&run(&args)), (1, 1), "the resolved input changed");
    assert_eq!(centre(output), BLUE);
}

#[test]
fn plain_exports_keep_the_final_partial_frame() {
    let (dir, scene) = two_halves("partial-frame-export", "");
    for (name, segment) in [("plain", ""), ("segmented", r#"<segment from="0" to="0.21"/>"#)] {
        let xml = format!(
            r##"<scene version="1.2"><project width="16" height="16" fps="30" duration="0.21"/>
          <output path="{name}_%03d.png" codec="png-sequence" fps="10">{segment}</output>
          <composition><shape id="s" shape="rect" width="16" height="16" fill="#FF0000"><animate property="fill"><key time="0" value="#FF0000" interpolation="hold"/><key time="0.2" value="#0000FF"/></animate></shape></composition></scene>"##
        );
        std::fs::write(&scene, xml).unwrap();
        let o = run(&["encode", scene.to_str().unwrap()]);
        if no_gpu(&o) {
            return;
        }
        assert!(o.status.success(), "{o:?}");
        assert!(dir.join(format!("{name}_002.png")).is_file(), "{name}: dropped the frame starting at 0.2 seconds");
        assert!(!dir.join(format!("{name}_003.png")).exists());
        assert_eq!(centre(dir.join(format!("{name}_002.png"))), BLUE);
    }
}

fn three_d_fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sr-cli-3d-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let xml = r##"<scene version="1.3"><project width="64" height="48" fps="10" duration="1" background="#102030"/>
<composition><object3D id="ball" primitive="sphere" radius="10" x="32" y="24"/></composition></scene>"##;
    std::fs::write(dir.join("r.scene.xml"), xml).unwrap();
    dir
}

/// True when the machine exposes an OpenGL adapter that can open a device (the test needs one to select): a listed adapter
/// whose device cannot be created, as on a WSL2 host without a working D3D12 driver, is a skip like a missing one.
fn has_gl_adapter() -> bool {
    let o = run_env(&["gpus", "--json"], &[("SR_GPU_BACKEND", "gl")]);
    let Ok(r) = serde_json::from_slice::<serde_json::Value>(&o.stdout) else { return false };
    if r["adapters"].as_array().is_none_or(|a| a.is_empty()) {
        return false;
    }
    let dir = render_fixture("gl-probe");
    let scene = dir.join("r.scene.xml").display().to_string();
    let o = run_env(&["render", &scene, "--bench", "--frames", "0..1"], &[("SR_GPU_BACKEND", "gl")]);
    let refused = String::from_utf8_lossy(&o.stderr).contains("refused to create a device");
    if refused {
        eprintln!("skipping: the OpenGL adapter cannot create a device");
    }
    !refused
}

#[test]
fn a_3d_document_is_not_rendered_without_its_3d_on_an_opengl_adapter() {
    // asking for OpenGL only, a document with a 3D object cannot be drawn: that is an error by default,
    // not a frame without the object and exit 0
    if !has_gl_adapter() {
        eprintln!("skipping: no OpenGL adapter");
        return;
    }
    let dir = three_d_fixture("render-gl");
    let scene = dir.join("r.scene.xml").display().to_string();
    let o = run_env(&["render", &scene, "--bench", "--frames", "0..1"], &[("SR_GPU_BACKEND", "gl")]);
    let (out, err) = (String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.status.code(), Some(2), "{out}{err}");
    assert!(err.contains("3D") && err.contains("Vulkan") && err.contains("scene-render gpus"), "{err}");
}

#[test]
fn encoding_a_3d_document_on_an_opengl_adapter_fails_and_writes_nothing() {
    if !has_gl_adapter() {
        eprintln!("skipping: no OpenGL adapter");
        return;
    }
    let dir = three_d_fixture("encode-gl");
    let scene = dir.join("r.scene.xml").display().to_string();
    let pattern = dir.join("gl_%03d.png");
    let o = run_env(&["encode", &scene, "-o", pattern.to_str().unwrap(), "--end", "0.2"], &[("SR_GPU_BACKEND", "gl")]);
    let (out, err) = (String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    // the same exit code as render: scripts test one number for "this adapter cannot draw the document"
    assert_eq!(o.status.code(), Some(2), "{out}{err}");
    assert!(format!("{out}{err}").contains("3D"), "{out}{err}");
    assert!(!dir.join("gl_000.png").exists(), "no frame without the 3D object is written");
}

#[test]
fn a_flat_document_still_renders_on_opengl() {
    if !has_gl_adapter() {
        eprintln!("skipping: no OpenGL adapter");
        return;
    }
    let dir = render_fixture("flat-gl");
    let scene = dir.join("r.scene.xml").display().to_string();
    let o = run_env(&["render", &scene, "--bench", "--frames", "0..1"], &[("SR_GPU_BACKEND", "gl")]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn a_3d_document_takes_an_adapter_that_can_draw_it_by_default() {
    // on a machine that offers OpenGL only for the GPU and Vulkan for lavapipe (WSL2), the default
    // adapter would drop the 3D object; the choice now moves to the Vulkan one
    let o = run(&["gpus", "--json"]);
    let Ok(r) = serde_json::from_slice::<serde_json::Value>(&o.stdout) else { return };
    let list = r["adapters"].as_array().cloned().unwrap_or_default();
    if !list.iter().any(|a| a["backend"] != "Gl") {
        eprintln!("skipping: no non-OpenGL adapter");
        return;
    }
    let dir = three_d_fixture("default");
    let scene = dir.join("r.scene.xml").display().to_string();
    let o = run(&["render", &scene, "--bench", "--frames", "0..1"]);
    let (out, err) = (String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.status.code(), Some(0), "{out}{err}");
    assert!(!out.contains("3D objects are not drawn"), "{out}");
    assert!(!out.contains("(Gl,"), "the render adapter must not be an OpenGL one: {out}");
}

#[test]
fn encoding_a_3d_document_by_default_takes_an_adapter_that_can_draw_it() {
    // encode builds its own adapter, so it needs the same document-driven choice as render
    let o = run(&["gpus", "--json"]);
    let Ok(r) = serde_json::from_slice::<serde_json::Value>(&o.stdout) else { return };
    let list = r["adapters"].as_array().cloned().unwrap_or_default();
    if !list.iter().any(|a| a["backend"] != "Gl") {
        eprintln!("skipping: no non-OpenGL adapter");
        return;
    }
    let dir = three_d_fixture("encode-default");
    let scene = dir.join("r.scene.xml").display().to_string();
    let pattern = dir.join("d_%03d.png");
    let o = run(&["encode", &scene, "-o", pattern.to_str().unwrap(), "--end", "0.2"]);
    let (out, err) = (String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.status.code(), Some(0), "{out}{err}");
    assert!(
        err.contains("rendering on") && !err.contains("(Gl,"),
        "the encode adapter must not be an OpenGL one: {err}"
    );
    assert!(dir.join("d_000.png").is_file(), "{out}{err}");
}

fn safe_area_fixture(name: &str, enforce: &str) -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("sr-cli-safe-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let xml = format!(
        r##"<scene version="1.1"><project width="360" height="640" fps="10" duration="1" background="#000000" safeArea="sa"/>
<safeAreas><safeArea id="sa" preset="youtube-shorts" {enforce}/></safeAreas>
<composition><shape id="cta1" shape="rect" x="0" y="600" width="80" height="30" fill="#FF0000" tags="cta"/></composition></scene>"##
    );
    let file = dir.join("s.scene.xml");
    std::fs::write(&file, xml).unwrap();
    let f = file.display().to_string();
    (dir, f)
}

#[test]
fn validate_reports_a_cta_outside_the_safe_area_at_the_level_enforce_asks() {
    let (_, f) = safe_area_fixture("validate-error", r#"enforce="error""#);
    let o = run(&["validate", &f]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert_eq!(o.status.code(), Some(1), "{out}");
    assert!(out.contains("error[SA01]") && out.contains("cta1"), "{out}");

    let (_, f) = safe_area_fixture("validate-warn", r#"enforce="warn""#);
    let o = run(&["validate", &f]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert_eq!(o.status.code(), Some(0), "{out}");
    assert!(out.contains("warning[SA01]"), "{out}");
    assert_eq!(run(&["validate", "--deny-warnings", &f]).status.code(), Some(1));

    let (_, f) = safe_area_fixture("validate-off", r#"enforce="off""#);
    let out = String::from_utf8_lossy(&run(&["validate", &f]).stdout).to_string();
    assert!(!out.contains("SA01"), "{out}");
}

#[test]
fn render_and_encode_stop_on_enforce_error_and_say_so_on_warn() {
    let (dir, f) = safe_area_fixture("deliver-error", r#"enforce="error""#);
    let o = run(&["render", &f, "--bench", "--frames", "0..1"]);
    if no_gpu(&o) {
        return;
    }
    let all = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.status.code(), Some(1), "{all}");
    assert!(all.contains("SA01") && all.contains("cta1"), "{all}");
    let png = dir.join("e_%03d.png");
    let o = run(&["encode", &f, "-o", png.to_str().unwrap(), "--end", "0.2"]);
    let all = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.status.code(), Some(1), "{all}");
    assert!(all.contains("SA01"), "{all}");
    assert!(!dir.join("e_000.png").exists(), "an enforce=error failure writes nothing");

    let (dir, f) = safe_area_fixture("deliver-warn", r#"enforce="warn""#);
    let o = run(&["render", &f, "--bench", "--frames", "0..1"]);
    let all = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.status.code(), Some(0), "{all}");
    assert!(all.contains("SA01"), "{all}");
    let png = dir.join("w_%03d.png");
    let o = run(&["encode", &f, "-o", png.to_str().unwrap(), "--end", "0.2", "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let r: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("SA01")), "{r}");
    assert!(dir.join("w_000.png").is_file());
}

/// Writes a one-shape document and returns its path.
fn compile_fixture(name: &str, assets: &str, node: &str) -> String {
    let dir = std::env::temp_dir().join(format!("sr-cli-compile-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let xml = format!(
        r##"<scene version="1.1"><project width="64" height="64" fps="10" duration="1"/>{assets}<composition>{node}</composition></scene>"##
    );
    let file = dir.join("c.scene.xml");
    std::fs::write(&file, xml).unwrap();
    file.display().to_string()
}

#[test]
fn validate_fails_on_what_compilation_would_reject_at_render() {
    // a document validate accepts must not fail when render compiles it: each of these is a document the compile step rejects
    for (name, code, assets, node) in [
        (
            "scale-key",
            "E04",
            "",
            r##"<shape id="s" shape="rect" width="20" height="20" fill="#FF0000"><animate property="scale"><key time="0" value="1"/><key time="1" value="2"/></animate></shape>"##,
        ),
        (
            "text-expression",
            "E02",
            r##"<assets><text id="t" text="a" width="40" height="20" size="12"/></assets>"##,
            r##"<layer id="l" asset="t"><expression property="text">"hi"</expression></layer>"##,
        ),
        (
            "bare-floor",
            "E01",
            "",
            r##"<shape id="s" shape="rect" width="20" height="20" fill="#FF0000"><expression property="x">floor(time * 10)</expression></shape>"##,
        ),
    ] {
        let f = compile_fixture(name, assets, node);
        let v = run(&["validate", &f]);
        let out = String::from_utf8_lossy(&v.stdout);
        assert_eq!(v.status.code(), Some(1), "{name}: {out}");
        assert!(out.contains(&format!("error[{code}]")), "{name}: {out}");
    }
}

#[test]
fn a_document_that_compiles_still_validates() {
    let f = compile_fixture(
        "ok",
        "",
        r##"<shape id="s" shape="rect" width="20" height="20" fill="#FF0000"><animate property="scale"><key time="0" value="1,1"/><key time="1" value="2,2"/></animate><expression property="x">Math.floor(time * 10)</expression></shape>"##,
    );
    let v = run(&["validate", &f]);
    assert_eq!(v.status.code(), Some(0), "{}", String::from_utf8_lossy(&v.stdout));
}

fn forced_fixture(name: &str, attr: &str) -> String {
    let dir = std::env::temp_dir().join(format!("sr-cli-force-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let xml = format!(
        r##"<scene version="1.1"><project width="360" height="640" fps="10" duration="1" background="#000000" safeArea="sa"/>
<safeAreas><safeArea id="sa" preset="youtube-shorts" enforce="error"/></safeAreas>
<symbols><symbol id="notch" width="64" height="64" duration="1"><shape id="mark" shape="rect" width="64" height="64" fill="#FF8000" tags="logo"/></symbol></symbols>
<composition><instance id="n" symbol="notch" x="0" y="560" {attr}/></composition></scene>"##
    );
    let file = dir.join("f.scene.xml");
    std::fs::write(&file, xml).unwrap();
    file.display().to_string()
}

#[test]
fn a_forced_node_passes_enforce_error_and_validate_says_so() {
    let f = forced_fixture("without", "");
    let o = run(&["validate", &f]);
    assert_eq!(o.status.code(), Some(1), "{}", String::from_utf8_lossy(&o.stdout));

    let f = forced_fixture("with", r#"safeAreaForce="true""#);
    let o = run(&["validate", &f]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert_eq!(o.status.code(), Some(0), "{out}");
    assert!(out.contains("info[SA02]: n forced outside the safe area"), "{out}");
    assert!(!out.contains("SA01"), "{out}");
    // the force is information, not a warning: it does not fail --deny-warnings
    assert_eq!(run(&["validate", "--deny-warnings", &f]).status.code(), Some(0));
    let j = run(&["validate", "--format", "json", &f]);
    let v: serde_json::Value = serde_json::from_slice(&j.stdout).unwrap();
    assert!(v["files"][0]["info"][0].as_str().unwrap().contains("SA02"), "{v}");
}

#[test]
fn render_and_encode_accept_a_forced_node() {
    let f = forced_fixture("deliver", r#"safeAreaForce="true""#);
    let o = run(&["render", &f, "--bench", "--frames", "0..1"]);
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
}

#[test]
fn validate_warns_about_attributes_this_build_does_not_read() {
    let f = compile_fixture(
        "ignored",
        "",
        r##"<group id="g" collapse="true" width="10" height="10"><shape id="s" shape="rect" width="8" height="8" fill="#FF0000"/></group>"##,
    );
    let v = run(&["validate", &f]);
    let out = String::from_utf8_lossy(&v.stdout);
    assert_eq!(v.status.code(), Some(0), "{out}");
    // an attribute with no effect is information (SREP 18): shown, and not counted as a warning
    assert!(out.contains("info[E19]") && out.contains("collapse"), "{out}");
    assert!(out.contains("0 warning(s), 1 info"), "{out}");
    assert_eq!(
        run(&["validate", "--deny-warnings", &f]).status.code(),
        Some(0),
        "inert attributes pass --deny-warnings"
    );
    let j = run(&["validate", "--format", "json", &f]);
    let v: serde_json::Value = serde_json::from_slice(&j.stdout).unwrap();
    let d = &v["files"][0]["diagnostics"][0];
    assert!(d["code"] == "E19" && d["severity"] == "info", "{v}");
}

#[test]
fn a_finding_that_changes_the_result_still_fails_deny_warnings() {
    // a mask in canvas coordinates lies outside its node and the node vanishes: a warning, not information
    let f = compile_fixture(
        "mask-miss",
        "",
        r##"<shape id="m" shape="rect" x="500" y="300" width="300" height="400" fill="#FF0000"><mask type="rect" x="500" y="300" width="300" height="400" mode="add"/></shape>"##,
    );
    let v = run(&["validate", &f]);
    let out = String::from_utf8_lossy(&v.stdout);
    assert!(out.contains("warning[E20]"), "{out}");
    assert_eq!(run(&["validate", "--deny-warnings", &f]).status.code(), Some(1));
}

#[test]
fn encode_strict_does_not_count_information_but_counts_a_warning() {
    let inert = compile_fixture(
        "strict-inert",
        "",
        r##"<group id="g" collapse="true" width="10" height="10"><shape id="s" shape="rect" width="8" height="8" fill="#FF0000"/></group>"##,
    );
    let out = std::env::temp_dir().join(format!("sr-strict-info-{}.mkv", std::process::id()));
    let o = run(&["encode", &inert, "-o", out.to_str().unwrap(), "--end", "0.2", "--strict", "--codec", "ffv1"]);
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
    let miss = compile_fixture(
        "strict-warn",
        "",
        r##"<shape id="m" shape="rect" x="500" y="300" width="300" height="400" fill="#FF0000"><mask type="rect" x="500" y="300" width="300" height="400" mode="add"/></shape>"##,
    );
    let w = run(&["encode", &miss, "-o", out.to_str().unwrap(), "--end", "0.2", "--strict", "--codec", "ffv1"]);
    assert_eq!(w.status.code(), Some(1), "{}", String::from_utf8_lossy(&w.stderr));
    assert!(String::from_utf8_lossy(&w.stderr).contains("--strict"));
}

#[test]
fn encode_strict_accepts_posterize_time_on_a_node_that_starts_later() {
    // a node absent at its posterized step start is drawn as it was, which is nothing: not a delivery gap
    let f = compile_fixture(
        "pt-start",
        "",
        r##"<group id="m" x="0" y="20" width="20" height="20" effects="pt" start="0.2"><shape id="b" shape="rect" x="0" y="0" width="20" height="20" fill="#FFFFFF"/></group>"##,
    );
    let text = std::fs::read_to_string(&f)
        .unwrap()
        .replace("</scene>", r#"<effects><effect id="pt" type="posterize-time" frequency="8"/></effects></scene>"#);
    std::fs::write(&f, text).unwrap();
    let out = PathBuf::from(&f).with_file_name("pt_%03d.png");
    let o = run(&["encode", &f, "-o", out.to_str().unwrap(), "--end", "0.5", "--strict"]);
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
}

#[test]
fn validate_checks_every_layout_the_document_declares() {
    // a layout has its own frame size and safe area: a caption that is inside the project's safe area can cross the
    // layout's, and only render and encode used to say so
    let dir = std::env::temp_dir().join(format!("sr-cli-layouts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("l.scene.xml");
    std::fs::write(
        &file,
        r##"<scene version="1.1"><project width="640" height="360" fps="10" duration="1" background="#000000" safeArea="sa-wide"/>
<layouts><layout id="wide" width="640" height="360" safeArea="sa-wide"/><layout id="tall" width="360" height="640" safeArea="sa-tall"/></layouts>
<safeAreas><safeArea id="sa-wide" preset="title-safe" enforce="error"/><safeArea id="sa-tall" preset="youtube-shorts" enforce="error"/></safeAreas>
<composition/>
<captions><captionTrack id="cc" language="en" mode="burn"><cue start="0" end="1" text="Hello there"/></captionTrack></captions></scene>"##,
    )
    .unwrap();
    let f = file.display().to_string();
    let v = run(&["validate", &f]);
    let out = String::from_utf8_lossy(&v.stdout);
    assert_eq!(v.status.code(), Some(1), "{out}");
    assert!(out.contains("SA01") && out.contains("layout tall"), "{out}");
}
