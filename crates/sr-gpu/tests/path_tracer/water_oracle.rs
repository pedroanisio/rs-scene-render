//! Light under water against an oracle that is not the shader: the irradiance and the radiance
//! that physics gives a diffuse surface below a flat refracting surface, computed on the CPU.

use super::common;

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
/// (8 units high, so that all its rays start on the same side of the surface) under a sun (the only
/// light). The wall is vertical: the surface under
/// the water that the horizontal floor of the other tests does not exercise.
fn wall_in_water(sun: &str, water: &str) -> String {
    wall_scene(sun, water, r#"x="0" y="-12" z="-20" pitch="-30" orthoHeight="8""#)
}

fn wall_scene(sun: &str, water: &str, camera: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="128" height="128" fps="10" duration="1" background="#101010"/>
      <materials>
        <material id="wallMat" baseColor="#CC2626" roughness="1" specular="0"/>
        <material id="floorMat" baseColor="#808080" roughness="1" specular="0"/>
        <material id="waterMat" baseColor="#FFFFFF" roughness="0.02" ior="{ETA}" doubleSided="true" transmission="1"/>
      </materials>
      <composition>
        <camera id="camera" projection="orthographic" {camera} renderer="pathtrace" pathSamples="512" maxBounces="1" denoise="false"/>
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

/// The radiance leaving the diffuse wall toward a viewer in the water, for a sun of irradiance
/// `rad` on a surface facing it: the sun's beam power per area of the interface is T cos(theta_air),
/// inside it spreads over a cross-section of cos(theta_water), and the wall catches the part along
/// its normal; the wall's diffuse lobe keeps what its own Fresnel term does not reflect. `view` is the
/// direction from the wall to the viewer, in the water.
fn wall_radiance_in_water(rad: f32, view: Vec3, up: Vec3) -> f32 {
    let to_sun = -shines(-35.0, -50.0).normalize();
    let cos_air = to_sun.dot(up);
    let travel = refract(-to_sun, up, 1.0, ETA);
    let to_sun_in_water = -travel;
    let cos_water = to_sun_in_water.dot(up);
    let wall_normal = Vec3::new(0.0, 0.0, -1.0);
    let beam = (1.0 - reflectance(cos_air, 1.0, ETA)) * cos_air / cos_water * to_sun_in_water.dot(wall_normal).max(0.0);
    let h = (to_sun_in_water + view).normalize();
    let f = 0.04 + 0.96 * (1.0 - view.dot(h).clamp(0.0, 1.0)).powi(5);
    (1.0 - f) * linear(0xCC) / std::f32::consts::PI * rad * beam
}

/// What a camera above the water sees of the wall: its ray refracts into the water (the share the
/// surface transmits), and the water is denser than the air by ETA, so the radiance seen from the
/// air is ETA^2 smaller than the radiance in the water.
fn expected_wall_red(rad: f32) -> f32 {
    expected_wall_red_through(rad, Vec3::new(0.0, -1.0, 0.0))
}

/// The same for a surface whose normal (on the air side) is `up`: flat, but tilted.
fn expected_wall_red_through(rad: f32, up: Vec3) -> f32 {
    // the camera's ray, pitched 30 degrees below the horizon
    let eye = Vec3::new(0.0, 0.5, 0.866_025_4);
    let view = -refract(eye, up, 1.0, ETA);
    let camera_share = 1.0 - reflectance(eye.dot(-up), 1.0, ETA);
    camera_share * wall_radiance_in_water(rad, view, up) / (ETA * ETA)
}

/// What a camera inside the water sees of the wall: the radiance in the water, with no interface
/// between them. Its ray is pitched 10 degrees below the horizontal.
fn expected_wall_red_from_water(rad: f32) -> f32 {
    let pitch = 10f32.to_radians();
    let view = -Vec3::new(0.0, pitch.sin(), pitch.cos());
    wall_radiance_in_water(rad, view, Vec3::new(0.0, -1.0, 0.0))
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

/// A camera inside a large water box (800 by 200 by 800, so that the ray meets the middle of the
/// faces) looks up at the top face at 45 degrees from the vertical, and
/// above it an emissive plane of radiance 1. What the ray meets at the face is the exit from the
/// denser medium: the share it transmits is 1 - R with the exact Fresnel reflectance R of the dense
/// side at 45 degrees (0.14, the critical angle is 48.6 degrees), not the 0.02 that the Schlick
/// term gives when it is fed the cosine of the dense side. What the face reflects goes down to
/// the box's other faces, which it meets at the same 45 degrees, and a second pass adds R^2 of
/// it. The camera is in the water, so the radiance it sees is ETA^2 that of the lamp.
#[test]
fn a_ray_leaving_water_at_45_degrees_is_reflected_as_fresnel_says() {
    let scene = format!(
        r##"<scene version="1.3"><project width="16" height="16" fps="10" duration="1" background="#000000"/>
      <materials>
        <material id="waterMat" baseColor="#FFFFFF" roughness="0.02" ior="{ETA}" doubleSided="true" transmission="1"/>
        <material id="lampMat" baseColor="#000000" roughness="1" specular="0" emissive="#FFFFFF" emissiveStrength="1"/>
      </materials>
      <composition>
        <camera id="camera" x="0" y="0" z="0" pitch="45" fov="2" renderer="pathtrace" pathSamples="2048" maxBounces="1" denoise="false"/>
        <object3D id="tank" primitive="box" width="800" height="200" depth="800" material="waterMat"/>
        <object3D id="lamp" primitive="plane" width="4000" height="4000" y="-400" rotationX="-90" material="lampMat"/>
      </composition>
      <lights><light id="off" type="ambient" color="#000000" intensity="0"/></lights></scene>"##
    );
    let Some(r) = render(&scene) else { return };
    let centre: f32 =
        (6..10).flat_map(|y| (6..10).map(move |x| (x, y))).map(|(x, y)| r.px[y * 16 + x][1]).sum::<f32>() / 16.0;
    let r45 = reflectance(std::f32::consts::FRAC_1_SQRT_2, ETA, 1.0);
    assert!((r45 - 0.1395).abs() < 1e-3, "the oracle: {r45}");
    // the radiance in the water is ETA^2 that of the air it looks at
    let want = ETA * ETA * (1.0 - r45) * (1.0 + r45 * r45);
    eprintln!("radiance through the exit {centre} against the oracle {want}");
    assert!((centre / want - 1.0).abs() < 0.03, "exit at 45 degrees: {centre} against the oracle {want}");
}

/// The camera is under the water: it starts inside the denser medium, so the surfaces it sees are
/// lit through the interface like those a camera in the air sees, and no interface lies between it
/// and the wall: the radiance it sees is the radiance in the water, ETA^2 more than the camera in
/// the air sees of the same wall.
#[test]
fn a_camera_under_the_water_sees_a_wall_lit_as_the_oracle_says() {
    let Some(r) = render(&wall_scene(SUN, WATER, r#"x="0" y="6" z="-20" pitch="-10" orthoHeight="8""#)) else { return };
    let (got, pixels) = wall_red(&r);
    assert!(pixels > 400, "the wall is seen: {pixels} pixels");
    let want = expected_wall_red_from_water(3.0);
    eprintln!("wall red from under the water {got} against the oracle {want}");
    assert!((got / want - 1.0).abs() < 0.04, "wall from under the water: {got} against the oracle {want}");
}

/// A flat water surface tilted by 15 degrees about the x axis is a flat interface with another
/// normal: the same oracle with that normal applies, for the camera's ray, the sun's beam and the
/// wall. The refraction point of the sun's ray from a wall point is still found.
#[test]
fn a_vertical_wall_under_a_tilted_surface_is_lit_as_the_oracle_says() {
    let tilted = WATER.replace(r#"rotationX="-90""#, r#"rotationX="-75""#);
    let Some(r) = render(&wall_in_water(SUN, &tilted)) else { return };
    let (got, pixels) = wall_red(&r);
    assert!(pixels > 400, "the wall is seen: {pixels} pixels");
    let theta = (-75f32).to_radians();
    let normal = Vec3::new(0.0, theta.sin(), -theta.cos());
    let want = expected_wall_red_through(3.0, normal);
    eprintln!("wall red under the tilted surface {got} against the oracle {want}");
    assert!((got / want - 1.0).abs() < 0.05, "wall under a tilted surface: {got} against the oracle {want}");
}

// ---- a ridge in the surface: two flat facets that meet over the floor

/// The water as two facets that meet in a ridge along the x axis at z = 0 and slope down from it at
/// `theta` degrees: the right facet (z > 0) has the normal (0, -cos, sin), the left one (z < 0)
/// (0, -cos, -sin). `facet` is 200 units wide along the slope.
fn ridge(theta: f32) -> String {
    let t = theta.to_radians();
    let half = 100.0;
    let (y, z) = (half * t.sin(), half * t.cos());
    format!(
        r##"<object3D id="waterRight" primitive="plane" width="400" height="200" y="{y}" z="{z}" rotationX="{}" material="waterMat"/>
        <object3D id="waterLeft" primitive="plane" width="400" height="200" y="{y}" z="{}" rotationX="{}" material="waterMat"/>"##,
        -(90.0 + theta),
        -z,
        -(90.0 - theta)
    )
}

/// A diffuse floor 20 units below the ridge, seen from under the water by a camera that looks
/// straight down over 30 units of z, lit by the sun alone.
fn floor_under_a_ridge(theta: f32, sun: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="128" height="128" fps="10" duration="1" background="#101010"/>
      <materials>
        <material id="floorMat" baseColor="#808080" roughness="1" specular="0"/>
        <material id="waterMat" baseColor="#FFFFFF" roughness="0.02" ior="{ETA}" doubleSided="true" transmission="1"/>
      </materials>
      <composition>
        <camera id="camera" projection="orthographic" x="0" y="12" z="10" pitch="-90" orthoHeight="30" renderer="pathtrace" pathSamples="256" maxBounces="1" denoise="false"/>
        <object3D id="floor" primitive="plane" width="400" height="400" y="20" rotationX="-90" material="floorMat"/>
        {}
      </composition>
      <lights>{sun}</lights></scene>"##,
        ridge(theta)
    )
}

/// The irradiance factors the floor point at `z` (x does not matter) can get from the sun through
/// the ridge, by the exact planar solution on the CPU: each facet refracts the sun into one
/// direction (the sun is directional), the ray from the point along it meets the facet's plane
/// somewhere, and that facet lights the point if the meeting is on its own side of the ridge. Each
/// solution is T cos(theta_air) / cos(theta_water) times the floor's cosine with the refracted
/// direction, with that direction (for the floor's Fresnel term). Over some floor two facets reach
/// it, over some none.
fn ridge_irradiance(theta: f32, z: f32) -> Vec<(f32, Vec3)> {
    let t = theta.to_radians();
    let to_sun = -shines(-35.0, -50.0).normalize();
    let p = Vec3::new(0.0, 20.0, z);
    let mut found = Vec::new();
    for (normal, side) in [(Vec3::new(0.0, -t.cos(), t.sin()), 1.0f32), (Vec3::new(0.0, -t.cos(), -t.sin()), -1.0)] {
        let cos_air = to_sun.dot(normal);
        if cos_air <= 0.0 {
            continue;
        }
        let toward_sun = -refract(-to_sun, normal, 1.0, ETA);
        // the facet's plane: through the ridge (0, 0, 0) with this normal
        let along = toward_sun.dot(normal);
        let reach = -p.dot(normal) / along;
        let meets = p + toward_sun * reach;
        if reach > 0.0 && meets.z * side >= 0.0 {
            let beam = (1.0 - reflectance(cos_air, 1.0, ETA)) * cos_air / along;
            found.push((beam * toward_sun.dot(Vec3::new(0.0, -1.0, 0.0)), toward_sun));
        }
    }
    found
}

/// The floor under a ridge in the surface is lit row by row as the facets' exact solution says: the
/// light that reaches a floor point is refracted at the point of the surface where the refracted ray
/// leaves toward the sun, found by following the ray from the floor, not at the point the straight
/// ray toward the sun happens to meet. Over the floor between the two, the first guess is on one
/// facet and the answer on the other. Where two facets reach the same point each sample takes one
/// of them (one shadow ray a sample), so the pixel lies between the two; where none does the floor
/// is dark.
#[test]
fn a_floor_under_a_ridge_is_lit_by_the_facet_whose_refraction_reaches_the_sun() {
    let theta = 20.0;
    let Some(r) = render(&floor_under_a_ridge(theta, SUN)) else { return };
    let radiance = |(beam, dir): (f32, Vec3)| {
        let view = Vec3::new(0.0, -1.0, 0.0);
        let h = (dir + view).normalize();
        let f = 0.04 + 0.96 * (1.0 - view.dot(h).clamp(0.0, 1.0)).powi(5);
        (1.0 - f) * linear(0x80) / std::f32::consts::PI * 3.0 * beam
    };
    let (mut single, mut double, mut none) = (0, 0, 0);
    // image row k covers z = 25 - (k + 0.5) * 30 / 128 (the camera looks down with +z up the image)
    for k in (0..128).step_by(2) {
        let z = 25.0 - (k as f32 + 0.5) * 30.0 / 128.0;
        let got: f32 = (32..96).map(|x| r.px[k * 128 + x][0]).sum::<f32>() / 64.0;
        let solutions: Vec<f32> = ridge_irradiance(theta, z).into_iter().map(radiance).collect();
        match solutions.as_slice() {
            [] => {
                none += 1;
                assert!(got < 0.003, "z {z:.2}: no facet reaches the sun, the floor is {got}");
            }
            [want] => {
                single += 1;
                assert!((got / want - 1.0).abs() < 0.04, "z {z:.2}: {got} against the facet's {want}");
            }
            [a, b] => {
                // each sample takes one facet, so the pixel is between the two
                double += 1;
                let (low, high) = (a.min(*b), a.max(*b));
                assert!(got > 0.96 * low && got < 1.04 * high, "z {z:.2}: {got} outside the facets' {low} to {high}");
            }
            _ => unreachable!(),
        }
    }
    assert!(single > 20 && double > 5, "the rows cover both kinds: {single} single, {double} double, {none} dark");
}

// ---- smoke in the water

/// A cache whose density is 1 everywhere (the volume asset's bounds cut a slab out of it).
fn uniform_cache() -> std::path::PathBuf {
    let mut cache = sr_volume::Volume::new();
    cache.insert("density", sr_volume::SparseGrid::new(sr_volume::Transform::identity(), 1.0, 0).unwrap()).unwrap();
    let path = common::fixtures().join("uniform_slab.srvol");
    cache.write(std::fs::File::create(&path).unwrap()).unwrap();
    path
}

const SLAB_EXTINCTION: f32 = 0.4;

/// A diffuse floor under water, with a smoke slab in the water between them that only absorbs
/// (no emission, albedo 0) 1.5 units thick, and the sun; `slab` false leaves the water clear.
/// Seen by the camera of the wall tests, from above the water.
fn floor_through_smoke(slab: bool) -> String {
    let cache = uniform_cache();
    let smoke = if slab {
        format!(
            r##"<object3D id="smoke" primitive="volume" volume="slab"><medium extinction="{SLAB_EXTINCTION}" albedo="#000000" stepSize="0.05" maxSteps="4096"/></object3D>"##
        )
    } else {
        String::new()
    };
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="1" background="#101010"/>
      <assets><volume id="slab" src="{}" boundsMinX="-25" boundsMinY="2" boundsMinZ="-30" boundsMaxX="25" boundsMaxY="3.5" boundsMaxZ="40"/></assets>
      <materials>
        <material id="floorMat" baseColor="#808080" roughness="1" specular="0"/>
        <material id="waterMat" baseColor="#FFFFFF" roughness="0.02" ior="{ETA}" doubleSided="true" transmission="1"/>
      </materials>
      <composition>
        <camera id="camera" projection="orthographic" x="0" y="-12" z="-20" pitch="-30" orthoHeight="8" renderer="pathtrace" pathSamples="512" maxBounces="1" denoise="false"/>
        <object3D id="floor" primitive="plane" width="400" height="400" y="4" rotationX="-90" material="floorMat"/>
        {WATER}
        {smoke}
      </composition>
      <lights>{SUN}</lights></scene>"##,
        cache.display()
    )
}

/// Smoke between a floor and the water surface dims the sun on its way down through the water and
/// the view on its way up: the light that reaches the floor crosses it along the refracted
/// direction, which the shadow ray under water must follow (it stopped counting the smoke), and
/// the camera's ray crosses it along its own refracted direction.
#[test]
fn smoke_in_the_water_dims_the_sun_and_the_view_of_a_floor() {
    let (Some(clear), Some(smoky)) = (render(&floor_through_smoke(false)), render(&floor_through_smoke(true))) else {
        return;
    };
    let up = Vec3::new(0.0, -1.0, 0.0);
    let to_sun = -shines(-35.0, -50.0).normalize();
    let sun_in_water = refract(-to_sun, up, 1.0, ETA);
    let eye = Vec3::new(0.0, 0.5, 0.866_025_4);
    let eye_in_water = refract(eye, up, 1.0, ETA);
    // the slab is 1.5 units thick; a ray's path through it is 1.5 / |cos| of its angle to the vertical
    let leg = |dir: Vec3| SLAB_EXTINCTION * 1.5 / dir.y.abs();
    let want = (-(leg(sun_in_water) + leg(eye_in_water))).exp();
    let mean = |r: &common::Rendered| {
        let v: Vec<f32> = r.px.iter().map(|p| p[0]).filter(|v| *v > 0.003).collect();
        assert!(v.len() > 1500, "the floor is seen: {} pixels", v.len());
        v.iter().sum::<f32>() / v.len() as f32
    };
    let got = mean(&smoky) / mean(&clear);
    eprintln!("floor through the smoke: {got} of the clear floor, expected {want}");
    assert!((got / want - 1.0).abs() < 0.05, "the smoke dims the floor to {got} of the clear floor, expected {want}");
}

/// A camera inside absorbing water looks along a glowing, absorbing smoke slab: the radiance is the
/// emission of each part of the slab, dimmed by the water between the camera and that part and by
/// the smoke in front of it. The water's absorption must act on the light the smoke adds at the
/// distance it adds it, not on the whole ray at once.
#[test]
fn glowing_smoke_seen_through_absorbing_water_is_dimmed_by_the_water_in_front_of_each_part() {
    let cache = uniform_cache();
    let scene = format!(
        r##"<scene version="1.3"><project width="16" height="16" fps="10" duration="1" background="#000000"/>
      <assets><volume id="slab" src="{}" boundsMinX="-50" boundsMinY="-50" boundsMinZ="10" boundsMaxX="50" boundsMaxY="50" boundsMaxZ="30"/></assets>
      <materials>
        <material id="waterMat" baseColor="#FFFFFF" roughness="0.02" ior="{ETA}" doubleSided="true" transmission="1" attenuationColor="#80C0E0" attenuationDistance="20"/>
      </materials>
      <composition>
        <camera id="camera" x="0" y="3" z="0" fov="2" renderer="pathtrace" pathSamples="64" maxBounces="1" denoise="false"/>
        <object3D id="water" primitive="plane" width="2000" height="2000" y="0" rotationX="-90" material="waterMat"/>
        <object3D id="smoke" primitive="volume" volume="slab"><medium extinction="{SLAB_EXTINCTION}" albedo="#000000" emissionColor="#FFFFFF" emissionScale="0.5" stepSize="0.05" maxSteps="4096"/></object3D>
      </composition>
      <lights><light id="off" type="ambient" color="#000000" intensity="0"/></lights></scene>"##,
        cache.display()
    );
    let Some(r) = render(&scene) else { return };
    let centre = r.px[8 * 16 + 8];
    for (c, value) in [0x80u8, 0xC0, 0xE0].iter().enumerate() {
        let sigma = -linear(*value).ln() / 20.0;
        // emission E per unit length: the integral of E exp(-k u) exp(-sigma (10 + u)) over the slab's 20 units
        let total = SLAB_EXTINCTION + sigma;
        let want = 0.5 * (-10.0 * sigma).exp() * (1.0 - (-total * 20.0).exp()) / total;
        eprintln!("channel {c}: {} against {want}", centre[c]);
        assert!((centre[c] / want - 1.0).abs() < 0.04, "channel {c}: {} against {want}", centre[c]);
    }
}

/// A scene with a transmissive object compiles the shader of the water, whose march through media
/// is a copy that takes the water's absorption into account. A camera in the air, with no
/// absorbing water about, must see a medium exactly as it does without the object: the plain march
/// runs there, and the copy gives the same colour and transmittance with no absorption.
#[test]
fn a_medium_looks_the_same_with_a_transmissive_object_in_the_scene() {
    let cache = uniform_cache();
    let scene = |glass: &str| {
        format!(
            r##"<scene version="1.3"><project width="32" height="32" fps="10" duration="1" background="#000000"/>
      <assets><volume id="slab" src="{}" boundsMinX="-4" boundsMinY="-4" boundsMinZ="6" boundsMaxX="4" boundsMaxY="4" boundsMaxZ="12"/></assets>
      <materials>
        <material id="glassMat" baseColor="#FFFFFF" roughness="0.02" ior="1.5" transmission="1" attenuationColor="#80C0E0" attenuationDistance="3"/>
      </materials>
      <composition>
        <camera id="camera" x="0" y="0" z="0" fov="40" renderer="pathtrace" pathSamples="16" maxBounces="1" denoise="false"/>
        <object3D id="smoke" primitive="volume" volume="slab"><medium extinction="0.3" albedo="#000000" emissionColor="#FFC080" emissionScale="0.4" stepSize="0.1" maxSteps="4096"/></object3D>
        {glass}
      </composition>
      <lights><light id="off" type="ambient" color="#000000" intensity="0"/></lights></scene>"##,
            cache.display()
        )
    };
    let glass =
        r##"<object3D id="ball" primitive="sphere" radius="1" segments="16" x="0" y="0" z="-8" material="glassMat"/>"##;
    let (Some(plain), Some(with)) = (render(&scene("")), render(&scene(glass))) else { return };
    assert!(plain.px.iter().any(|p| p[0] > 0.05), "the medium glows");
    assert_eq!(plain.px, with.px, "a transmissive object elsewhere changes the medium's pixels");
}
