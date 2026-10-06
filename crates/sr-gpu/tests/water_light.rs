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
/// the light left is `attenuationColor`, per channel. Without the attributes it does not absorb. The
/// lamp behind the pane is a submerged emitter: seen from the air it is 1 / eta^2 as bright.
#[test]
fn water_absorbs_by_beer_lambert_along_the_path() {
    // reflection of the interface at normal incidence, as the tracer's Schlick term gives it; the
    // lamp is in the water, so the radiance seen from the air is 1 / eta^2 of the lamp's own
    let transmitted = (1.0 - ((1.0 - 1.0 / 1.333f32) / (1.0 + 1.0 / 1.333)).powi(2)) / (1.333 * 1.333);
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

// ---- a seabed under flat water, lit by analytic lights and by brute force

/// Mean luminance of a rectangle of a rendered frame.
fn patch(r: &common::Rendered, [x0, y0, x1, y1]: [usize; 4]) -> f32 {
    let w = r.size[0] as usize;
    let mut sum = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            sum += lum(r.px[y * w + x]);
        }
    }
    sum / ((x1 - x0) * (y1 - y0)) as f32
}

/// The direction a light with this yaw and pitch (degrees) shines along.
fn shines(yaw: f32, pitch: f32) -> glam::Vec3 {
    (glam::Mat4::from_rotation_y(yaw.to_radians()) * glam::Mat4::from_rotation_x(pitch.to_radians()))
        .transform_vector3(glam::Vec3::Z)
}

const WATER: &str = r##"<material id="waterMat" baseColor="#FFFFFF" roughness="0.02" ior="1.333" doubleSided="true" transmission="1" {water}/>"##;

/// A diffuse floor 4 units under a flat water plane (when `water` is not empty), with a box on it.
/// `lights` and `extra` (more objects) are spliced in; the camera looks down at the floor.
fn seabed(water: &str, lights: &str, extra: &str, camera: &str, size: [u32; 2], samples: u32, bounces: u32) -> String {
    let pane = if water.is_empty() {
        String::new()
    } else {
        r##"<object3D id="water" primitive="plane" width="400" height="400" y="0" rotationX="-90" material="waterMat"/>"##
            .to_string()
    };
    let water_material = if water.is_empty() {
        String::new()
    } else {
        WATER.replace("{water}", if water == "plain" { "" } else { water })
    };
    format!(
        r##"<scene version="1.3"><project width="{w}" height="{h}" fps="10" duration="1" background="#101010"/>
      <materials>
        <material id="floorMat" baseColor="#B0B0B0" roughness="0.9"/>
        <material id="boxMat" baseColor="#C03020" roughness="0.8"/>
        <material id="sunMat" baseColor="#000000" roughness="1" emissive="#FFFFFF" emissiveStrength="{{emission}}"/>
        {water_material}
      </materials>
      <composition>
        <camera id="camera" {camera} renderer="pathtrace" pathSamples="{samples}" maxBounces="{bounces}" denoise="false"/>
        <object3D id="floor" primitive="plane" width="400" height="400" y="4" rotationX="-90" material="floorMat"/>
        <object3D id="box" primitive="box" width="4" height="3" depth="4" x="9" y="2.5" material="boxMat"/>
        {pane}{extra}
      </composition>
      <lights>{lights}</lights></scene>"##,
        w = size[0],
        h = size[1],
    )
}

const LOOKING_DOWN: &str = r#"x="0" y="-12" z="-20" pitch="-30" fov="45""#;
/// The open floor, clear of the box and the horizon, in a 320x180 frame.
const OPEN_FLOOR: [usize; 4] = [75, 15, 175, 65];
const SMALL: [u32; 2] = [320, 180];

/// How a brute-force comparison is sized. A discrete or integrated GPU renders 320x180 frames at
/// 1024 samples per pixel. A software adapter (llvmpipe) takes half an hour for that and, even at
/// the reduced size below, about ten minutes for the two comparisons (191 s and 428 s measured), so
/// there they run only when asked: `SR_BRUTE_FORCE=1` renders half-size frames (the same view, the
/// patch scaled with them) at 1024 samples, which the assertion allows because it compares the mean
/// of a patch of the open floor, whose noise is that of one pixel divided by the square root of the
/// pixels in the patch; `SR_BRUTE_FORCE=full` renders the full size and samples. A GPU always runs
/// them at full size, whatever the setting.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Brute {
    size: [u32; 2],
    floor: [usize; 4],
    samples: u32,
}

