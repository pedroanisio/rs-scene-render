//! SREP 71: the procedural sky of a dome light, material maps fed by generator assets, and the subsurface lobe.
//!
//! The SREP's pixel cases (sr-core `conformance/srep_cases/srep-0071.json`), with its documents and measures: a region's
//! mean colour within 3 code values; per colour class, the centre of its pixels and the extent of its bounding box
//! within 2 px. The two subsurface cases compare with a second render, which the kit's runner cannot express yet: here
//! they are the comparisons the kit states (equal at weight 0; brighter just past the terminator at weight 1). The
//! remaining tests carry the same sky through a 3D pass, the path tracer and image-based lighting.

mod common;
use common::*;

/// The kit's document: 640 × 360 on black, with assets, materials, a composition and lights.
fn kit(assets: &str, materials: &str, body: &str, lights: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<scene version=\"1.6\">\n<project width=\"640\" height=\"360\" fps=\"24\" duration=\"1\" background=\"#000000FF\" seed=\"1\"/>\n<output id=\"still\" path=\"out/frame_%04d.png\" codec=\"png-sequence\"/>\n{assets}{materials}<composition>\n{body}\n</composition>\n{lights}</scene>\n"
    )
}

fn render_xml(xml: &str) -> Option<Rendered> {
    let d =
        sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    let r = render(&d)?;
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    Some(r)
}

/// The 8-bit display values of the frame.
fn display(r: &Rendered) -> Vec<u8> {
    r.renderer.to_srgb8(&r.px)
}

/// The mean colour of the box [x0, x1) × [y0, y1) in 8-bit code values.
fn mean(r: &Rendered, d: &[u8], [x0, y0, x1, y1]: [u32; 4]) -> [f64; 3] {
    let mut sum = [0f64; 3];
    for y in y0..y1 {
        for x in x0..x1 {
            let offset = ((y * r.size[0] + x) * 4) as usize;
            for (k, s) in sum.iter_mut().enumerate() {
                *s += d[offset + k] as f64;
            }
        }
    }
    let n = ((x1 - x0) * (y1 - y0)) as f64;
    sum.map(|s| s / n)
}

/// The kit's region measure: the box's mean within 3 code values of `rgb`.
#[track_caller]
fn region(r: &Rendered, d: &[u8], b: [u32; 4], rgb: [f64; 3]) {
    let got = mean(r, d, b);
    assert!(got.iter().zip(rgb).all(|(g, w)| (g - w).abs() <= 3.0), "box {b:?}: {got:?}, expected {rgb:?}");
}

/// The kit's class measure: (cx, cy, w, h) of the pixels of a class, or None under 12 pixels.
fn measure(r: &Rendered, is: impl Fn([u8; 3]) -> bool) -> Option<[f64; 4]> {
    let d = display(r);
    let [w, h] = r.size;
    let (mut n, mut sx, mut sy) = (0usize, 0f64, 0f64);
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            let o = ((y * w + x) * 4) as usize;
            if is([d[o], d[o + 1], d[o + 2]]) {
                n += 1;
                sx += x as f64;
                sy += y as f64;
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    (n >= 12).then(|| [sx / n as f64 + 0.5, sy / n as f64 + 0.5, (x1 - x0 + 1) as f64, (y1 - y0 + 1) as f64])
}

fn red(c: [u8; 3]) -> bool {
    c[0] > 127 && c[1] < 77 && c[2] < 77
}

fn green(c: [u8; 3]) -> bool {
    c[1] > 127 && c[0] < 77 && c[2] < 77
}

#[track_caller]
fn expect(got: Option<[f64; 4]>, want: &[f64], what: &str) {
    let got = got.unwrap_or_else(|| panic!("no {what} drawn"));
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 2.0), "{what}: measured {got:?}, expected {want:?}");
}

