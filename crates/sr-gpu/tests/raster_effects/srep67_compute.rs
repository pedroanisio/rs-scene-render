//! SREP 67, Semantics 1 to 3: the compute node, on the documents and shaders of sr-core's kit
//! (conformance/srep_cases/srep-0067.json; the shaders are copied into tests/shaders). A pixel is within 3 code values of
//! the kit's; the histogram itself is exact.

use super::common;
use common::*;

fn shaders() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/shaders")
}

fn doc(compute: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.6"><project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/><composition>{compute}</composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: Some(shaders()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}"))
}

/// The frame at 0 as 8-bit display sRGB, and the frame's problems.
fn frame(d: &sr_model::Document) -> Option<(Vec<u8>, Vec<String>)> {
    let gpu = gpu()?;
    let ev = sr_eval::Evaluator::new(d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = sr_gpu::Renderer::new(gpu, ev.program());
    let g = ev.evaluate(0.0);
    let mut sub = |t: f64| ev.evaluate(t);
    let f = r.render_with(&g, ev.program(), Some(&mut sub));
    let problems = f.stats.unsupported.iter().chain(&f.stats.errors).cloned().collect();
    Some((r.to_srgb8(&r.read(&f.texture)), problems))
}

fn px(img: &[u8], x: usize, y: usize) -> [f64; 3] {
    let k = (y * 640 + x) * 4;
    [img[k] as f64, img[k + 1] as f64, img[k + 2] as f64]
}

fn close(got: [f64; 3], want: [f64; 3], what: &str) {
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 3.0), "{what}: {got:?}, kit {want:?}");
}