const FULL: Brute = Brute { size: SMALL, floor: OPEN_FLOOR, samples: 1024 };
const REDUCED: Brute = Brute { size: [160, 90], floor: [37, 7, 87, 32], samples: 1024 };

/// Mean and relative noise of the mean of a patch of luminance, from the pixels themselves: their
/// spread (coefficient of variation) over the square root of their number. A floor lit by one light
/// varies a little across the patch as well, which only makes this larger, so it is conservative.
fn patch_noise(px: &[[f32; 4]], width: usize, [x0, y0, x1, y1]: [usize; 4]) -> (f32, f32) {
    let values: Vec<f32> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| lum(px[y * width + x]))).collect();
    let n = values.len() as f32;
    let mean = values.iter().sum::<f32>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / (n - 1.0);
    (mean, variance.sqrt() / mean / n.sqrt())
}

/// The tolerance of a comparison against brute force: the one the test states, or four times the
/// noise of the two references' means when that is larger, so a smaller reference never loosens a
/// test below its stated tolerance and never fails on its own noise.
fn tolerance(stated: f32, wet_noise: f32, dry_noise: f32) -> f32 {
    stated.max(4.0 * wet_noise.hypot(dry_noise))
}

/// The sizing of a brute-force comparison for an adapter that is a software one or not, and the value of
/// `SR_BRUTE_FORCE`.
fn brute_force_for(software: bool, setting: Option<&str>) -> Option<Brute> {
    match (software, setting) {
        (false, _) => Some(FULL),
        (true, Some("full")) => Some(FULL),
        (true, Some(asked)) if !asked.is_empty() => Some(REDUCED),
        (true, _) => None,
    }
}

fn brute_force() -> Option<Brute> {
    let software = common::gpu().is_some_and(|g| g.info.device_type == wgpu::DeviceType::Cpu);
    let brute = brute_force_for(software, std::env::var("SR_BRUTE_FORCE").ok().as_deref());
    if brute.is_none() {
        eprintln!("skipping a brute-force comparison on a software adapter; SR_BRUTE_FORCE=1 runs it reduced, =full at full size");
    }
    brute
}

/// A light that exists only as emissive geometry (so brute-force paths find it) matching an analytic
/// one: the emission is set so the dry floor agrees with the analytic light's within a few percent.
fn emissive_sphere(centre: glam::Vec3, radius: f32) -> String {
    format!(
        r#"<object3D id="lamp" primitive="sphere" radius="{radius}" segments="48" x="{}" y="{}" z="{}" material="sunMat" castShadow="false"/>"#,
        centre.x, centre.y, centre.z
    )
}

const NO_LIGHT: &str = r##"<light id="off" type="ambient" color="#000000" intensity="0"/>"##;

/// What one comparison against brute force found over one region: the analytic ratio of wet over
/// dry, the brute-force one, and the relative noise of the brute-force ratio.
#[derive(Clone, Copy, Debug)]
struct Ratio {
    got: f32,
    want: f32,
    noise: f32,
}

/// The two regions a comparison looks at: the open floor, and the box standing on it (its vertical
/// faces and its top, picked out by their colour in the analytic wet frame).
#[derive(Clone, Copy, Debug)]
struct Wet {
    floor: Ratio,
    face: Ratio,
}

/// Mean and relative noise of the mean over the pixels at `indices`, from their spread.
fn region_noise(px: &[[f32; 4]], indices: &[usize]) -> (f32, f32) {
    let values: Vec<f32> = indices.iter().map(|i| lum(px[*i])).collect();
    let n = values.len() as f32;
    let mean = values.iter().sum::<f32>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / (n - 1.0);
    (mean, variance.sqrt() / mean / n.sqrt())
}

