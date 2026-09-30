//! What a frame costs in passes, copies and allocations: pointwise colour operations in a row
//! run as one pass, blend modes the blender can do need no backdrop copy, and effect targets
//! that follow moving content reuse their textures. Each test also checks the pixels.

mod common;

use common::*;

fn scene(project: &str, body: &str, effects: &str) -> sr_model::Document {
    let effects = if effects.is_empty() { String::new() } else { format!("<effects>{effects}</effects>") };
    let xml = format!(
        r#"<scene version="1.1"><project {project}/>{ASSETS}<composition>{body}</composition>{effects}</scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

const PROJECT: &str = r##"width="96" height="64" fps="30" duration="2" background="#203040""##;

const COLOUR_OPS: &str = r##"<effect id="grade" type="color-grade" saturation="1.3" contrast="1.2" brightness="0.02"/>
<effect id="exp" type="exposure" exposure="0.5"/>
<effect id="vig" type="vignette" amount="0.4" radius="30"/>
<effect id="grain" type="film-grain" amount="0.05" seed="7"/>"##;

#[test]
fn colour_operations_in_a_row_run_as_one_pass() {
    let picture = r#"<layer id="img" asset="wide" x="8" y="8" scaleX="10" scaleY="12"/>"#;
    // one chain of four pointwise operations, and the same four as separate adjustment layers
    let fused =
        scene(PROJECT, &format!(r#"{picture}<adjustment id="finish" effects="grade exp vig grain"/>"#), COLOUR_OPS);
    let apart = scene(
        PROJECT,
        &format!(
            r#"{picture}<adjustment id="a1" effects="grade"/><adjustment id="a2" effects="exp"/><adjustment id="a3" effects="vig"/><adjustment id="a4" effects="grain"/>"#
        ),
        COLOUR_OPS,
    );
    let (Some(f), Some(a)) = (render_sub(&fused, 0.5), render_sub(&apart, 0.5)) else { return };
    let db = psnr(&f.px, &a.px);
    assert!(db >= 60.0, "one pass vs four: {db:.1} dB");
    assert_eq!(a.stats.fx_passes, 4);
    assert_eq!(f.stats.fx_passes, 1, "four colour operations in a row are one pass");
}

#[test]
fn a_chain_is_not_fused_across_other_effects() {
    // grade, blur, exposure: the blur needs its own passes, so the chain keeps three stages
    let d = scene(
        PROJECT,
        r#"<layer id="img" asset="wide" x="8" y="8" scaleX="10" scaleY="12"/><adjustment id="finish" effects="grade soft exp"/>"#,
        r##"<effect id="grade" type="color-grade" saturation="1.3"/><effect id="soft" type="blur" radius="1"/><effect id="exp" type="exposure" exposure="0.5"/>"##,
    );
    let e = scene(
        PROJECT,
        r#"<layer id="img" asset="wide" x="8" y="8" scaleX="10" scaleY="12"/><adjustment id="a1" effects="grade"/><adjustment id="a2" effects="soft"/><adjustment id="a3" effects="exp"/>"#,
        r##"<effect id="grade" type="color-grade" saturation="1.3"/><effect id="soft" type="blur" radius="1"/><effect id="exp" type="exposure" exposure="0.5"/>"##,
    );
    let (Some(a), Some(b)) = (render_sub(&d, 0.0), render_sub(&e, 0.0)) else { return };
    assert!(psnr(&a.px, &b.px) >= 60.0);
}

#[test]
fn moving_effect_targets_reuse_their_textures() {
    // a blurred layer drifting about 1.2 px per frame: its effect bounds move every frame and
    // change size by a pixel now and then
    let d = scene(
        PROJECT,
        r#"<layer id="m" asset="white" x="10" y="12.3" scaleX="6" scaleY="5" effects="soft">
             <animate property="x"><key time="0" value="10"/><key time="2" value="80"/></animate>
             <animate property="y"><key time="0" value="12.3"/><key time="2" value="20.9"/></animate></layer>"#,
        r#"<effect id="soft" type="blur" radius="0.6"/>"#,
    );
    let times: Vec<f64> = (0..10).map(|k| 0.2 + k as f64 / 30.0).collect();
    let Some(f) = render_sub_frames(&d, &times) else { return };
    let created: Vec<usize> = f.iter().map(|s| s.stats.textures_created).collect();
    assert!(created[0] > 0);
    assert!(created[3..].iter().all(|&c| c == 0), "textures created per frame: {created:?}");
    // the reused textures hold the right pixels
    let fresh = render_sub(&d, times[9]).unwrap();
    assert_eq!(f[9].px, fresh.px);
}

#[test]
fn cached_effect_results_follow_every_animated_value() {
    // what the cache key hashes: a node's animated properties, its parts (a mask) and its
    // effects' parameters; a change in any of them must not reuse the previous result
    for (body, fx) in [
        (
            r##"<shape id="s" shape="rect" x="20" y="10" width="40" height="30" fill="#FF0000" effects="soft">
                 <animate property="fill"><key time="0" value="#FF0000"/><key time="1" value="#00FF00"/></animate></shape>"##,
            r#"<effect id="soft" type="blur" radius="1"/>"#,
        ),
        (
            r##"<shape id="s" shape="rect" x="20" y="10" width="40" height="30" fill="#FF0000" effects="soft">
                 <mask type="ellipse" x="0" y="0" width="40" height="30">
                   <animate property="width"><key time="0" value="40"/><key time="1" value="10"/></animate></mask></shape>"##,
            r#"<effect id="soft" type="blur" radius="1"/>"#,
        ),
        (
            r##"<shape id="s" shape="rect" x="20" y="10" width="40" height="30" fill="#FF0000" effects="soft"/>"##,
            r#"<effect id="soft" type="blur" radius="1"><animate property="radius"><key time="0" value="1"/><key time="1" value="4"/></animate></effect>"#,
        ),
    ] {
        let d = scene(PROJECT, body, fx);
        let Some(f) = render_sub_frames(&d, &[0.2, 0.5]) else { return };
        let fresh = render_sub(&d, 0.5).unwrap();
        assert_ne!(f[0].px, f[1].px, "the value animates: {body}");
        assert_eq!(f[1].px, fresh.px, "no stale result: {body}{fx}");
    }
}

#[test]
fn caches_of_a_posterized_group_survive_its_held_frames() {
    // a posterized group redraws its children only on the first frame of each step; the
    // children's cached results and textures must outlive the frames between, not be freed
    // and made again (freeing a texture the GPU is still reading waits for the GPU)
    let d = scene(
        r##"width="96" height="64" fps="60" duration="2" background="#203040""##,
        r##"<group id="rig" effects="pt">
              <layer id="a" asset="white" x="10" y="10" scaleX="5" scaleY="5" effects="soft">
                <animate property="x"><key time="0" value="10"/><key time="2" value="60"/></animate></layer>
              <shape id="b" shape="ellipse" x="50" y="20" width="20" height="20" fill="#FF8000" effects="soft"/>
            </group>"##,
        r#"<effect id="pt" type="posterize-time" frequency="30"/><effect id="soft" type="blur" radius="0.8"/>"#,
    );
    let times: Vec<f64> = (0..12).map(|k| 0.5 + (k as f64 + 0.2) / 60.0).collect();
    let Some(f) = render_sub_frames(&d, &times) else { return };
    let released: Vec<usize> = f.iter().map(|s| s.stats.textures_released).collect();
    let created: Vec<usize> = f.iter().map(|s| s.stats.textures_created).collect();
    // sizes only the first frames asked for are released once, eight frames later; nothing is
    // made after warm-up, and nothing else is destroyed
    assert!(released[..8].iter().chain(&released[9..]).all(|&r| r == 0), "textures released per frame: {released:?}");
    assert!(created[4..].iter().all(|&c| c == 0), "textures created per frame: {created:?}");
}

#[test]
fn additive_blending_needs_no_backdrop_copy() {
    // add and linear-dodge are dst + src on premultiplied colour (alpha as normal): the blender
    // does that directly, without copying the backdrop for the shader to read
    for mode in ["add", "linear-dodge"] {
        let d = scene(
            r##"width="64" height="32" fps="10" duration="1" background="#000000""##,
            &format!(
                r##"<shape id="base" shape="rect" x="0" y="0" width="64" height="32" fill="#406080"/>
                    <shape id="top" shape="rect" x="16" y="8" width="32" height="16" fill="#20304080" blend="{mode}"/>"##
            ),
            "",
        );
        let Some(r) = render_sub(&d, 0.0) else { return };
        assert_eq!(r.stats.backdrop_copies, 0, "{mode}");
        let a = 128.0 / 255.0;
        let want = [lin8(0x40) + lin8(0x20) * a, lin8(0x60) + lin8(0x30) * a, lin8(0x80) + lin8(0x40) * a, 1.0];
        assert_px(&r, 32, 16, want, 2e-3);
        assert_px(&r, 4, 4, [lin8(0x40), lin8(0x60), lin8(0x80), 1.0], 2e-3);
    }
    // screen clamps its inputs to [0, 1], which the blender cannot: it still reads a backdrop copy
    let d = scene(
        r##"width="64" height="32" fps="10" duration="1" background="#000000""##,
        r##"<shape id="base" shape="rect" x="0" y="0" width="64" height="32" fill="#406080"/>
            <shape id="top" shape="rect" x="16" y="8" width="32" height="16" fill="#20304080" blend="screen"/>"##,
        "",
    );
    let Some(r) = render_sub(&d, 0.0) else { return };
    assert_eq!(r.stats.backdrop_copies, 1);
}
