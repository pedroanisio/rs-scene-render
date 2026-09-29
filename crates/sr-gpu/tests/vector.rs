//! Shapes, strokes, trim, modifiers, gradients,
//! masks, SVG and Lottie assets, and deformed layers.

mod common;
use common::*;

/// A 64×32 document on black with extra assets.
fn doc_assets(assets: &str, body: &str) -> sr_model::Document {
    let a = ASSETS.replace("</assets>", &format!("{assets}</assets>"));
    let xml = format!(
        r##"<scene version="1.1"><project width="64" height="32" fps="10" duration="2" background="#000000"/>{a}<composition>{body}</composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}"))
}

fn write(name: &str, data: &str) {
    std::fs::write(fixtures().join(name), data).unwrap();
}

#[test]
fn shapes_fill_with_exact_coverage() {
    let d = doc(
        r##"background="#000000""##,
        "",
        r##"<shape id="r" shape="rect" x="8.5" y="4" width="16" height="8" fill="#FF0000"/>
           <shape id="e" shape="ellipse" x="36" y="4" width="20" height="20" fill="#00FF00"/>"##,
    );
    let Some(r) = render(&d) else { return };
    assert_px(&r, 16, 8, [1.0, 0.0, 0.0, 1.0], 1e-3);
    // x = 8.5: pixel 8 is half covered
    assert_px(&r, 8, 8, [0.5, 0.0, 0.0, 1.0], 2e-3);
    assert_px(&r, 4, 8, [0.0, 0.0, 0.0, 1.0], 1e-3);
    // the ellipse's green sums to its area, less the chord error of 0.05 px flattening
    // (at most perimeter × 0.05 × 2/3 ≈ 2.1 px² for r = 10)
    let area: f32 = r.px.iter().map(|p| p[1]).sum();
    let exact = std::f32::consts::PI * 100.0;
    assert!(area <= exact + 0.05 && area > exact - 2.2, "{area}");
    // both simple shapes rasterise in one batch: background + one draw
    assert_eq!(r.stats.draws, 2, "{:?}", r.stats);
    assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
}

#[test]
fn strokes_trim_dash_and_position() {
    let d = doc(
        r##"background="#000000" width="64" height="32""##,
        "",
        r##"<shape id="l" shape="line" x="0" y="0" width="64" height="8" stroke="#FFFFFF" strokeWidth="2" trimEnd="0.5"/>
           <shape id="d" shape="line" x="0" y="10" width="64" height="4" stroke="#FFFFFF" strokeWidth="2" dash="8 8"/>
           <shape id="o" shape="rect" x="20" y="18" width="12" height="12" fill="#FF0000" stroke="#0000FF" strokeWidth="2" strokePosition="outside"/>"##,
    );
    let Some(r) = render(&d) else { return };
    // trimmed line: left half only (centre y = 4)
    assert_px(&r, 10, 4, [1.0, 1.0, 1.0, 1.0], 1e-3);
    assert_px(&r, 50, 4, [0.0, 0.0, 0.0, 1.0], 1e-3);
    // dashes: on 0–8, off 8–16
    assert_px(&r, 4, 12, [1.0, 1.0, 1.0, 1.0], 1e-3);
    assert_px(&r, 12, 12, [0.0, 0.0, 0.0, 1.0], 1e-3);
    // outside stroke: 2 px band outside the red box, box interior untouched
    assert_px(&r, 19, 24, [0.0, 0.0, 1.0, 1.0], 1e-3);
    assert_px(&r, 21, 24, [1.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 17, 24, [0.0, 0.0, 0.0, 1.0], 1e-3);
}

#[test]
fn modifiers_gradients_and_isolated_vectors() {
    let d = doc(
        r##"background="#000000""##,
        r##"<paints><linearGradient id="g" dither="false"><stop offset="0" color="#000000"/><stop offset="1" color="#FFFFFF"/></linearGradient></paints>"##,
        r##"<shape id="rep" shape="rect" x="0" y="0" width="8" height="8" fill="#FF0000">
             <shapeModifier type="repeater" copies="3" offsetX="10"/>
           </shape>
           <shape id="grad" shape="rect" x="0" y="16" width="32" height="8" fill="url(#g)"/>
           <shape id="masked" shape="rect" x="40" y="16" width="16" height="16" fill="#00FF00" blend="screen">
             <mask type="rect" x="0" y="0" width="8" height="16"/>
           </shape>"##,
    );
    let Some(r) = render(&d) else { return };
    // repeater pivots on the box centre: copies at x = 0, 10, 20
    for x in [4, 14, 24] {
        assert_px(&r, x, 4, [1.0, 0.0, 0.0, 1.0], 1e-3);
    }
    assert_px(&r, 9, 4, [0.0, 0.0, 0.0, 1.0], 1e-3);
    // gradient over the shape's box, left to right (linear-light interpolation)
    let (a, b) = (r.at(2, 20)[0], r.at(29, 20)[0]);
    assert!(a < 0.15 && b > 0.85 && a < b, "{a} {b}");
    // a masked, screen-blended shape rasterises separately and keeps its mask
    assert_px(&r, 44, 20, [0.0, 1.0, 0.0, 1.0], 2e-3);
    assert_px(&r, 52, 20, [0.0, 0.0, 0.0, 1.0], 2e-3);
}

#[test]
fn svg_and_lottie_assets() {
    write(
        "b5.svg",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="8" height="16" fill="#ff0000"/><circle cx="12" cy="8" r="4" fill="#0000ff"/></svg>"##,
    );
    write(
        "b5.json",
        r##"{"fr":10,"ip":0,"op":20,"w":16,"h":16,"layers":[{"ty":4,"ind":1,"ip":0,"op":20,
          "ks":{"p":{"a":1,"k":[{"t":0,"s":[0,0],"o":{"x":[0],"y":[0]},"i":{"x":[1],"y":[1]}},{"t":10,"s":[8,0]}]}},
          "shapes":[{"ty":"rc","p":{"a":0,"k":[4,8]},"s":{"a":0,"k":[8,16]},"r":{"a":0,"k":0}},{"ty":"fl","c":{"sid":"c","a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100}}]}]}"##,
    );
    let d = doc_assets(
        r##"<vector id="v" shape="svg" src="b5.svg" width="16" height="16"/>
             <lottie id="lt" src="b5.json" width="16" height="16"><slot id="c" value="#00FF00"/></lottie>
             <vector id="star" shape="star" width="16" height="16" points="5" fill="#FFFFFF"/>"##,
        r##"<layer id="s" asset="v" x="0" y="0" scaleX="2" scaleY="2"/>
           <layer id="l" asset="lt" x="32" y="0"/>
           <layer id="st" asset="star" x="48" y="0"/>"##,
    );
    let Some(r) = render_times(&d, &[0.5]) else { return };
    // SVG at 2×: red left half, blue circle at (24, 16)
    assert_px(&r, 4, 16, [1.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 24, 16, [0.0, 0.0, 1.0, 1.0], 1e-3);
    // Lottie at 0.5 s = frame 5: the square moved 4 px right, green from the slot override
    assert_px(&r, 32 + 6, 8, [0.0, 1.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 32 + 2, 8, [0.0, 0.0, 0.0, 1.0], 1e-3);
    // star centre is white
    assert_px(&r, 56, 8, [1.0, 1.0, 1.0, 1.0], 1e-3);
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
}

#[test]
fn deformed_image_layer_draws_a_mesh() {
    let d = doc(
        r##"background="#000000""##,
        "",
        r##"<layer id="w" asset="white" x="16" y="8" scaleX="4" scaleY="4">
             <deform><modifier type="corner-pin" corners="0 0 8 0 8 4 0 4"/></deform>
           </layer>"##,
    );
    let Some(r) = render(&d) else { return };
    // the 4×4 layer (16×16 px on screen) is pinned onto 8×4 local units: 32×16 px from (16, 8)
    assert_px(&r, 20, 12, [1.0, 1.0, 1.0, 1.0], 1e-3);
    assert_px(&r, 44, 12, [1.0, 1.0, 1.0, 1.0], 1e-3);
    assert_px(&r, 50, 12, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 20, 26, [0.0, 0.0, 0.0, 1.0], 1e-3);
}

#[test]
fn skinned_layer_follows_its_skeleton() {
    let d = doc(
        r##"background="#000000""##,
        "",
        r##"<skeleton id="rig">
             <bone id="b" x="0" y="8" length="32"><animate property="rotation"><key time="0" value="0"/><key time="1" value="90"/></animate></bone>
           </skeleton>
           <layer id="w" asset="white" x="0" y="0" scaleX="8" scaleY="4">
             <deform><modifier type="skin" skeleton="rig"/></deform>
           </layer>"##,
    );
    let Some(rest) = render_times(&d, &[0.0]) else { return };
    assert_px(&rest, 24, 8, [1.0, 1.0, 1.0, 1.0], 1e-3);
    // one bone: the layer turns rigidly 90° about the joint (0, 8)
    let Some(bent) = render_times(&d, &[1.0]) else { return };
    assert_px(&bent, 24, 8, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&bent, 3, 24, [1.0, 1.0, 1.0, 1.0], 1e-3);
}

#[test]
fn puppet_pins_deform_a_shape() {
    let d = doc(
        r##"background="#000000""##,
        "",
        r##"<shape id="p" shape="rect" x="8" y="8" width="48" height="16" fill="#FFFFFF">
             <deform><modifier type="puppet">
               <pin restX="4" restY="8"/>
               <pin restX="44" restY="8" y="-6"/>
             </modifier></deform>
           </shape>"##,
    );
    let Some(r) = render(&d) else { return };
    // the pinned left end stays; the right end lifts 6 px
    assert_px(&r, 12, 22, [1.0, 1.0, 1.0, 1.0], 1e-3);
    assert_px(&r, 52, 20, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 52, 5, [1.0, 1.0, 1.0, 1.0], 1e-3);
}

#[test]
fn pattern_paints_fill_shapes() {
    let xml = format!(
        r##"<scene version="1.1"><project width="64" height="64" fps="10" duration="1" background="#00000000"/>{ASSETS}<paints><pattern id="pat" asset="red" tileWidth="8" tileHeight="8"/></paints><composition><shape id="c" shape="ellipse" x="8" y="8" width="48" height="48" fill="url(#pat)"/></composition></scene>"##
    );
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) })
        .unwrap_or_else(|e| panic!("{e:?}"));
    let Some(r) = render(&d) else { return };
    assert!(
        r.stats.unsupported.is_empty() && r.stats.errors.is_empty(),
        "{:?} {:?}",
        r.stats.unsupported,
        r.stats.errors
    );
    let c = r.at(32, 32);
    assert!(c[3] > 0.99 && c[0] > 0.9 && c[1] < 0.05, "pattern inside the circle: {c:?}");
    assert_eq!(r.at(9, 9)[3], 0.0, "nothing outside it");
    assert_eq!(r.at(1, 32)[3], 0.0);
}