const GRADIENT: &str = "<lights><light id=\"sky\" type=\"dome\" sky=\"gradient\" skyZenith=\"#2050A0\" skyHorizon=\"#C0D0E0\" skyGround=\"#403020\" skyExponent=\"1\" environmentVisible=\"true\"/></lights>\n";

fn sun(azimuth: &str, elevation: &str) -> String {
    format!("<lights><light id=\"sky\" type=\"dome\" sky=\"gradient\" skyZenith=\"#000000\" skyHorizon=\"#000000\" skyGround=\"#000000\" sunAzimuth=\"{azimuth}\" sunElevation=\"{elevation}\" sunSize=\"5\" sunColor=\"#FF0000\" sunIntensity=\"1\" environmentVisible=\"true\"/></lights>\n")
}

#[test]
fn srep_0071_sky_gradient() {
    let Some(r) = render_xml(&kit("", "", "", GRADIENT)) else { return };
    let d = display(&r);
    for ((x, y), rgb) in [
        ((320, 0), [163.6, 180.8, 207.0]),
        ((320, 90), [178.0, 194.6, 215.4]),
        ((320, 270), [178.9, 193.1, 207.5]),
        ((320, 359), [165.8, 178.1, 190.9]),
        ((0, 0), [167.4, 184.5, 209.2]),
    ] {
        region(&r, &d, [x, y, x + 1, y + 1], rgb);
    }
}

#[test]
fn srep_0071_sky_horizon() {
    let Some(r) = render_xml(&kit("", "", "", GRADIENT)) else { return };
    region(&r, &display(&r), [300, 179, 340, 181], [192.0, 208.0, 224.0]);
}

#[test]
fn srep_0071_sky_sun() {
    let Some(r) = render_xml(&kit("", "", "", &sun("0", "0"))) else { return };
    expect(measure(&r, red), &[320.0, 180.0, 48.4, 48.4], "red");
}

#[test]
fn srep_0071_sky_sun_azimuth() {
    let Some(r) = render_xml(&kit("", "", "", &sun("10", "5"))) else { return };
    expect(measure(&r, red), &[417.7, 130.8], "red");
}

const GEN_ASSETS: &str = "<assets><generator id=\"g\" kind=\"solid\" width=\"16\" height=\"16\" paint=\"#00FF00FF\"/><image id=\"im\" src=\"../assets/tile.png\" width=\"8\" height=\"8\"/></assets>\n";
const PLANE: &str =
    "<object3D id=\"o\" primitive=\"plane\" width=\"100\" height=\"100\" x=\"320\" y=\"180\" material=\"m\"/>";

#[test]
fn srep_0071_map_generator() {
    let mat =
        "<materials><material id=\"m\" baseColor=\"#FFFFFFFF\" unlit=\"true\" baseColorMap=\"#g\"/></materials>\n";
    let Some(r) = render_xml(&kit(GEN_ASSETS, mat, PLANE, "")) else { return };
    expect(measure(&r, green), &[320.0, 180.0, 100.0, 100.0], "green");
    // and the colour itself: #00FF00 through an 8-bit sRGB map
    region(&r, &display(&r), [300, 160, 340, 200], [0.0, 255.0, 0.0]);
}

