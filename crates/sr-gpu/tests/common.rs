//! Shared helpers: a GPU (or a skip), fixture images, and one-call rendering.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::OnceLock;

use sr_gpu::{Gpu, RenderStats, Renderer};

pub fn gpu() -> Option<Gpu> {
    static G: OnceLock<Option<Gpu>> = OnceLock::new();
    G.get_or_init(|| match Gpu::new() {
        Ok(g) => Some(g),
        Err(e) => {
            assert!(std::env::var("SR_REQUIRE_GPU").as_deref() != Ok("1"), "required GPU unavailable: {e}");
            eprintln!("skipping GPU tests: {e}");
            None
        }
    })
    .clone()
}

/// A directory with fixture images:
/// red.png 4×4 opaque (255,0,0); half.png 4×4 white at alpha 128;
/// quad.png 2×2 red/green/blue/white; wide.png 8×4 left half red, right half blue.
pub fn fixtures() -> PathBuf {
    static D: OnceLock<PathBuf> = OnceLock::new();
    D.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("sr-gpu-fixtures-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let save = |name: &str, w: u32, h: u32, f: &dyn Fn(u32, u32) -> [u8; 4]| {
            let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba(f(x, y)));
            img.save(dir.join(name)).unwrap();
        };
        save("red.png", 4, 4, &|_, _| [255, 0, 0, 255]);
        save("half.png", 4, 4, &|_, _| [255, 255, 255, 128]);
        save("quad.png", 2, 2, &|x, y| {
            [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 255, 255]][(y * 2 + x) as usize]
        });
        save("wide.png", 8, 4, &|x, _| if x < 4 { [255, 0, 0, 255] } else { [0, 0, 255, 255] });
        save("gray.png", 4, 4, &|_, _| [128, 128, 128, 255]);
        save("src.png", 4, 4, &|_, _| [200, 100, 50, 255]);
        save("white.png", 4, 4, &|_, _| [255, 255, 255, 255]);
        // equirect: grey, with a red patch at the centre column (+z) within ±17° of the horizon
        save("sky.png", 64, 32, &|x, y| {
            if (29..35).contains(&x) && (13..19).contains(&y) {
                [255, 0, 0, 255]
            } else {
                [128, 128, 128, 255]
            }
        });
        for k in 1..=3u8 {
            save(&format!("seq_{k:04}.png"), 2, 2, &|_, _| [k * 80, 0, 0, 255]);
        }
        dir
    })
    .clone()
}

pub const ASSETS: &str = r##"<assets>
  <image id="red" src="red.png" width="4" height="4"/>
  <image id="half" src="half.png" width="4" height="4"/>
  <image id="quad" src="quad.png" width="2" height="2"/>
  <image id="wide" src="wide.png" width="8" height="4"/>
  <image id="gray" src="gray.png" width="4" height="4"/>
  <image id="src" src="src.png" width="4" height="4"/>
  <image id="white" src="white.png" width="4" height="4"/>
  <imageSequence id="seq" src="seq_%04d.png" first="1" last="3" fps="10" width="2" height="2"/>
  <generator id="checker" kind="checkerboard" width="16" height="16" scale="4" paint="#FFFFFF" paint2="#000000"/>
  <generator id="noise" kind="fractal-noise" width="16" height="16" scale="4" seed="7"/>
  <generator id="solid" kind="solid" width="4" height="4" paint="#00FF00"/>
</assets>"##;

pub struct Rendered {
    pub px: Vec<[f32; 4]>,
    pub size: [u32; 2],
    pub stats: RenderStats,
    pub renderer: Renderer,
}

impl Rendered {
    pub fn at(&self, x: u32, y: u32) -> [f32; 4] {
        self.px[(y * self.size[0] + x) as usize]
    }
}

