//! Light through transparent and refracting surfaces in the path tracer: what a ray that crosses
//! glass or water and reaches nothing sees, how water absorbs, and how lights reach surfaces
//! under water.

mod common;

/// Linear luminance of an RGBA pixel.
fn lum(p: [f32; 4]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

/// A glass ball at the origin seen by a perspective camera, over `behind` (2D layers), under a
/// visible grey dome (or none).
fn glass_ball(behind: &str, dome: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="48" height="48" fps="10" duration="1" background="#00000000"/>
      <materials><material id="glassMat" transmission="1" roughness="0.02" ior="1.5"/></materials>
      <composition>{behind}
        <camera id="camera" x="0" y="0" z="-30" fov="40" renderer="pathtrace" pathSamples="32" maxBounces="4" denoise="false"/>
        <object3D id="ball" primitive="sphere" radius="6" segments="32" material="glassMat"/>
      </composition>
      <lights>{dome}<light id="sun" type="directional" intensity="1" yaw="30" pitch="-40"/></lights></scene>"##
    )
}

const VISIBLE_DOME: &str =
    r##"<light id="sky" type="dome" environment="gray.png" environmentVisible="true" intensity="1"/>"##;
const HIDDEN_DOME: &str = r##"<light id="sky" type="dome" environment="gray.png" intensity="1"/>"##;

fn render(scene: &str) -> Option<common::Rendered> {
    let gpu = common::gpu()?;
    let doc =
        sr_model::load_str(scene, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(common::fixtures()) })
            .unwrap_or_else(|e| panic!("{e:?}"));
    let shot = common::render_times_on(gpu, &doc, &[0.0]).unwrap();
    assert!(shot.stats.errors.is_empty(), "{:?}", shot.stats.errors);
    Some(shot)
}

/// Where the 2D layers behind glass leave a pixel empty, a ray that crosses the glass and reaches
/// nothing sees the visible dome, as it does without the glass.
#[test]
fn glass_over_nothing_shows_the_visible_dome() {
    let Some(r) = render(&glass_ball("", VISIBLE_DOME)) else { return };
    let dome = lum(r.px[0]);
    assert!(dome > 0.15, "the dome beside the ball: {dome}");
    let centre = r.px[24 * 48 + 24];
    assert!(lum(centre) > 0.5 * dome, "through the glass: {centre:?} against the dome {dome}");
}

/// A dome that is not visible stays hidden through glass: only the 2D layers show there.
#[test]
fn glass_over_nothing_does_not_show_a_hidden_dome() {
    let Some(r) = render(&glass_ball("", HIDDEN_DOME)) else { return };
    assert!(lum(r.px[24 * 48 + 24]) < 0.02, "{:?}", r.px[24 * 48 + 24]);
}

/// Opaque 2D layers behind the glass are what shows through it; the dome does not leak into them.
#[test]
fn glass_over_an_opaque_layer_shows_the_layer() {
    let layer = r##"<shape id="wall" shape="rect" x="0" y="0" width="48" height="48" fill="#FF0000"/>"##;
    let Some(r) = render(&glass_ball(layer, VISIBLE_DOME)) else { return };
    let c = r.px[24 * 48 + 24];
    assert!(c[0] > 0.4 && c[1] < 0.05 && c[2] < 0.05, "{c:?}");
}

fn linear(srgb: u8) -> f32 {
    let c = f32::from(srgb) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// A face-on water pane at z = 0 and an emissive wall `depth` units behind it, seen along the axis
/// (the centre pixel): what reaches the camera is the wall's emission, less the interface's
/// reflection and the water's absorption.
fn water_over_a_lamp(attenuation: &str, depth: f32) -> String {
    format!(
        r##"<scene version="1.3"><project width="16" height="16" fps="10" duration="1" background="#00000000"/>
      <materials>
        <material id="waterMat" transmission="1" roughness="0.02" ior="1.333" doubleSided="true" {attenuation}/>
        <material id="lampMat" baseColor="#000000" roughness="1" specular="0" emissive="#FFFFFF" emissiveStrength="1"/>
      </materials>
      <composition>
        <camera id="camera" x="0" y="0" z="-30" fov="20" renderer="pathtrace" pathSamples="1024" maxBounces="4" denoise="false"/>
        <object3D id="pane" primitive="plane" width="200" height="200" z="0" material="waterMat"/>
        <object3D id="lamp" primitive="plane" width="200" height="200" z="{depth}" material="lampMat"/>
      </composition>
      <lights><light id="off" type="ambient" color="#000000" intensity="0"/></lights></scene>"##
    )
}

/// The water absorbs by Beer-Lambert along the path inside it: after `attenuationDistance` units
/// the light left is `attenuationColor`, per channel. Without the attributes it does not absorb.
#[test]
fn water_absorbs_by_beer_lambert_along_the_path() {
    // reflection of the interface at normal incidence, as the tracer's Schlick term gives it
    let transmitted = 1.0 - ((1.0 - 1.0 / 1.333f32) / (1.0 + 1.0 / 1.333)).powi(2);
    let colour = [0x80u8, 0xC0, 0xE0];
    let attenuation = r##"attenuationColor="#80C0E0" attenuationDistance="4""##;
    let Some(plain) = render(&water_over_a_lamp("", 4.0)) else { return };
    let centre = |r: &common::Rendered| r.px[8 * 16 + 8];
    for c in 0..3 {
        let got = centre(&plain)[c];
        assert!((got / transmitted - 1.0).abs() < 0.03, "no attenuation, channel {c}: {got} against {transmitted}");
    }
    // one attenuation distance deep: the colour itself; half a distance deep: its square root
    for (depth, power) in [(4.0, 1.0f32), (2.0, 0.5)] {
        let Some(r) = render(&water_over_a_lamp(attenuation, depth)) else { return };
        for (c, value) in colour.iter().enumerate() {
            let want = transmitted * linear(*value).powf(power);
            let got = centre(&r)[c];
            assert!((got / want - 1.0).abs() < 0.03, "depth {depth}, channel {c}: {got} against {want}");
        }
    }
    // an attenuation colour with no distance does not absorb
    let Some(unbounded) = render(&water_over_a_lamp(r##"attenuationColor="#80C0E0""##, 4.0)) else { return };
    assert_eq!(unbounded.px, plain.px, "no distance, no absorption");
}