/// How much light a floor and a box under water keep against the same scene dry, under one light:
/// analytic light with the engine's shadow rays against brute force (an emissive copy of the light,
/// found by paths that refract out of the water), each as wet over dry so the two lights'
/// calibration cancels.
fn wet_over_dry(analytic: &str, emissive: &str, emission: f32, water: &str, bounces: u32) -> Option<Wet> {
    let brute = brute_force()?;
    let render = |water: &str, lights: &str, extra: &str, samples: u32, bounces: u32| {
        let xml = seabed(water, lights, extra, LOOKING_DOWN, brute.size, samples, bounces)
            .replace("{emission}", &emission.to_string());
        render(&xml)
    };
    let dry = render("", analytic, "", 128, 4)?;
    let wet = render(water, analytic, "", 128, 4)?;
    let dry_ref = render("", NO_LIGHT, emissive, brute.samples, bounces)?;
    let wet_ref = render(water, NO_LIGHT, emissive, brute.samples, bounces)?;
    let width = brute.size[0] as usize;
    let [x0, y0, x1, y1] = brute.floor;
    let floor: Vec<usize> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| y * width + x)).collect();
    // the box: red far above green in the analytic wet frame, away from the frame's edge
    let face: Vec<usize> =
        wet.px.iter().enumerate().filter(|(_, p)| p[0] > 2.0 * p[1] && p[0] > 0.01).map(|(i, _)| i).collect();
    assert!(face.len() > 150, "the box is seen: {} pixels", face.len());
    let ratio = |indices: &[usize]| {
        let mean = |r: &common::Rendered| region_noise(&r.px, indices);
        let (analytic_wet, analytic_dry) = (mean(&wet).0, mean(&dry).0);
        let ((ref_wet, wet_noise), (ref_dry, dry_noise)) = (mean(&wet_ref), mean(&dry_ref));
        Ratio { got: analytic_wet / analytic_dry, want: ref_wet / ref_dry, noise: wet_noise.hypot(dry_noise) }
    };
    Some(Wet { floor: ratio(&floor), face: ratio(&face) })
}

/// The sun reaches a floor under water: through the interface with the Fresnel loss, the change of
/// solid angle and the refracted direction, as brute-force paths see it (the analytic sun was
/// blocked by the water before: wet over dry was 0).
#[test]
fn the_sun_lights_a_floor_under_water_as_brute_force_does() {
    let (yaw, pitch, sun) = (-35.0, -50.0, 3.0);
    let analytic = format!(
        r##"<light id="sun" type="directional" color="#FFFFFF" yaw="{yaw}" pitch="{pitch}" intensity="{sun}" castShadow="true"/>"##
    );
    let (distance, radius) = (500.0, 60.0);
    let omega = std::f32::consts::PI * radius * radius / (distance * distance);
    let lamp = emissive_sphere(-shines(yaw, pitch) * distance, radius);
    let Some(wet) = wet_over_dry(&analytic, &lamp, sun / omega, "plain", 24) else { return };
    for (name, r) in [("floor", wet.floor), ("box", wet.face)] {
        let Ratio { got, want, noise } = r;
        eprintln!("sun {name}: wet over dry {got} against the brute force {want} (noise {noise})");
        assert!(want > 0.3, "{name}: the reference keeps light under the water: {want}");
        let limit = tolerance(0.08, noise, 0.0);
        assert!(
            (got / want - 1.0).abs() < limit,
            "{name}: wet over dry {got} against the brute force {want} (within {limit})"
        );
    }
}

