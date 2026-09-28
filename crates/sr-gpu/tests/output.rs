//! Delivery conversion: YUV codes, depth, range, letterboxing and float output.

mod common;
use common::*;

use sr_gpu::output::{OutputColor, OutputStage};
use sr_media::encode::InputFormat;
use sr_model::model::{ColorSpace, Transfer};

fn frame_of(background: &str) -> Option<(sr_gpu::Renderer, sr_gpu::Frame)> {
    let d = doc(&format!(r#"width="8" height="4" background="{background}""#), "", "");
    let gpu = gpu()?;
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let mut r = sr_gpu::Renderer::new(gpu, ev.program());
    let f = r.render(&ev.evaluate(0.0), ev.program());
    Some((r, f))
}

#[test]
fn nv12_and_p010_codes_follow_bt709_limited_range() {
    let Some((r, f)) = frame_of("#FF0000") else { return };
    let gpu = r.gpu().clone();
    let mut st = OutputStage::new(gpu.device.clone(), gpu.queue.clone());
    let out = OutputColor::new(ColorSpace::Rec709, Transfer::Auto, false);
    let p = st.submit(&f.texture, &r.working(), &out, InputFormat::Nv12, [8, 4], false);
    let b = st.wait(p);
    assert_eq!(b.len(), 8 * 4 + 4 * 2 * 2);
    // Y = 16 + 219·Kr = 62.56, dithered between 62 and 63 around that mean
    assert!(b[..32].iter().all(|&y| y == 62 || y == 63), "{:?}", &b[..8]);
    let mean = b[..32].iter().map(|&v| v as f64).sum::<f64>() / 32.0;
    assert!((mean - 62.56).abs() < 0.35, "{mean}");
    // Cb = 128 − 224·Kr/(2(1 − Kb)) = 102.3, Cr = 240
    assert!((101..=103).contains(&b[32]) && (239..=240).contains(&b[33]), "{:?}", &b[32..34]);
    let p = st.submit(&f.texture, &r.working(), &out, InputFormat::P010, [8, 4], false);
    let b = st.wait(p);
    let y = u16::from_le_bytes([b[0], b[1]]) >> 6;
    let cr = u16::from_le_bytes([b[64 + 2], b[64 + 3]]) >> 6;
    assert_eq!((y, cr), (250, 960));
    let full = OutputColor::new(ColorSpace::Rec709, Transfer::Auto, true);
    let p = st.submit(&f.texture, &r.working(), &full, InputFormat::Nv12, [8, 4], false);
    let y = st.wait(p)[0];
    assert!((54..=55).contains(&y), "full-range Y = 255·Kr = 54.2: {y}");
}

#[test]
fn rgba_letterboxing_and_float_planes() {
    let Some((r, f)) = frame_of("#808080") else { return };
    let gpu = r.gpu().clone();
    let mut st = OutputStage::new(gpu.device.clone(), gpu.queue.clone());
    let srgb = OutputColor::new(ColorSpace::Srgb, Transfer::Auto, true);
    // 8×4 frame into 16×4: 4-pixel black bars left and right
    let p = st.submit(&f.texture, &r.working(), &srgb, InputFormat::Rgba8, [16, 4], true);
    let b = st.wait(p);
    let px = |x: usize| &b[x * 4..x * 4 + 4];
    assert_eq!(px(0), &[0, 0, 0, 0]);
    assert!(px(8)[..3].iter().all(|&v| (127..=129).contains(&v)) && px(8)[3] == 255, "{:?}", px(8));
    let lin = OutputColor::new(ColorSpace::LinearSrgb, Transfer::Linear, true);
    let p = st.submit(&f.texture, &r.working(), &lin, InputFormat::Gbrapf32, [8, 4], true);
    let b = st.wait(p);
    let g0 = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let a0 = f32::from_le_bytes(b[3 * 128..3 * 128 + 4].try_into().unwrap());
    assert!((g0 - lin8(128)).abs() < 2e-3 && (a0 - 1.0).abs() < 1e-6, "{g0} {a0}");
    let p = st.submit(&f.texture, &r.working(), &srgb, InputFormat::Rgba16, [8, 4], false);
    let b = st.wait(p);
    let v = u16::from_le_bytes([b[0], b[1]]);
    assert!((v as i32 - 32896).abs() < 300, "{v}");
}

#[test]
fn pq_output_places_reference_white_at_203_nits() {
    let Some((r, f)) = frame_of("#FFFFFF") else { return };
    let gpu = r.gpu().clone();
    let mut st = OutputStage::new(gpu.device.clone(), gpu.queue.clone());
    let hdr = OutputColor::new(ColorSpace::Rec2020, Transfer::Pq, false);
    let p = st.submit(&f.texture, &r.working(), &hdr, InputFormat::P010, [8, 4], false);
    let b = st.wait(p);
    let y = (u16::from_le_bytes([b[0], b[1]]) >> 6) as f64;
    // PQ(203 cd/m²) = 0.5807; 10-bit limited code 64 + 876·0.5807 = 572.7
    assert!((y - 572.7).abs() < 1.5, "{y}");
}