/// Document with `project` attributes, extra top-level sections and composition body.
pub fn doc(project: &str, extra: &str, body: &str) -> sr_model::Document {
    let w = if project.contains("width=") { "" } else { r#"width="64""# };
    let h = if project.contains("height=") { "" } else { r#"height="32""# };
    let xml = format!(
        r#"<scene version="1.1"><project {w} {h} fps="10" duration="2" {project}/>{ASSETS}{extra}<composition>{body}</composition></scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    match sr_model::load_str(&xml, &opts) {
        Ok(d) => d,
        Err(e) => panic!("{e:?}\n{xml}"),
    }
}

/// Renders the document at times `ts` with one renderer; returns the last frame.
pub fn render_times(d: &sr_model::Document, ts: &[f64]) -> Option<Rendered> {
    render_times_on(gpu()?, d, ts)
}

/// Renders the document at times `ts` on `gpu`; returns the last frame.
pub fn render_times_on(gpu: Gpu, d: &sr_model::Document, ts: &[f64]) -> Option<Rendered> {
    let ev = sr_eval::Evaluator::new(d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = Renderer::new(gpu, ev.program());
    let mut last = None;
    for &t in ts {
        let g = ev.evaluate(t);
        let f = r.render(&g, ev.program());
        last = Some((r.read(&f.texture), f.stats, f.texture.size));
    }
    let (px, stats, size) = last?;
    Some(Rendered { px, size, stats, renderer: r })
}

pub fn render(d: &sr_model::Document) -> Option<Rendered> {
    render_times(d, &[0.0])
}

pub fn close(a: [f32; 4], b: [f32; 4], tol: f32) -> bool {
    (0..4).all(|i| (a[i] - b[i]).abs() <= tol)
}

#[track_caller]
pub fn assert_px(r: &Rendered, x: u32, y: u32, want: [f32; 4], tol: f32) {
    let got = r.at(x, y);
    assert!(close(got, want, tol), "pixel ({x},{y}) = {got:?}, want {want:?} ±{tol}");
}

pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear value of an 8-bit sRGB channel.
pub fn lin8(v: u8) -> f32 {
    srgb_to_linear(v as f32 / 255.0)
}

/// A document with sections before the composition (`pre`) and after it (`post`: lights, effects).
pub fn doc_with(project: &str, pre: &str, body: &str, post: &str) -> sr_model::Document {
    let xml = format!(
        r#"<scene version="1.1"><project width="64" height="32" fps="10" duration="4" {project}/>{pre}{ASSETS}<composition>{body}</composition>{post}</scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    match sr_model::load_str(&xml, &opts) {
        Ok(d) => d,
        Err(e) => panic!("{e:?}\n{xml}"),
    }
}

/// Renders each of `ts` in turn with one renderer (so caches carry over) and a sub-frame
/// provider; returns every frame.
pub fn render_sub_frames(d: &sr_model::Document, ts: &[f64]) -> Option<Vec<Shot>> {
    let gpu = gpu()?;
    let ev = sr_eval::Evaluator::new(d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = Renderer::new(gpu, ev.program());
    let mut out = Vec::new();
    for &t in ts {
        let g = ev.evaluate(t);
        let mut sub = |st: f64| ev.evaluate(st);
        let f = r.render_with(&g, ev.program(), Some(&mut sub));
        out.push(Shot { px: r.read(&f.texture), size: f.texture.size, stats: f.stats });
    }
    Some(out)
}

/// One rendered frame without its renderer.
pub struct Shot {
    pub px: Vec<[f32; 4]>,
    pub size: [u32; 2],
    pub stats: RenderStats,
}

impl Shot {
    pub fn at(&self, x: u32, y: u32) -> [f32; 4] {
        self.px[(y * self.size[0] + x) as usize]
    }
}

/// Peak signal-to-noise ratio of two frames' linear RGBA, clamped to [0, 1], in dB.
pub fn psnr(a: &[[f32; 4]], b: &[[f32; 4]]) -> f64 {
    assert_eq!(a.len(), b.len());
    let se: f64 = a
        .iter()
        .zip(b)
        .map(|(p, q)| (0..4).map(|c| ((p[c].clamp(0.0, 1.0) - q[c].clamp(0.0, 1.0)) as f64).powi(2)).sum::<f64>())
        .sum();
    let mse = se / (a.len() * 4) as f64;
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (1.0 / mse).log10()
    }
}

/// Renders at `t` with a sub-frame provider (motion blur and time effects).
pub fn render_sub(d: &sr_model::Document, t: f64) -> Option<Rendered> {
    let gpu = gpu()?;
    let ev = sr_eval::Evaluator::new(d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = Renderer::new(gpu, ev.program());
    let g = ev.evaluate(t);
    let mut sub = |st: f64| ev.evaluate(st);
    let f = r.render_with(&g, ev.program(), Some(&mut sub));
    let px = r.read(&f.texture);
    Some(Rendered { px, size: f.texture.size, stats: f.stats, renderer: r })
}