/// A point light and a sphere light reach the floor under the water as brute force sees them.
/// The shadow ray is refracted at the interface point it crosses, the light's radiance is taken at
/// the straight distance: the error against brute force is measured, not zero.
#[test]
fn point_and_sphere_lights_reach_a_floor_under_water_as_brute_force_does() {
    let at = glam::Vec3::new(-10.0, -30.0, -10.0);
    let radius = 3.0;
    let lamp = emissive_sphere(at, radius);
    // 0.3 times the engine's 100-unit falloff: emission matching radiance at 100 units
    let emission = 0.3 * 100.0 * 100.0 / (std::f32::consts::PI * radius * radius);
    for (name, light) in [
        (
            "point",
            format!(
                r##"<light id="l" type="point" x="{}" y="{}" z="{}" intensity="0.3" castShadow="true"/>"##,
                at.x, at.y, at.z
            ),
        ),
        (
            "sphere",
            format!(
                r##"<light id="l" type="sphere-area" x="{}" y="{}" z="{}" radius="{radius}" intensity="0.3" castShadow="true"/>"##,
                at.x, at.y, at.z
            ),
        ),
    ] {
        let Some(wet) = wet_over_dry(&light, &lamp, emission, "plain", 24) else { return };
        for (region, r) in [("floor", wet.floor), ("box", wet.face)] {
            let Ratio { got, want, noise } = r;
            eprintln!("{name} {region}: wet over dry {got} against the brute force {want} (noise {noise})");
            assert!(want > 0.3, "{name} {region}: the reference keeps light under the water: {want}");
            let limit = tolerance(0.15, noise, 0.0);
            assert!(
                (got / want - 1.0).abs() < limit,
                "{name} {region}: wet over dry {got} against the brute force {want} (within {limit})"
            );
        }
    }
}

/// With the sun straight down, the camera straight down, one bounce (direct light only) and no
/// other light, the floor under absorbing water has a closed form: the view and the sun each cross
/// the interface (transmittance 1 - F at normal incidence) and the water (the attenuation colour at
/// its distance), the light also loses 1 / eta^2, and the floor reflects albedo / pi times (1 - F0).
#[test]
fn the_floor_under_absorbing_water_is_lit_as_the_closed_form_says() {
    let albedo = linear(0xBC);
    let attenuation = [0x80u8, 0xC0, 0xE0];
    let scene = r##"<scene version="1.3"><project width="16" height="16" fps="10" duration="1" background="#101010"/>
      <materials>
        <material id="waterMat" transmission="1" roughness="0.02" ior="1.333" doubleSided="true" attenuationColor="#80C0E0" attenuationDistance="4"/>
        <material id="floorMat" baseColor="#BCBCBC" roughness="1" specular="0"/>
      </materials>
      <composition>
        <camera id="camera" x="0" y="-20" z="0" pitch="-90" fov="10" renderer="pathtrace" pathSamples="256" maxBounces="1" denoise="false"/>
        <object3D id="floor" primitive="plane" width="400" height="400" y="4" rotationX="-90" material="floorMat"/>
        <object3D id="water" primitive="plane" width="400" height="400" y="0" rotationX="-90" material="waterMat"/>
      </composition>
      <lights><light id="sun" type="directional" color="#FFFFFF" pitch="-90" intensity="3" castShadow="true"/></lights></scene>"##;
    let Some(r) = render(scene) else { return };
    let transmitted = 1.0 - ((1.0 - 1.0 / 1.333f32) / (1.0 + 1.0 / 1.333)).powi(2);
    let floor_diffuse = 1.0 - 0.04; // the floor's Fresnel reflectance at normal incidence
    let centre = r.px[8 * 16 + 8];
    for (c, value) in attenuation.iter().enumerate() {
        let down = linear(*value);
        let want = transmitted * down * floor_diffuse * albedo / std::f32::consts::PI * 3.0 * transmitted * down
            / (1.333 * 1.333);
        assert!((centre[c] / want - 1.0).abs() < 0.04, "channel {c}: {} against {want}", centre[c]);
    }
}