#[test]
fn a_generator_map_is_the_generator_s_picture_and_an_image_asset_its_file() {
    // a checkerboard as the base colour of an unlit plane: the plane shows the checkerboard's squares, in its colours
    let assets = "<assets><generator id=\"c\" kind=\"checkerboard\" width=\"64\" height=\"64\" scale=\"32\" paint=\"#FF0000\" paint2=\"#0000FF\"/><image id=\"q\" src=\"quad.png\" width=\"2\" height=\"2\"/></assets>\n";
    let mat = |map: &str, filter: &str| {
        format!("<materials><material id=\"m\" unlit=\"true\" baseColorMap=\"{map}\" {filter}/></materials>\n")
    };
    let Some(r) = render_xml(&kit(assets, &mat("#c", ""), PLANE, "")) else { return };
    let d = display(&r);
    // the four quadrants of the plane: the checkerboard's 2 × 2 squares
    let q = |x: u32, y: u32| mean(&r, &d, [x - 5, y - 5, x + 5, y + 5]);
    let (a, b) = (q(295, 155), q(345, 155));
    let (c, e) = (q(295, 205), q(345, 205));
    let is_red = |v: [f64; 3]| v[0] > 200.0 && v[2] < 60.0;
    let is_blue = |v: [f64; 3]| v[2] > 200.0 && v[0] < 60.0;
    assert!(is_red(a) != is_red(b) && is_red(a) == is_red(e) && is_red(b) == is_red(c), "{a:?} {b:?} {c:?} {e:?}");
    assert!([a, b, c, e].iter().all(|v| is_red(*v) || is_blue(*v)), "{a:?} {b:?} {c:?} {e:?}");
    // an image asset: the file of the asset, resolved against the document
    let xml = kit(assets, &mat("#q", ""), PLANE, "");
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    let doc = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}"));
    let r = render(&doc).unwrap();
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    let d = display(&r);
    // quad.png's four pixels, red, green, blue and white, one in each quadrant of the plane
    let mut seen: Vec<&str> = [(295, 155), (345, 155), (295, 205), (345, 205)]
        .iter()
        .map(|&(x, y)| {
            let v = mean(&r, &d, [x - 5, y - 5, x + 5, y + 5]);
            match v.map(|c| c > 200.0) {
                [true, false, false] => "red",
                [false, true, false] => "green",
                [false, false, true] => "blue",
                [true, true, true] => "white",
                _ => panic!("({x}, {y}) is {v:?}"),
            }
        })
        .collect();
    seen.sort();
    assert_eq!(seen, ["blue", "green", "red", "white"]);
}

#[test]
fn a_generator_map_in_a_data_slot_reads_code_values() {
    // a mid-grey generator as the metallic-roughness map (roughness in green, metallic in blue) of a white metal:
    // the same picture as an 8-bit file of #808080 would give, so the plane renders as with the gray.png file
    let assets = "<assets><generator id=\"g\" kind=\"solid\" width=\"4\" height=\"4\" paint=\"#808080\"/><image id=\"gray\" src=\"gray.png\" width=\"4\" height=\"4\"/></assets>\n";
    let lights = "<lights><light id=\"k\" type=\"directional\" yaw=\"20\" pitch=\"-30\" intensity=\"2\"/><light id=\"a\" type=\"ambient\" intensity=\"0.3\"/></lights>\n";
    let render_map = |map: &str| {
        let mat = format!("<materials><material id=\"m\" metallic=\"1\" roughness=\"1\" metallicRoughnessMap=\"{map}\"/></materials>\n");
        let xml = kit(assets, &mat, PLANE, lights);
        let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
        render(&sr_model::load_str(&xml, &opts).unwrap())
    };
    let Some(a) = render_map("#g") else { return };
    let b = render_map("gray.png").unwrap();
    let (da, db) = (display(&a), display(&b));
    let (ma, mb) = (mean(&a, &da, [280, 140, 360, 220]), mean(&b, &db, [280, 140, 360, 220]));
    assert!(ma.iter().zip(mb).all(|(x, y)| (x - y).abs() <= 1.0), "{ma:?} vs {mb:?}");
}

const LIT: &str = "<lights><light id=\"sun\" type=\"directional\" yaw=\"60\" intensity=\"3\"/></lights>\n";
const BALL: &str = "<object3D id=\"o\" primitive=\"sphere\" radius=\"80\" x=\"320\" y=\"180\" material=\"m\"/>";

fn subsurface(attrs: &str) -> String {
    format!("<materials><material id=\"m\" baseColor=\"#B0B0B0FF\" roughness=\"0.5\" {attrs}/></materials>\n")
}

const SSS_ATTRS: &str = "subsurfaceColor=\"#E0F0D0FF\" subsurfaceRadius=\"20\"";

