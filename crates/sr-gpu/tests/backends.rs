//! Backend parity: the OpenGL backend (how WSL2 reaches its GPU, through Mesa's D3D12
//! driver) must draw what the native backend draws. Each test renders on the native
//! adapter and on a GL adapter and skips when the machine lacks one of them.

mod common;

use common::*;
use sr_gpu::gpu::GpuOptions;
use sr_gpu::Gpu;

fn on(backends: wgpu::Backends) -> Option<Gpu> {
    match Gpu::with_options(&GpuOptions { backends, adapter: None }) {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("skipping: no {backends:?} adapter ({e})");
            None
        }
    }
}

fn gl() -> Option<Gpu> {
    on(wgpu::Backends::GL)
}

fn native() -> Option<Gpu> {
    on(wgpu::Backends::PRIMARY)
}

/// Peak signal-to-noise ratio of two linear RGBA frames, in dB (colour and alpha, clamped to [0, 1]).
fn psnr(a: &Rendered, b: &Rendered) -> f64 {
    assert_eq!(a.size, b.size);
    let mut se = 0.0f64;
    for (p, q) in a.px.iter().zip(&b.px) {
        for c in 0..4 {
            let d = (p[c].clamp(0.0, 1.0) - q[c].clamp(0.0, 1.0)) as f64;
            se += d * d;
        }
    }
    let mse = se / (a.px.len() * 4) as f64;
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (1.0 / mse).log10()
    }
}

fn pattern_scene() -> sr_model::Document {
    doc(
        r##"width="64" height="64" background="#00000000""##,
        r#"<paints><pattern id="pat" asset="wide" tileWidth="16" tileHeight="8"/></paints>"#,
        r#"<shape id="c" shape="rect" x="0" y="0" width="64" height="64" fill="url(#pat)"/>"#,
    )
}

#[test]
fn gl_draws_pattern_paints() {
    // a pattern samples its tile with a repeating sampler, which GL cannot pair with the
    // clamping one on the same texture binding, so the texture is bound a second time for it
    let Some(g) = gl() else { return };
    let r = render_times_on(g, &pattern_scene(), &[0.0]).expect("GL renders");
    // the tile (left half red, right half blue) repeats every 16 px across and 8 px down
    let (red, blue) = ([1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]);
    for (x, y, want) in [(4, 4, red), (12, 4, blue), (20, 4, red), (28, 4, blue), (52, 60, red), (60, 60, blue)] {
        assert_px(&r, x, y, want, 0.05);
    }
}

#[test]
fn gl_matches_the_native_backend_on_pattern_paints() {
    let (Some(g), Some(n)) = (gl(), native()) else { return };
    let d = pattern_scene();
    let (a, b) = (render_times_on(g, &d, &[0.0]).unwrap(), render_times_on(n, &d, &[0.0]).unwrap());
    let db = psnr(&a, &b);
    assert!(db >= 40.0, "GL vs native: {db:.1} dB");
}

#[test]
fn gl_matches_the_native_backend_on_a_mixed_scene() {
    // particles, additive and screen blending, a gradient, blur and grade effects, an
    // adjustment layer and a feathered mask, animated so evaluation differs per frame
    let xml = format!(
        r##"<scene version="1.1"><project width="96" height="64" fps="10" duration="2" background="#101820"/>{ASSETS}
<paints><linearGradient id="g" x1="0" y1="0" x2="1" y2="0"><stop offset="0" color="#FF6A1A"/><stop offset="1" color="#1A6AFF"/></linearGradient></paints>
<composition>
<layer id="img" asset="wide" x="8" y="8" scaleX="4" scaleY="4" effects="soft"/>
<shape id="bar" shape="rect" x="0" y="40" width="96" height="16" fill="url(#g)" blend="screen">
  <mask type="ellipse" x="10" y="0" width="76" height="16" feather="4"/>
</shape>
<particleEmitter id="p" x="48" y="32" emitterShape="point" rate="0" lifetime="2" speed="30" spread="360"
                 size="4" color="#FFFFFF" shape="disc" blend="add" seed="7"><burst time="0" count="30"/></particleEmitter>
<shape id="dot" shape="ellipse" x="70" y="6" width="16" height="16" fill="#FFFFFF" blend="add">
  <animate property="x"><key time="0" value="70"/><key time="1" value="10"/></animate>
</shape>
<adjustment id="finish" effects="grade"/>
</composition>
<effects><effect id="soft" type="blur" radius="2"/><effect id="grade" type="color-grade" saturation="1.2" contrast="1.1"/></effects>
</scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    let d = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}"));
    let (Some(g), Some(n)) = (gl(), native()) else { return };
    let ts = [0.0, 0.3, 0.6];
    let (a, b) = (render_times_on(g, &d, &ts).unwrap(), render_times_on(n, &d, &ts).unwrap());
    let db = psnr(&a, &b);
    assert!(db >= 40.0, "GL vs native: {db:.1} dB");
}