/// The shadow of a box standing on a floor under water falls where the refracted sun puts it, not
/// where the straight direction would: a submerged tall box seen from straight above (orthographic,
/// so the camera's own refraction does not move anything).
#[test]
fn a_shadow_under_water_falls_where_the_refracted_sun_puts_it() {
    // sun 40 degrees from the vertical in air, 28.8 in water; it shines toward +z
    let scene = r##"<scene version="1.3"><project width="128" height="128" fps="10" duration="1" background="#101010"/>
      <materials>
        <material id="waterMat" transmission="1" roughness="0.02" ior="1.333" doubleSided="true"/>
        <material id="floorMat" baseColor="#B0B0B0" roughness="1" specular="0"/>
        <material id="boxMat" baseColor="#C03020" roughness="1" specular="0"/>
      </materials>
      <composition>
        <camera id="camera" projection="orthographic" x="0" y="-60" z="0" pitch="-90" renderer="pathtrace" pathSamples="4" maxBounces="1" denoise="false"/>
        <object3D id="floor" primitive="plane" width="400" height="400" y="24" rotationX="-90" material="floorMat"/>
        <object3D id="box" primitive="box" width="12" height="18" depth="12" y="15" material="boxMat"/>
        <object3D id="water" primitive="plane" width="400" height="400" y="0" rotationX="-90" material="waterMat"/>
      </composition>
      <lights><light id="sun" type="directional" color="#FFFFFF" yaw="0" pitch="-50" intensity="3" castShadow="true"/></lights></scene>"##;
    let Some(r) = render(scene) else { return };
    let at = |x: usize, y: usize| r.px[y * 128 + x];
    // the column through the box's centre: the box top is red, the floor beyond it is lit or in shadow
    let column: Vec<[f32; 4]> = (0..128).map(|y| at(64, y)).collect();
    let red = |p: &[f32; 4]| p[0] > 2.0 * p[1] && p[0] > 0.05;
    let rows: Vec<usize> = (0..128).filter(|y| red(&column[*y])).collect();
    let (first, last) = (*rows.first().expect("the box is seen"), *rows.last().unwrap());
    assert!(last - first >= 9 && last - first <= 14, "the box is 12 units wide: rows {first} to {last}");
    let lit = lum(column[2]).max(lum(column[125]));
    assert!(lit > 0.05, "lit floor {lit}");
    // shadow rows: dark floor next to the box, on whichever side the sun throws it
    let dark = |y: usize| lum(column[y]) < 0.2 * lit;
    let behind = (last + 1..128).take_while(|y| dark(*y)).count();
    let before = (0..first).rev().take_while(|y| dark(*y)).count();
    let length = behind.max(before) as f32;
    // the top edge of the box is 18 units above the floor: tan(28.8 degrees) * 18 = 9.9 refracted,
    // tan(40 degrees) * 18 = 15.1 straight
    assert!((length - 9.9).abs() < 1.5, "shadow length {length} (refracted 9.9, straight 15.1)");
}

/// A glass ball in air still casts a black shadow: only surfaces seen from inside the denser medium
/// get light through the surface above them.
#[test]
fn a_glass_ball_in_air_still_casts_its_black_shadow() {
    let scene = r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="1" background="#101010"/>
      <materials>
        <material id="glassMat" transmission="1" roughness="0.02" ior="1.5"/>
        <material id="floorMat" baseColor="#B0B0B0" roughness="1" specular="0"/>
      </materials>
      <composition>
        <camera id="camera" projection="orthographic" x="0" y="-60" z="0" pitch="-90" renderer="pathtrace" pathSamples="16" maxBounces="1" denoise="false"/>
        <object3D id="floor" primitive="plane" width="400" height="400" y="4" rotationX="-90" material="floorMat"/>
        <object3D id="ball" primitive="sphere" radius="6" segments="32" y="-2" material="glassMat"/>
      </composition>
      <lights><light id="sun" type="directional" color="#FFFFFF" pitch="-90" intensity="3" castShadow="true"/></lights></scene>"##;
    let Some(r) = render(scene) else { return };
    let lit = lum(r.px[2 * 64 + 2]);
    let under = lum(r.px[32 * 64 + 32]);
    assert!(lit > 0.1 && under < 0.05 * lit, "under the ball {under}, open floor {lit}");
}

/// Dome light through the water arrives by paths that refract out; four bounces already agree with
/// twenty-four (total internal reflection returns the light and it is spent within about eight).
#[test]
fn dome_light_through_water_does_not_depend_on_the_bounce_limit() {
    let dome = r##"<light id="sky" type="dome" environment="gray.png" environmentVisible="true" intensity="1"/>"##;
    let mean = |bounces: u32| {
        let xml = seabed("plain", dome, "", LOOKING_DOWN, SMALL, 256, bounces).replace("{emission}", "1");
        render(&xml).map(|r| patch(&r, OPEN_FLOOR))
    };
    let (Some(few), Some(many)) = (mean(4), mean(24)) else { return };
    assert!((few / many - 1.0).abs() < 0.03, "4 bounces {few}, 24 bounces {many}");
}