#[test]
fn srep_0071_subsurface_zero_neutral() {
    // weight 0: the same picture as without the subsurface attributes, bit for bit
    let Some(a) = render_xml(&kit("", &subsurface(&format!("subsurface=\"0\" {SSS_ATTRS}")), BALL, LIT)) else {
        return;
    };
    let b = render_xml(&kit("", &subsurface(""), BALL, LIT)).unwrap();
    assert!(a.px == b.px, "weight 0 changes the picture");
}

/// The mean of the pixels of the sphere whose normal is just past the terminator of the kit's light: the band of
/// cos θ ∈ [−0.25, −0.02] on the sphere's visible disc, with the frame's light direction recovered from the lit
/// render (the brightest pixel's normal).
fn terminator_band(r: &Rendered, light: [f64; 3]) -> ([f64; 3], usize) {
    let d = display(r);
    let (mut sum, mut n) = ([0f64; 3], 0usize);
    for y in 100..260u32 {
        for x in 240..400u32 {
            let (dx, dy) = ((x as f64 + 0.5 - 320.0) / 80.0, (y as f64 + 0.5 - 180.0) / 80.0);
            let rr = dx * dx + dy * dy;
            if rr > 0.81 {
                continue;
            }
            // the visible normal (toward the camera is −z), scene space: +x right, +y down
            let nrm = [dx, dy, -(1.0 - rr).sqrt()];
            let c = nrm[0] * light[0] + nrm[1] * light[1] + nrm[2] * light[2];
            if (-0.25..=-0.02).contains(&c) {
                let o = ((y * r.size[0] + x) * 4) as usize;
                for k in 0..3 {
                    sum[k] += d[o + k] as f64;
                }
                n += 1;
            }
        }
    }
    (sum.map(|s| s / n.max(1) as f64), n)
}

/// The direction toward the light, from the brightest pixel of the sphere in a Lambertian render.
fn toward_light(r: &Rendered) -> [f64; 3] {
    let d = display(r);
    let (mut best, mut at) = (0u32, [0.0; 3]);
    for y in 100..260u32 {
        for x in 240..400u32 {
            let (dx, dy) = ((x as f64 + 0.5 - 320.0) / 80.0, (y as f64 + 0.5 - 180.0) / 80.0);
            let rr = dx * dx + dy * dy;
            if rr > 0.9 {
                continue;
            }
            let o = ((y * r.size[0] + x) * 4) as usize;
            let v = d[o] as u32 + d[o + 1] as u32 + d[o + 2] as u32;
            if v > best {
                best = v;
                at = [dx, dy, -(1.0 - rr).sqrt()];
            }
        }
    }
    at
}

#[test]
fn srep_0071_subsurface_wraps_terminator() {
    let Some(w0) = render_xml(&kit("", &subsurface(&format!("subsurface=\"0\" {SSS_ATTRS}")), BALL, LIT)) else {
        return;
    };
    let w1 = render_xml(&kit("", &subsurface(&format!("subsurface=\"1\" {SSS_ATTRS}")), BALL, LIT)).unwrap();
    let light = toward_light(&w0);
    let (dark, n) = terminator_band(&w0, light);
    let (wrapped, m) = terminator_band(&w1, light);
    assert!(n > 200 && n == m, "{n} {m} pixels past the terminator");
    // brighter just past the terminator (Semantics 1, the wrap property), and redder there (the bleeding order)
    assert!(wrapped.iter().sum::<f64>() > dark.iter().sum::<f64>() + 6.0, "{wrapped:?} vs {dark:?}");
    assert!(wrapped[0] > wrapped[2], "{wrapped:?}");
}

