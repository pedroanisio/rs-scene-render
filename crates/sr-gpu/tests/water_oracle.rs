//! Light under water against an oracle that is not the shader: the irradiance and the radiance
//! that physics gives a diffuse surface below a flat refracting surface, computed on the CPU.

mod common;

use glam::Vec3;

/// A sun the engine's light convention shines along: yaw and pitch in degrees.
fn shines(yaw: f32, pitch: f32) -> Vec3 {
    (glam::Mat4::from_rotation_y(yaw.to_radians()) * glam::Mat4::from_rotation_x(pitch.to_radians()))
        .transform_vector3(Vec3::Z)
}

/// Snell's law for a ray of direction `i` meeting a surface of normal `n` (against `i`), going from a
/// medium of index n1 to one of index n2.
fn refract(i: Vec3, n: Vec3, n1: f32, n2: f32) -> Vec3 {
    let eta = n1 / n2;
    let cosi = -i.dot(n);
    let k = 1.0 - eta * eta * (1.0 - cosi * cosi);
    assert!(k >= 0.0, "total internal reflection");
    eta * i + (eta * cosi - k.sqrt()) * n
}

/// The exact Fresnel reflectance of an unpolarised ray at incidence cosine `cosi`, from index n1 to n2.
fn reflectance(cosi: f32, n1: f32, n2: f32) -> f32 {
    let sint = n1 / n2 * (1.0 - cosi * cosi).max(0.0).sqrt();
    if sint >= 1.0 {
        return 1.0;
    }
    let cost = (1.0 - sint * sint).sqrt();
    let rs = ((n1 * cosi - n2 * cost) / (n1 * cosi + n2 * cost)).powi(2);
    let rp = ((n1 * cost - n2 * cosi) / (n1 * cost + n2 * cosi)).powi(2);
    0.5 * (rs + rp)
}

fn linear(srgb: u8) -> f32 {
    let c = f32::from(srgb) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

const ETA: f32 = 1.333;

/// A diffuse wall of colour #CC2626 standing in water 16 units deep, seen by an orthographic camera
/// from above the surface, under a sun (the only light). The wall is vertical: the surface under
/// the water that the horizontal floor of the other tests does not exercise.
fn wall_in_water(sun: &str, water: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="128" height="128" fps="10" duration="1" background="#101010"/>
      <materials>
        <material id="wallMat" baseColor="#CC2626" roughness="1" specular="0"/>
        <material id="floorMat" baseColor="#808080" roughness="1" specular="0"/>
        <material id="waterMat" baseColor="#FFFFFF" roughness="0.02" ior="{ETA}" doubleSided="true" transmission="1"/>
      </materials>
      <composition>
        <camera id="camera" projection="orthographic" x="0" y="-12" z="-20" pitch="-30" renderer="pathtrace" pathSamples="512" maxBounces="1" denoise="false"/>
        <object3D id="floor" primitive="plane" width="400" height="400" y="20" rotationX="-90" material="floorMat"/>
        <object3D id="wall" primitive="plane" width="80" height="16" x="0" y="12" z="20" material="wallMat"/>
        {water}
      </composition>
      <lights>{sun}</lights></scene>"##
    )
}

const WATER: &str =
    r##"<object3D id="water" primitive="plane" width="400" height="400" y="0" rotationX="-90" material="waterMat"/>"##;
const SUN: &str =
    r##"<light id="sun" type="directional" color="#FFFFFF" yaw="-35" pitch="-50" intensity="3" castShadow="true"/>"##;

fn render(scene: &str) -> Option<common::Rendered> {
    let gpu = common::gpu()?;
    let doc =
        sr_model::load_str(scene, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(common::fixtures()) })
            .unwrap_or_else(|e| panic!("{e:?}"));
    let shot = common::render_times_on(gpu, &doc, &[0.0]).unwrap();
    assert!(shot.stats.errors.is_empty(), "{:?}", shot.stats.errors);
    Some(shot)
}

/// Mean of the red channel over the pixels that are wall (red far above green).
fn wall_red(r: &common::Rendered) -> (f32, usize) {
    let red: Vec<f32> = r.px.iter().filter(|p| p[0] > 0.004 && p[0] > 8.0 * p[1]).map(|p| p[0]).collect();
    (red.iter().sum::<f32>() / red.len().max(1) as f32, red.len())
}

/// What the camera sees of the wall: the camera ray refracts into the water (the share the surface
/// transmits), the wall reflects the irradiance that the sun's beam brings through the surface
/// (its power per area of the interface is T cos(theta_air); inside it spreads over a cross-section
/// of cos(theta_water), and the wall catches the part along its normal), and the water is denser
/// than the air by ETA, so the radiance seen from the air is ETA^2 smaller than the radiance in the
/// water.
fn expected_wall_red(rad: f32) -> f32 {
    let up = Vec3::new(0.0, -1.0, 0.0);
    let to_sun = -shines(-35.0, -50.0).normalize();
    let cos_air = to_sun.dot(up);
    let travel = refract(-to_sun, up, 1.0, ETA);
    let to_sun_in_water = -travel;
    let cos_water = to_sun_in_water.dot(up);
    let wall_normal = Vec3::new(0.0, 0.0, -1.0);
    let beam = (1.0 - reflectance(cos_air, 1.0, ETA)) * cos_air / cos_water * to_sun_in_water.dot(wall_normal).max(0.0);
    // the camera's ray, pitched 30 degrees below the horizon
    let eye = Vec3::new(0.0, 0.5, 0.866_025_4);
    let view = -refract(eye, up, 1.0, ETA);
    let camera_share = 1.0 - reflectance(eye.dot(-up), 1.0, ETA);
    // the wall's diffuse lobe keeps what its own Fresnel term does not reflect
    let h = (to_sun_in_water + view).normalize();
    let f = 0.04 + 0.96 * (1.0 - view.dot(h).clamp(0.0, 1.0)).powi(5);
    camera_share * (1.0 - f) * linear(0xCC) / std::f32::consts::PI * rad * beam / (ETA * ETA)
}

/// A vertical wall under water is lit by the sun as physics says: the cosine at the wall (the
/// horizontal floor of the other tests has the same cosine as the interface and cannot see it) and
/// the change of the beam's cross-section at the surface both enter.
#[test]
fn a_vertical_wall_under_water_is_lit_as_the_oracle_says() {
    let Some(r) = render(&wall_in_water(SUN, WATER)) else { return };
    let (got, pixels) = wall_red(&r);
    assert!(pixels > 400, "the wall is seen: {pixels} pixels");
    let want = expected_wall_red(3.0);
    eprintln!("wall red {got} against the oracle {want}");
    assert!((got / want - 1.0).abs() < 0.04, "wall under water: {got} against the oracle {want}");
}

/// A light that does not cast shadows (the default of `light@castShadow`) still reaches a surface
/// under water through the interface: only the two blocker tests are skipped, not the refraction,
/// the Fresnel loss or the change of the beam's cross-section.
#[test]
fn a_light_without_shadows_lights_a_wall_under_water_as_one_with() {
    let unshadowed = SUN.replace(r#" castShadow="true""#, "");
    assert!(!unshadowed.contains("castShadow"));
    let Some(r) = render(&wall_in_water(&unshadowed, WATER)) else { return };
    let (got, pixels) = wall_red(&r);
    assert!(pixels > 400, "the wall is seen: {pixels} pixels");
    let want = expected_wall_red(3.0);
    eprintln!("wall red without castShadow {got} against the oracle {want}");
    assert!((got / want - 1.0).abs() < 0.04, "wall under water, no shadows: {got} against the oracle {want}");
}