/// A transmissive object that no path touches changes nothing: the scene keeps its picture although
/// its pipeline now tracks media (a ball below the floor, which no path can reach).
#[test]
fn an_untouched_transmissive_object_leaves_the_picture_alone() {
    let lights = r##"<light id="sun" type="directional" color="#FFFFFF" yaw="-35" pitch="-50" intensity="3" castShadow="true"/><light id="sky" type="dome" environment="gray.png" environmentVisible="true" intensity="1"/>"##;
    let ball = r##"<object3D id="ball" primitive="sphere" radius="3" segments="16" y="40" material="waterMat"/>"##;
    let plain = seabed("", lights, "", LOOKING_DOWN, SMALL, 16, 4).replace("{emission}", "1");
    // the pane exists only as a material here: the ball under the floor uses it
    let with_ball = seabed("plain", lights, ball, LOOKING_DOWN, SMALL, 16, 4)
        .replace("{emission}", "1")
        .replace(r#"<object3D id="water" primitive="plane" width="400" height="400" y="0" rotationX="-90" material="waterMat"/>"#, "");
    let (Some(a), Some(b)) = (render(&plain), render(&with_ball)) else { return };
    let db = common::psnr(&a.px, &b.px);
    assert!(db > 55.0, "the picture moved: {db:.1} dB");
}

/// A sea with steep waves over a floor, lit by the sun and the dome; `waves` is the ocean's wave
/// element (empty for a flat sea) and `sea` false leaves the floor dry.
fn wavy_sea(sea: bool, waves: &str) -> String {
    let ocean = if sea {
        format!(
            r##"<ocean id="sea" width="48" depth="48" cellSize="1" bottomDepth="6" material="waterMat" dt="0.05">{waves}</ocean>"##
        )
    } else {
        String::new()
    };
    format!(
        r##"<scene version="1.3"><project width="160" height="90" fps="10" duration="1" background="#101010"/>
      <materials>
        <material id="waterMat" baseColor="#FFFFFF" roughness="0.02" ior="1.333" doubleSided="true" transmission="1"/>
        <material id="floorMat" baseColor="#B0B0B0" roughness="0.9"/>
      </materials>
      <composition>
        <camera id="camera" x="0" y="-14" z="-20" target="floor" fov="45" renderer="pathtrace" pathSamples="64" maxBounces="4" denoise="false"/>
        <object3D id="floor" primitive="plane" width="200" height="200" y="6" rotationX="-90" material="floorMat"/>
        {ocean}
      </composition>
      <lights>
        <light id="sun" type="directional" color="#FFFFFF" yaw="-35" pitch="-50" intensity="3" castShadow="true"/>
        <light id="sky" type="dome" environment="gray.png" environmentVisible="true" intensity="1"/>
      </lights></scene>"##
    )
}

/// The real sea is not flat. Under steep waves (slopes up to about 40 degrees), the floor lit
/// through the refracted shadow ray never gets brighter than the dry floor under the same light, per
/// channel, its mean stays near the flat sea's, and no pixel flares.
#[test]
fn a_floor_under_steep_waves_gains_no_energy_and_no_fireflies() {
    let steep = r#"<wave wavelength="10" amplitude="1.2" direction="25" phase="0"/>"#;
    let Some(gpu) = common::gpu() else { return };
    let frame = |xml: &str| {
        let doc =
            sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(common::fixtures()) })
                .unwrap();
        let shot = common::render_times_on(gpu.clone(), &doc, &[0.3]).unwrap();
        assert!(shot.stats.errors.is_empty(), "{:?}", shot.stats.errors);
        shot
    };
    let (dry, flat, wavy) = (frame(&wavy_sea(false, "")), frame(&wavy_sea(true, "")), frame(&wavy_sea(true, steep)));
    let region = [40usize, 30, 120, 80];
    let mean = |r: &common::Rendered, c: usize| {
        let mut sum = 0.0;
        for y in region[1]..region[3] {
            for x in region[0]..region[2] {
                sum += r.px[y * 160 + x][c];
            }
        }
        sum / ((region[2] - region[0]) * (region[3] - region[1])) as f32
    };
    for c in 0..3 {
        let (d, f, w) = (mean(&dry, c), mean(&flat, c), mean(&wavy, c));
        assert!(w <= d * 1.0, "channel {c}: the wet floor is brighter than the dry one: {w} against {d}");
        assert!((w / f - 1.0).abs() < 0.25, "channel {c}: steep waves {w} against the flat sea {f}");
    }
    let brightest = |r: &common::Rendered| r.px.iter().map(|p| lum(*p)).fold(0.0f32, f32::max);
    assert!(r_finite(&wavy), "no pixel is NaN or infinite");
    assert!(
        brightest(&wavy) <= 1.25 * brightest(&dry),
        "a flare: {} against the dry {}",
        brightest(&wavy),
        brightest(&dry)
    );
}

