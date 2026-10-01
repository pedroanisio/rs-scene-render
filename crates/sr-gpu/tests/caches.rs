//! Caches across frames: an offscreen is reused only while everything it shows is unchanged
//! (the paints its content fills with, the nodes its effects read), and decoded pictures are
//! kept only while they are in use.

mod common;

use common::*;

fn scene(pre: &str, body: &str, post: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.1"><project width="64" height="32" fps="10" duration="2" background="#00000000"/>{ASSETS}{pre}<composition>{body}</composition>{post}</scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

const GRADIENT: &str = r##"<paints><linearGradient id="g" x1="0" y1="0" x2="1" y2="0">
  <stop offset="0" color="#FF0000"><animate property="color"><key time="0" value="#FF0000"/><key time="1" value="#0000FF"/></animate></stop>
  <stop offset="1" color="#FFFFFF"><animate property="color"><key time="0" value="#FFFFFF"/><key time="1" value="#000000"/></animate></stop></linearGradient></paints>"##;

/// Frames at 0 and 1 s from one renderer: the second must be what a new renderer draws at 1 s.
#[track_caller]
fn assert_follows(d: &sr_model::Document, what: &str) {
    let Some(f) = render_sub_frames(d, &[0.0, 1.0]) else { return };
    let fresh = render_sub(d, 1.0).unwrap();
    assert!(f[0].stats.unsupported.is_empty(), "{what}: {:?}", f[0].stats.unsupported);
    assert!(f[0].px != fresh.px, "{what}: the frame must change between 0 and 1 s");
    assert!(f[1].px == fresh.px, "{what}: the second frame shows the first frame's content");
}

#[test]
fn isolated_groups_follow_the_paints_of_their_content() {
    for attrs in [r#"isolate="true""#, r#"blend="screen""#, r#"width="40" height="24" clip="true""#, r#"matte="m""#, ""]
    {
        let mask = if attrs.is_empty() { r#"<mask type="ellipse" x="0" y="0" width="64" height="32"/>"# } else { "" };
        let d = scene(
            GRADIENT,
            &format!(
                r##"<layer id="m" asset="white" x="0" y="0" scaleX="16" scaleY="8" visible="false"/>
                <group id="grp" {attrs}>{mask}<shape id="s" shape="rect" x="4" y="4" width="40" height="20" fill="url(#g)"/></group>"##
            ),
            "",
        );
        assert_follows(&d, if attrs.is_empty() { "masked" } else { attrs });
    }
}

#[test]
fn mattes_follow_the_paints_they_are_filled_with() {
    for mode in ["luma", "alpha"] {
        // the matte's gradient goes from red over white to blue over black (luma), and, for
        // alpha, fades out
        let paints = if mode == "luma" {
            GRADIENT.to_string()
        } else {
            GRADIENT.replace(r##"<key time="1" value="#0000FF"/>"##, r##"<key time="1" value="#0000FF00"/>"##)
        };
        let d = scene(
            &paints,
            &format!(
                r##"<shape id="m" shape="rect" x="0" y="0" width="64" height="32" fill="url(#g)" visible="false"/>
                <layer id="l" asset="white" x="8" y="4" scaleX="12" scaleY="6" matte="m" matteMode="{mode}"/>"##
            ),
            "",
        );
        assert_follows(&d, mode);
    }
}

#[test]
fn effects_follow_the_nodes_they_read() {
    // a still layer displaced by, keyed against, or sampling in a shader, a node that moves
    let moving = r#"<layer id="bg" asset="wide" x="0" y="0" scaleX="4" scaleY="4" visible="false"><animate property="x"><key time="0" value="0"/><key time="1" value="24"/></animate></layer>"#;
    std::fs::write(
        fixtures().join("other.glsl"),
        "uniform sampler2D other;\nvoid main() { fragColor = texture(other, uv); }\n",
    )
    .unwrap();
    for effect in [
        r#"<effect id="fx" type="displacement-map" source="bg" amount="6"/>"#,
        r#"<effect id="fx" type="difference-key" source="bg" tolerance="0.1"/>"#,
        r#"<effect id="fx" type="shader" src="other.glsl"><param name="other" value="bg"/></effect>"#,
    ] {
        let d = scene(
            "",
            &format!(r#"{moving}<layer id="l" asset="quad" x="8" y="4" scaleX="12" scaleY="12" effects="fx"/>"#),
            &format!("<effects>{effect}</effects>"),
        );
        assert_follows(&d, effect);
    }
}

/// Textures the renderer holds after each of the frames at `ts`, rendered in turn.
fn held(d: &sr_model::Document, ts: &[f64]) -> Option<Vec<usize>> {
    let ev = sr_eval::Evaluator::new(d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = sr_gpu::Renderer::new(gpu()?, ev.program());
    let mut out = Vec::new();
    for &t in ts {
        let f = r.render(&ev.evaluate(t), ev.program());
        assert!(f.stats.errors.is_empty() && f.stats.unsupported.is_empty(), "{:?}", f.stats);
        out.push(r.cached_textures());
    }
    Some(out)
}

#[test]
fn image_sequences_keep_only_the_frames_in_use() {
    let dir = fixtures();
    for k in 0..12u8 {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([k * 20, 0, 0, 255]));
        img.save(dir.join(format!("long_{k:04}.png"))).unwrap();
    }
    let xml = r##"<scene version="1.1"><project width="64" height="32" fps="10" duration="2"/>
      <assets><imageSequence id="long" src="long_%04d.png" first="0" last="11" fps="10" width="2" height="2"/>
        <image id="still" src="red.png" width="4" height="4"/></assets>
      <composition><layer id="s" asset="long" scaleX="8" scaleY="8"/><layer id="i" asset="still" x="32"/></composition></scene>"##;
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir) };
    let d = sr_model::load_str(xml, &opts).unwrap_or_else(|e| panic!("{e:?}"));
    let ts: Vec<f64> = (0..12).map(|k| k as f64 / 10.0 + 0.01).collect();
    let Some(n) = held(&d, &ts) else { return };
    assert_eq!(n[0], 2, "the still and the first frame");
    // a frame shown is kept for a few frames, not for the rest of the render
    assert!(n[11] <= 6, "{n:?}: every frame shown is still held");
    assert!(n[11] >= 2, "{n:?}: the still stays");
}

#[test]
fn raster_map_tiles_are_kept_only_while_shown() {
    let tiles = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sr-geo/tests/fixtures/raster.pmtiles");
    // zoom 0 shows the tiles of z1, zoom 1 those of z2
    let xml = format!(
        r##"<scene version="1.2"><project width="256" height="256" fps="10" duration="2" background="#000000"/>
        <assets><tiles id="t" src="{}"/>
          <map id="m" width="256" height="256" projection="web-mercator" centerLon="0" centerLat="0" zoom="0">
            <basemap tiles="t" attribution="false"/>
            <animate property="zoom"><key time="0" value="0"/><key time="0.05" value="0"/><key time="0.1" value="1"/></animate></map></assets>
        <composition><layer id="l" asset="m"/></composition></scene>"##,
        tiles.display()
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    let d = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    let ts: Vec<f64> = (0..8).map(|k| k as f64 / 10.0 + 0.01).collect();
    let (Some(n), Some(alone)) = (held(&d, &ts), held(&d, &[0.5])) else { return };
    assert!(n[0] > 0 && alone[0] > 0, "{n:?}, {alone:?}: the map draws tile pictures");
    assert!(n[1] > alone[0], "{n:?}: both zooms' tiles are held when the zoom changes");
    assert_eq!(n[7], alone[0], "{n:?}: the first zoom's tiles are still held");
}

#[test]
fn an_evolving_generator_does_not_fill_the_texture_pool() {
    // Noise that evolves is a new picture every frame. Each one leaves the cache a few frames later and its
    // texture goes to the pool, where a blur of the same size keeps that size in use: the pool must not then
    // hold one more texture per frame for the rest of the render.
    let xml = r##"<scene version="1.2"><project width="64" height="64" fps="10" duration="6"/>
      <assets><generator id="a" kind="fractal-noise" width="64" height="64" scale="8" seed="3">
        <animate property="evolution"><key time="0" value="0"/><key time="6" value="6"/></animate></generator></assets>
      <composition><layer id="l" asset="a" effects="soft"/></composition>
      <effects><effect id="soft" type="blur" radius="1"/></effects></scene>"##;
    let d = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    let Some(gpu) = gpu() else { return };
    let ev = sr_eval::Evaluator::new(&d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = sr_gpu::Renderer::new(gpu, ev.program());
    let (mut held, mut shown) = (Vec::new(), Vec::new());
    for k in 0..60 {
        let f = r.render(&ev.evaluate(k as f64 * 0.1), ev.program());
        assert!(f.stats.errors.is_empty() && f.stats.unsupported.is_empty(), "{:?}", f.stats);
        if k < 2 {
            shown.push(r.read(&f.texture));
        }
        held.push(r.pooled_textures());
    }
    assert!(shown[0] != shown[1], "the noise must evolve from frame to frame");
    let max = *held.iter().max().unwrap();
    assert!(max <= 16, "pooled textures stay bounded: {held:?}");
}