#[test]
fn the_kit_tonemaps_render() {
    let three = |tm: &str, src: &str| {
        format!(
            r##"<compute id="c" src="{src}" width="400" height="200" invocations="1110"><tonemap ramp="#000000FF #FFFFFFFF" {tm}/></compute>"##
        )
    };
    for (what, xml, want) in [
        ("log-density", three("", "srep67-three.wgsl"), [255.0, 170.3, 88.5]),
        ("order", three("", "srep67-three-interleaved.wgsl"), [255.0, 170.3, 88.5]),
        ("reference", three(r#"reference="2000""#, "srep67-three.wgsl"), [231.8, 154.8, 80.4]),
        ("gain-gamma", three(r#"gain="0.01" gamma="2.2""#, "srep67-three.wgsl"), [255.0, 145.1, 58.9]),
        ("linear", three(r#"scale="linear""#, "srep67-three.wgsl"), [255.0, 25.5, 2.6]),
    ] {
        let Some((img, problems)) = frame(&doc(&xml)) else { return };
        assert!(problems.is_empty(), "{what}: {problems:?}");
        for (k, x) in [100, 200, 300].into_iter().enumerate() {
            close(px(&img, x, 100), [want[k]; 3], &format!("{what} at ({x}, 100)"));
        }
        // kit compute-empty-pixel: a pixel with no points is ramp(0), black
        close(px(&img, 150, 150), [0.0; 3], &format!("{what}: empty pixel"));
    }
}

#[test]
fn the_kit_colour_planes_render() {
    let xml = r##"<compute id="c" src="srep67-colour.wgsl" width="400" height="200" invocations="100" channels="4" fractionBits="8"><tonemap ramp="#000000FF #FFFFFFFF"/></compute>"##;
    let Some((img, problems)) = frame(&doc(xml)) else { return };
    assert!(problems.is_empty(), "{problems:?}");
    close(px(&img, 100, 100), [255.0, 0.0, 0.0], "red");
    close(px(&img, 200, 100), [0.0, 0.0, 255.0], "blue");
}

/// The 64-bit cells: amounts near 2^32 wrap the low word many times, in any order, and the pair is the exact sum.
#[test]
fn the_cells_are_exact_64_bit_sums_whatever_the_order() {
    let Some(gpu) = gpu() else { return };
    let src = "fn sr_point(i: u32) {\n  sr_accumulate(i32(i % 3u), 0, 0u, 0xFFFFFFFFu - (i % 7u));\n  sr_accumulate(5, 1, 0u, 1u);\n}";
    let program = sr_gpu::compute::compile(&gpu.device, src).unwrap();
    let job = sr_gpu::compute::Job {
        width: 8,
        height: 2,
        invocations: 100_000,
        channels: 1,
        fraction_bits: 0,
        frame: 0,
        time: 0.0,
        seed: 0,
        params: vec![],
    };
    let hist = sr_gpu::compute::run(&gpu.device, &gpu.queue, &program, &job).unwrap();
    for x in 0..3u64 {
        let want: u64 = (0..100_000u64).filter(|i| i % 3 == x).map(|i| 0xFFFF_FFFFu64 - (i % 7)).sum();
        assert_eq!(hist[x as usize], want, "cell ({x}, 0)");
    }
    assert_eq!(hist[8 + 5], 100_000, "cell (5, 1)");
    assert_eq!(hist.iter().sum::<u64>() - hist[0] - hist[1] - hist[2] - hist[13], 0, "nothing else");
    // a second run gives the same bytes
    assert_eq!(sr_gpu::compute::run(&gpu.device, &gpu.queue, &program, &job).unwrap(), hist);
}

#[test]
fn out_of_range_calls_add_nothing_and_params_and_seed_reach_the_shader() {
    let Some(gpu) = gpu() else { return };
    let src = "fn sr_point(i: u32) {\n  sr_accumulate(-1, 0, 0u, 1u);\n  sr_accumulate(4, 0, 0u, 1u);\n  sr_accumulate(0, 2, 0u, 1u);\n  sr_accumulate(0, 0, 1u, 1u);\n  sr_accumulate(0, 0, 0u, u32(sr_param(0u)));\n  sr_accumulate(1, 0, 0u, u32(sr_param(5u)));\n  sr_accumulate(2, 0, 0u, sr.seed_lo & 0xFFu);\n  sr_accumulate_f(3, 0, 0u, 1.5);\n}";
    let program = sr_gpu::compute::compile(&gpu.device, src).unwrap();
    let seed = 0x1234_5678_9ABC_DEF0u64;
    let job = sr_gpu::compute::Job {
        width: 4,
        height: 2,
        invocations: 3,
        channels: 1,
        fraction_bits: 2,
        frame: 0,
        time: 0.0,
        seed,
        params: vec![7.0],
    };
    let hist = sr_gpu::compute::run(&gpu.device, &gpu.queue, &program, &job).unwrap();
    // (0, 0): 7 per invocation; (1, 0): param 5 is absent (0); (2, 0): the seed's low byte; (3, 0): 1.5 * 4 = 6
    assert_eq!(hist, vec![21, 0, 3 * (seed as u32 & 0xFF) as u64, 18, 0, 0, 0, 0]);
}

#[test]
fn a_bad_source_is_cmp11_and_a_missing_one_cmp10() {
    let bad = doc(r#"<compute id="c" src="broken.glsl" width="4" height="4" invocations="1"/>"#);
    let Some((_, problems)) = frame(&bad) else { return };
    assert!(problems.iter().any(|p| p.contains("CMP11")), "{problems:?}");
    let missing = doc(r#"<compute id="c" src="nowhere.wgsl" width="4" height="4" invocations="1"/>"#);
    let (_, problems) = frame(&missing).unwrap();
    assert!(problems.iter().any(|p| p.contains("CMP10")), "{problems:?}");
    let wrong = doc(&format!(
        r#"<compute id="c" src="srep67-three.wgsl" sha256="{}" width="4" height="4" invocations="1"/>"#,
        "0".repeat(64)
    ));
    let (_, problems) = frame(&wrong).unwrap();
    assert!(problems.iter().any(|p| p.contains("CMP10") && p.contains("SHA-256")), "{problems:?}");
}