fn r_finite(r: &common::Rendered) -> bool {
    r.px.iter().all(|p| p.iter().all(|c| c.is_finite()))
}

/// Tiles must not change a pixel of a scene whose floor is lit through water: a diffuse wall inside
/// an absorbing glass block, seen through it, lit by a light outside.
#[test]
fn tiled_frames_equal_whole_frames_with_light_through_glass() {
    use glam::{Mat4, Vec3};
    use sr_3d::camera::{resolve, CameraParams};
    use sr_3d::{prim, MaterialParams};
    use sr_gpu::pathtrace::{self, PathOpts, PtGpu, PtInputs};
    use sr_gpu::three::{Draw3, Light3, LightKind, MeshSrc, Scene3, ThreeEngine};

    let Some(g) = common::gpu() else { return };
    let eng = ThreeEngine::new(g.device.clone(), g.queue.clone());
    let black = eng.upload_f16([1, 1], &[[0.0; 4]]).create_view(&Default::default());
    let smp = g.device.create_sampler(&Default::default());
    let input = PtInputs { env: &black, backdrop: None, black: &black, sampler: &smp };
    let size = [200, 120];
    let block = prim::cuboid(60.0, 60.0, 60.0);
    let wall = prim::cuboid(30.0, 30.0, 2.0);
    let draw = |mesh: &sr_3d::Primitive, at: Vec3, material: MaterialParams| Draw3 {
        mesh: MeshSrc::Cached(eng.upload_mesh(&mesh.vertices, &mesh.indices)),
        model: Mat4::from_translation(at),
        material,
        maps: Default::default(),
        opacity: 1.0,
        cast_shadow: true,
        receive_shadow: true,
    };
    let glass = MaterialParams {
        transmission: 1.0,
        ior: 1.333,
        roughness: 0.02,
        attenuation_color: [0.8, 0.9, 1.0],
        attenuation_distance: 40.0,
        double_sided: true,
        ..Default::default()
    };
    let matte = MaterialParams { base_color: [0.7, 0.7, 0.7, 1.0], roughness: 1.0, ..Default::default() };
    let scene = Scene3 {
        cam: resolve(
            &CameraParams { position: Some(Vec3::new(0.0, 0.0, -90.0)), ..Default::default() },
            size[0] as f32,
            size[1] as f32,
        ),
        clip_fix: Mat4::IDENTITY,
        size,
        exposure: 1.0,
        dof: None,
        lens_k1: 0.0,
        draws: vec![draw(&block, Vec3::ZERO, glass), draw(&wall, Vec3::new(0.0, 0.0, 10.0), matte)],
        lights: vec![Light3 {
            kind: LightKind::Directional,
            pos: Vec3::ZERO,
            dir: Vec3::new(0.3, 0.4, 0.8).normalize(),
            right: Vec3::X,
            color: Vec3::splat(std::f32::consts::PI),
            range: 0.0,
            falloff: 2.0,
            cos_outer: 0.0,
            cos_inner: 0.0,
            cast_shadow: true,
            softness: 0.0,
            bias: 0.0005,
            map_size: 128,
            size: [0.0; 3],
            ies: None,
            affects_diffuse: true,
            affects_specular: true,
            contact: 0.0,
        }],
        env: None,
        splats: Vec::new(),
        volumes: Vec::new(),
        encode_srgb: false,
        ao: None,
        ssr: false,
        path: Some(PathOpts { samples: 4, bounces: 2, denoise: false }),
        geodesic: None,
    };
    assert!(pathtrace::limit_note(&scene, &g.device.limits()).is_none());
    let data = pathtrace::build(&scene);
    let render = |tile_bytes: Option<u64>| {
        let mut pt = PtGpu::new(&g.device, wgpu::TextureFormat::Rgba16Float);
        if let Some(bytes) = tile_bytes {
            pt.limit_buffers(bytes).unwrap();
        }
        let target = eng.target(size);
        let mut enc = g.device.create_command_encoder(&Default::default());
        pathtrace::render(
            &pt,
            &g.device,
            &mut enc,
            &scene,
            &data,
            scene.path.unwrap(),
            &input,
            &target.create_view(&Default::default()),
        );
        g.queue.submit([enc.finish()]);
        eng.read(&target)
    };
    let whole = render(None);
    let wall_pixel = whole[60 * 200 + 100];
    assert!(lum(wall_pixel) > 0.02, "the wall inside the glass is lit through it: {wall_pixel:?}");
    let tiled = render(Some(1_200_000));
    assert_eq!(whole, tiled, "tiles must not change a pixel");
}

