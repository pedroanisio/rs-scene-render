//! Custom GLSL shaders: the conventions, uniform rules, colour handling and fallbacks of the
//! Python engine (its fixtures live in tests/shaders).

mod common;
use common::*;

fn sh(name: &str) -> String {
    format!("{}/tests/shaders/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn fx_doc(body: &str, effects: &str) -> sr_model::Document {
    doc_with(r##"background="#00000000""##, "", body, &format!("<effects>{effects}</effects>"))
}

fn problems(r: &Rendered) -> Vec<String> {
    r.stats.unsupported.iter().chain(&r.stats.errors).cloned().collect()
}

/// A red 16×16 square at (8, 8) with effect `id`.
fn square(id: &str) -> String {
    format!(r#"<layer id="a" asset="red" x="8" y="8" scaleX="4" scaleY="4" effects="{id}"/>"#)
}

/// Renders `ts` in order the way the CLI does (with a provider for other times).
fn render_seq(d: &sr_model::Document, ts: &[f64]) -> Option<Rendered> {
    let gpu = gpu()?;
    let ev = sr_eval::Evaluator::new(d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = sr_gpu::Renderer::new(gpu, ev.program());
    let mut last = None;
    for &t in ts {
        let g = ev.evaluate(t);
        let mut sub = |t: f64| ev.evaluate(t);
        let f = r.render_with(&g, ev.program(), Some(&mut sub));
        last = Some((r.read(&f.texture), f.stats, f.texture.size));
    }
    let (px, stats, size) = last?;
    Some(Rendered { px, size, stats, renderer: r })
}

fn transition_doc(extra: &str) -> sr_model::Document {
    let body = format!(
        r#"<layer id="a" asset="red" x="0" y="0" scaleX="16" scaleY="8" end="2"/>
           <layer id="b" asset="wide" x="0" y="0" scaleX="8" scaleY="8" start="2"/>
           <transition type="shader" from="a" to="b" duration="1" curve="linear" motionBlur="false" {extra}/>"#
    );
    doc_with("", "", &body, "")
}

#[test]
fn invert_in_both_conventions() {
    for file in ["invert.glsl", "invert_shadertoy.glsl"] {
        let d = fx_doc(&square("i"), &format!(r#"<effect id="i" type="shader" src="{}"/>"#, sh(file)));
        let Some(r) = render(&d) else { return };
        assert!(problems(&r).is_empty(), "{file}: {:?}", problems(&r));
        assert_px(&r, 16, 16, [0.0, 1.0, 1.0, 1.0], 1e-2);
        assert_px(&r, 2, 2, [0.0, 0.0, 0.0, 0.0], 1e-3);
    }
}

#[test]
fn params_attributes_and_defaults_feed_uniforms() {
    // invert's `amount` default is 1; a param overrides it
    let d = fx_doc(
        &square("i"),
        &format!(
            r#"<effect id="i" type="shader" src="{}"><param name="amount" value="0"/></effect>"#,
            sh("invert.glsl")
        ),
    );
    let Some(r) = render(&d) else { return };
    assert_px(&r, 16, 16, [1.0, 0.0, 0.0, 1.0], 1e-2);
    // an attribute of the effect reaches a uniform of the same name (color, straight sRGB)
    std::fs::write(
        fixtures().join("tint.glsl"),
        "uniform vec4 color;\nvoid main() { vec4 c = texture(inputTexture, uv); fragColor = vec4(color.rgb, c.a); }\n",
    )
    .unwrap();
    let d = fx_doc(&square("t"), r##"<effect id="t" type="shader" src="tint.glsl" color="#00FF00"/>"##);
    let r = render(&d).unwrap();
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    assert_px(&r, 16, 16, [0.0, 1.0, 0.0, 1.0], 1e-2);
}

#[test]
fn shadertoy_time_is_deterministic_and_animates() {
    let d = fx_doc(
        &square("r"),
        &format!(
            r#"<effect id="r" type="shader" src="{}"><param name="intensity" value="1"/></effect>"#,
            sh("rainbow_shadertoy.glsl")
        ),
    );
    let Some(a) = render_times(&d, &[0.5]) else { return };
    let b = render_times(&d, &[0.5]).unwrap();
    let c = render_times(&d, &[1.5]).unwrap();
    assert!(problems(&a).is_empty(), "{:?}", problems(&a));
    assert_eq!(a.at(16, 16), b.at(16, 16));
    assert_ne!(a.at(16, 16), c.at(16, 16));
}

#[test]
fn padding_grows_the_tile_so_glows_spill() {
    let fx = format!(
        r##"<effect id="g" type="shader" src="{}" radius="3" color="#00FF00"><param name="padding" value="6"/></effect>"##,
        sh("glow.glsl")
    );
    let d = fx_doc(&square("g"), &fx);
    let Some(r) = render(&d) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    assert!(r.at(6, 16)[3] > 0.2 && r.at(6, 16)[1] > 0.1, "glow spills left of the square: {:?}", r.at(6, 16));
    assert_px(&r, 16, 16, [1.0, 0.0, 0.0, 1.0], 2e-2);
}

#[test]
fn isf_multipass_with_defaults() {
    let d = fx_doc(&square("d"), &format!(r#"<effect id="d" type="shader" src="{}"/>"#, sh("duotone.fs")));
    let Some(r) = render(&d) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    // red's luma (0.2126 in encoded sRGB terms of pure red → l = 0.2126) maps between dark and light
    let l = 0.2126f32;
    let enc = |d: f32, lt: f32| srgb_to_linear(d + (lt - d) * l);
    let want = [enc(0.1, 1.0), enc(0.0, 0.8), enc(0.3, 0.2), 1.0];
    assert_px(&r, 16, 16, want, 3e-2);
}

#[test]
fn data_uris_and_compile_errors() {
    let code = "void main() { vec4 c = texture(inputTexture, uv); fragColor = vec4(c.bgr, c.a); }";
    let uri = format!("data:,{}", code.replace(' ', "%20").replace('{', "%7B").replace('}', "%7D"));
    let d = fx_doc(&square("u"), &format!(r#"<effect id="u" type="shader" src="{uri}"/>"#));
    let Some(r) = render(&d) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    assert_px(&r, 16, 16, [0.0, 0.0, 1.0, 1.0], 1e-2);
    // a broken shader passes its input through and reports the user's identifiers
    let d = fx_doc(&square("b"), &format!(r#"<effect id="b" type="shader" src="{}"/>"#, sh("broken.glsl")));
    let r = render(&d).unwrap();
    assert_px(&r, 16, 16, [1.0, 0.0, 0.0, 1.0], 1e-2);
    let p = problems(&r).join("\n");
    assert!(p.contains("undefinedThing") || p.contains("notDeclared"), "{p}");
    // (a missing @src file is rejected when the document loads: asset check A01)
}

#[test]
fn identity_round_trips_every_space() {
    std::fs::write(fixtures().join("id.glsl"), "void main() { fragColor = texture(inputTexture, uv); }\n").unwrap();
    for space in [
        "srgb",
        "linear-srgb",
        "rec709",
        "display-p3",
        "dci-p3",
        "rec2020",
        "acescg",
        "aces2065-1",
        "acescct",
        "xyz-d65",
        "raw",
    ] {
        let body = r#"<layer id="a" asset="src" x="8" y="8" scaleX="4" scaleY="4" effects="i"/>"#;
        let d = fx_doc(body, &format!(r#"<effect id="i" type="shader" src="id.glsl" space="{space}"/>"#));
        let Some(r) = render(&d) else { return };
        assert!(problems(&r).is_empty(), "{space}: {:?}", problems(&r));
        assert_px(&r, 16, 16, [lin8(200), lin8(100), lin8(50), 1.0], 1.5e-2);
    }
}

#[test]
fn source_channel_and_named_samplers() {
    std::fs::write(
        fixtures().join("two.glsl"),
        "uniform sampler2D extra;\nvoid main() { vec4 a = texture(inputTexture, uv); vec4 b = texture(sourceTexture, uv); vec4 e = texture(extra, uv); fragColor = vec4(b.r, e.g, a.b, a.a); }\n",
    )
    .unwrap();
    let body = r#"<layer id="w" asset="white" x="8" y="8" scaleX="4" scaleY="4" visible="false"/>
        <layer id="a" asset="red" x="8" y="8" scaleX="4" scaleY="4" effects="s"/>"#;
    let d = fx_doc(
        body,
        r#"<effect id="s" type="shader" src="two.glsl" source="w"><param name="extra" value="white"/></effect>"#,
    );
    let Some(r) = render(&d) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    assert_px(&r, 16, 16, [1.0, 1.0, 0.0, 1.0], 2e-2);
}

#[test]
fn gl_transitions_start_and_end_on_their_inputs() {
    for file in ["fade.glsl", "circleopen.glsl", "directionalwipe.glsl", "doorway.glsl"] {
        let d = transition_doc(&format!(r#"shader="{}""#, sh(file)));
        let Some(start) = render_times(&d, &[1.5005]) else { return };
        let end = render_times(&d, &[2.4995]).unwrap();
        assert!(problems(&start).is_empty(), "{file}: {:?}", problems(&start));
        assert_px(&start, 10, 10, [1.0, 0.0, 0.0, 1.0], 4e-2);
        assert!(close(end.at(52, 10), [0.0, 0.0, 1.0, 1.0], 5e-2), "{file} end: {:?}", end.at(52, 10));
    }
}

#[test]
fn transition_uniforms_direction_and_fallback() {
    // directionalwipe's vec2 direction comes from @direction (a unit motion vector, +y up)
    let d = transition_doc(&format!(r#"shader="{}" direction="right""#, sh("directionalwipe.glsl")));
    let Some(r) = render_times(&d, &[2.0]) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    // moving right: the incoming side (blue at x ≥ 32) has reached the left half, not the right edge
    assert!(r.at(4, 16)[2] > 0.5 || r.at(60, 16)[0] > 0.5, "{:?} {:?}", r.at(4, 16), r.at(60, 16));
    // a broken transition shader crossfades
    let d = transition_doc(&format!(r#"shader="{}""#, sh("broken.glsl")));
    let r = render_times(&d, &[2.0]).unwrap();
    assert_px(&r, 52, 10, [0.5, 0.0, 0.5, 1.0], 3e-2);
    assert!(problems(&r).join(" ").contains("notDeclared"), "{:?}", problems(&r));
}

#[test]
fn isf_persistent_feedback_accumulates_frame_to_frame() {
    std::fs::write(
        fixtures().join("acc.fs"),
        r#"/*{ "INPUTS": [{"NAME": "inputImage", "TYPE": "image"}],
            "PASSES": [{"TARGET": "acc", "PERSISTENT": true}, {}] }*/
void main() {
    if (PASSINDEX == 0) {
        vec4 prev = IMG_NORM_PIXEL(acc, isf_FragNormCoord);
        gl_FragColor = vec4(min(prev.r + 0.25, 1.0), 0.0, 0.0, 1.0);
    } else {
        gl_FragColor = IMG_NORM_PIXEL(acc, isf_FragNormCoord);
    }
}
"#,
    )
    .unwrap();
    let d = fx_doc(&square("f"), r#"<effect id="f" type="shader" src="acc.fs" space="raw"/>"#);
    let Some(one) = render_seq(&d, &[0.0]) else { return };
    let three = render_seq(&d, &[0.0, 0.1, 0.2]).unwrap();
    assert!(problems(&three).is_empty(), "{:?}", problems(&three));
    let at = |r: &Rendered| r.at(16, 16)[0];
    assert!((at(&one) - 0.25).abs() < 1e-2, "{:?}", one.at(16, 16));
    assert!((at(&three) - 0.75).abs() < 1e-2, "{:?}", three.at(16, 16));
    // a seek replays the frames before it: frame 2 alone equals frames 0, 1, 2 in order
    let direct = render_seq(&d, &[0.2]).unwrap();
    assert!((at(&direct) - at(&three)).abs() < 1e-3, "{} vs {}", at(&direct), at(&three));
    // seeking back, and rendering one frame twice, do not advance the history
    let back = render_seq(&d, &[0.3, 0.1]).unwrap();
    assert!((at(&back) - 0.5).abs() < 1e-2, "{:?}", back.at(16, 16));
    let twice = render_seq(&d, &[0.0, 0.1, 0.1]).unwrap();
    assert!((at(&twice) - 0.5).abs() < 1e-2, "{:?}", twice.at(16, 16));
}

#[test]
fn isf_audio_textures() {
    use sr_gpu::shader::audio_texture;
    let rate = 48000.0;
    let n = 48000;
    // a full-scale sine centred on FFT bin 100 of the 2048-sample window reads 1 in that bin
    let f = 100.0 * rate / 2048.0;
    let sine: Vec<f32> = (0..n).map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / rate).sin() as f32).collect();
    let (w, h, px) = audio_texture(&[sine.clone(), vec![0.0; n]], rate, 0.5, true, None);
    assert_eq!((w, h), (1025, 2));
    assert!((px[100][0] - 1.0).abs() < 0.01, "{}", px[100][0]);
    assert!(px[1025 + 100][0] < 1e-3, "second channel is silent");
    // waveforms centre on 0.5; one column holds the RMS
    let (w1, _, rms) = audio_texture(&[sine], rate, 0.5, false, Some(1));
    assert_eq!(w1, 1);
    assert!((rms[0][0] - (0.5 + 0.5 / 2f32.sqrt())).abs() < 0.01, "{}", rms[0][0]);
    let (_, _, silent) = audio_texture(&[], rate, 0.5, false, Some(8));
    assert!(silent.iter().all(|p| (p[0] - 0.5).abs() < 1e-6));
}

/// Every gl-transitions shader compiles (set SR_GL_TRANSITIONS to a folder of .glsl files).
#[test]
fn gl_transitions_corpus_compiles() {
    let Ok(dir) = std::env::var("SR_GL_TRANSITIONS") else { return };
    let mut bad = Vec::new();
    let mut n = 0;
    for f in std::fs::read_dir(dir).unwrap().flatten() {
        let code = std::fs::read_to_string(f.path()).unwrap();
        n += 1;
        let r = sr_gpu::glsl::build_transition(&code).and_then(|p| sr_gpu::fx::check_glsl(&p.glsl));
        if let Err(e) = r {
            bad.push(format!("{}: {}", f.path().display(), e.lines().take(3).collect::<Vec<_>>().join(" | ")));
        }
    }
    assert!(bad.is_empty(), "{}/{n} fail:\n{}", bad.len(), bad.join("\n"));
}