#[test]
fn the_wrap_grows_with_the_radius_and_zero_radius_is_lambert() {
    let band = |radius: &str| {
        let m = subsurface(&format!("subsurface=\"1\" subsurfaceColor=\"#B0B0B0FF\" subsurfaceRadius=\"{radius}\""));
        render_xml(&kit("", &m, BALL, LIT))
    };
    let Some(lambert) = render_xml(&kit("", &subsurface(""), BALL, LIT)) else { return };
    let light = toward_light(&lambert);
    let at = |r: &Rendered| terminator_band(r, light).0.iter().sum::<f64>();
    let zero = band("0").unwrap();
    // radius 0 with the base colour as the subsurface colour: the Lambertian picture (within rounding of the lobe)
    let (dl, dz) = (display(&lambert), display(&zero));
    let worst = dl.iter().zip(&dz).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
    assert!(worst <= 2, "radius 0 differs from Lambert by {worst} code values");
    let (r5, r20, r60) = (at(&band("5").unwrap()), at(&band("20").unwrap()), at(&band("60").unwrap()));
    assert!(at(&zero) < r5 && r5 < r20 && r20 < r60, "{} {r5} {r20} {r60}", at(&zero));
}

#[test]
fn the_sky_of_a_3d_pass_is_the_same_sky() {
    // with an object in the frame the 3D pass draws the background: the kit's gradient where the object is not
    let body = "<object3D id=\"o\" primitive=\"sphere\" radius=\"20\" x=\"600\" y=\"330\"/>";
    let Some(r) = render_xml(&kit("", "", body, GRADIENT)) else { return };
    let d = display(&r);
    for ((x, y), rgb) in
        [((320, 0), [163.6, 180.8, 207.0]), ((320, 90), [178.0, 194.6, 215.4]), ((0, 0), [167.4, 184.5, 209.2])]
    {
        region(&r, &d, [x, y, x + 1, y + 1], rgb);
    }
    region(&r, &d, [300, 179, 340, 181], [192.0, 208.0, 224.0]);
}

#[test]
fn the_path_tracer_draws_the_same_sky() {
    // the same camera, path traced and rasterised: the background is the sky's formula in both
    let body = |renderer: &str| {
        format!("<camera id=\"c\" renderer=\"{renderer}\" pathSamples=\"4\" maxBounces=\"1\" denoise=\"false\"/><object3D id=\"o\" primitive=\"sphere\" radius=\"20\" x=\"600\" y=\"330\"/>")
    };
    let Some(raster) = render_xml(&kit("", "", &body("raster"), GRADIENT)) else { return };
    let traced = render_xml(&kit("", "", &body("pathtrace"), GRADIENT)).unwrap();
    let (a, b) = (display(&raster), display(&traced));
    for bx in [[300, 0, 340, 20], [0, 0, 40, 40], [300, 170, 340, 190], [100, 300, 140, 340]] {
        let (ma, mb) = (mean(&raster, &a, bx), mean(&traced, &b, bx));
        assert!(ma.iter().zip(mb).all(|(x, y)| (x - y).abs() <= 3.0), "{bx:?}: raster {ma:?}, traced {mb:?}");
    }
}

#[test]
fn chrome_reflects_the_sky_instead_of_a_flat_grey() {
    // a mirror ball under the gradient sky: its top reflects the zenith's blue and its bottom the ground's brown
    let mat = "<materials><material id=\"m\" baseColor=\"#FFFFFFFF\" metallic=\"1\" roughness=\"0.05\"/></materials>\n";
    let dome = GRADIENT.replace(" environmentVisible=\"true\"", "");
    let Some(r) = render_xml(&kit("", mat, BALL, &dome)) else { return };
    let d = display(&r);
    let top = mean(&r, &d, [315, 108, 325, 114]);
    let bottom = mean(&r, &d, [315, 246, 325, 252]);
    assert!(top[2] > top[0] + 20.0, "top {top:?}");
    assert!(
        bottom[0] > bottom[2] + 5.0 || bottom.iter().sum::<f64>() + 60.0 < top.iter().sum::<f64>(),
        "bottom {bottom:?}, top {top:?}"
    );
}