/// The reduced reference never loosens a test below the tolerance it states and never fails on its
/// own noise: the tolerance is the stated one until four times the references' noise exceeds it.
#[test]
fn the_brute_force_tolerance_follows_the_noise_of_its_references() {
    // the reduced frame is the full view at half the size: same floor, a quarter of the pixels
    let pixels = |b: Brute| (b.floor[2] - b.floor[0]) * (b.floor[3] - b.floor[1]);
    assert_eq!(REDUCED.size, [FULL.size[0] / 2, FULL.size[1] / 2]);
    assert_eq!(pixels(REDUCED) * 4, pixels(FULL));
    assert_eq!(FULL.samples, 1024);
    assert_eq!(tolerance(0.08, 0.0, 0.0), 0.08);
    assert_eq!(tolerance(0.08, 0.01, 0.01), 0.08);
    assert!((tolerance(0.08, 0.03, 0.04) - 0.2).abs() < 1e-6, "4 x hypot(0.03, 0.04)");
    // noise from the pixels: a patch alternating 0.9 and 1.1 has a spread of 0.1 and a mean of 1
    let mut px = vec![[0.0f32; 4]; 16 * 16];
    for (i, p) in px.iter_mut().enumerate() {
        let v = if i % 2 == 0 { 0.9 } else { 1.1 };
        *p = [v; 4];
    }
    let (mean, noise) = patch_noise(&px, 16, [0, 0, 16, 16]);
    assert!((mean - 1.0).abs() < 1e-5 && (noise - 0.1 / 16.0).abs() < 1e-3, "{mean} {noise}");
}

/// A software adapter takes about ten minutes for the two brute-force comparisons even at the reduced size
/// (measured on llvmpipe: 191 s and 428 s), so they run there only when asked; a GPU always runs them.
#[test]
fn the_brute_force_comparisons_run_on_a_gpu_and_on_a_software_adapter_only_when_asked() {
    assert_eq!(brute_force_for(false, None), Some(FULL), "a GPU renders the full size");
    assert_eq!(brute_force_for(false, Some("1")), Some(FULL), "and the setting does not reduce it");
    assert_eq!(brute_force_for(true, None), None, "a software adapter skips them unless asked");
    assert_eq!(brute_force_for(true, Some("")), None, "an empty setting is not asking");
    assert_eq!(brute_force_for(true, Some("1")), Some(REDUCED), "asked, it renders the reduced size");
    assert_eq!(brute_force_for(true, Some("full")), Some(FULL), "or the full size on request");
}
