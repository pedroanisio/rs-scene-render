//! The horizon of a glossy dark sea at few samples per pixel: few of a pixel's samples may differ
//! from the rest by a lobe the surface hardly has, and the mean must stay what many samples give.

use super::common;

const W: u32 = 160;
const H: u32 = 90;

/// A camera 1.5 above a glossy dark plane that reaches the horizon (a far sea), under a visible grey dome.
fn sea(samples: u32) -> String {
    surface(samples, "#06212C", 0.06)
}

/// The same view of a plane of the given colour and roughness.
fn surface(samples: u32, base: &str, roughness: f32) -> String {
    format!(
        r##"<scene version="1.3"><project width="{W}" height="{H}" fps="10" duration="1" background="#00000000"/>
      <materials><material id="water" baseColor="{base}" roughness="{roughness}" ior="1.333" doubleSided="true"/></materials>
      <composition>
        <camera id="camera" x="0" y="-1.5" z="-10" fov="40" renderer="pathtrace" pathSamples="{samples}" maxBounces="4" denoise="false"/>
        <object3D id="farSea" primitive="plane" width="40000" height="40000" y="0" rotationX="-90" material="water"/>
      </composition>
      <lights><light id="sky" type="dome" environment="gray.png" environmentVisible="true" intensity="1"/></lights></scene>"##
    )
}

fn lum(p: [f32; 4]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

fn render(scene: &str) -> Option<common::Rendered> {
    let gpu = common::gpu()?;
    let doc =
        sr_model::load_str(scene, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(common::fixtures()) })
            .unwrap_or_else(|e| panic!("{e:?}"));
    let shot = common::render_times_on(gpu, &doc, &[0.0]).unwrap();
    assert!(shot.stats.errors.is_empty(), "{:?}", shot.stats.errors);
    Some(shot)
}

/// Rows just under the horizon, where the plane is seen at a grazing angle.
const BAND: std::ops::Range<u32> = 47..62;

/// (mean luminance of the band, pixels more than 15 % under their row's mean in the reference).
fn band(shot: &common::Rendered, reference: &common::Rendered) -> (f32, usize) {
    let mut sum = 0.0;
    let mut dark = 0;
    for y in BAND {
        let row_ref: f32 = (0..W).map(|x| lum(reference.at(x, y))).sum::<f32>() / W as f32;
        for x in 0..W {
            let v = lum(shot.at(x, y));
            sum += v;
            dark += usize::from(v < 0.85 * row_ref);
        }
    }
    (sum / (W * BAND.len() as u32) as f32, dark)
}

#[test]
fn a_glossy_sea_at_the_horizon_has_no_specks_at_eight_samples_and_the_same_mean_as_many() {
    let (Some(few), Some(many)) = (render(&sea(8)), render(&sea(512))) else { return };
    let (mean_few, dark_few) = band(&few, &many);
    let (mean_many, dark_many) = band(&many, &many);
    let pixels = (W * BAND.len() as u32) as usize;
    println!(
        "band mean {mean_few:.4} at 8 samples, {mean_many:.4} at 512; pixels 15 % under their row: {dark_few} of {pixels} at 8, {dark_many} at 512"
    );
    assert!((mean_few - mean_many).abs() <= 0.01 * mean_many, "the mean moved: {mean_few} against {mean_many}");
    assert!(dark_few * 200 <= pixels, "{dark_few} of {pixels} pixels are specks (at most 0.5 %)");
}

// ---------------------------------------------------------------- the reflectance a pixel must show

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Mean luminance of each row of the picture in rows `rows`, against the reflectance times the dome's radiance.
fn rows_against_reflectance(base: [f64; 3], roughness: f64, rows: &[u32]) -> Option<Vec<(u32, f64, f64)>> {
    let shot = render(&surface(
        256,
        &format!("#{:02X}{:02X}{:02X}", base[0] as u8, base[1] as u8, base[2] as u8),
        roughness as f32,
    ))?;
    let dome = srgb_to_linear(128.0 / 255.0);
    let albedo_lum = 0.2126 * srgb_to_linear(base[0] / 255.0)
        + 0.7152 * srgb_to_linear(base[1] / 255.0)
        + 0.0722 * srgb_to_linear(base[2] / 255.0);
    let focal = (W as f64 * 0.5) / (20.0f64.to_radians()).tan();
    let mut out = Vec::new();
    for &y in rows {
        let (mut got, mut want) = (0.0, 0.0);
        for x in 0..W {
            got += f64::from(lum(shot.at(x, y)));
            let (dx, dy) = (x as f64 + 0.5 - W as f64 * 0.5, y as f64 + 0.5 - H as f64 * 0.5);
            let nv = dy / (dx * dx + dy * dy + focal * focal).sqrt();
            let (spec, diffuse) = common::directional_albedo(nv, roughness, 1.333);
            want += dome * (spec + albedo_lum * diffuse);
        }
        out.push((y, got / W as f64, want / W as f64));
    }
    Some(out)
}

#[test]
fn a_surface_is_as_bright_as_its_reflectance_says_at_every_angle_with_the_sampler_of_visible_normals() {
    // a glossy dark sea and a rough bright floor, under a uniform dome: the radiance of a pixel is the dome's
    // times the directional albedo at its view angle, computed here by quadrature without any sampling
    let rows = [47, 48, 50, 54, 60, 70, 85];
    for (base, roughness) in [([6.0, 33.0, 44.0], 0.06), ([176.0, 176.0, 176.0], 0.5), ([128.0, 40.0, 40.0], 0.25)] {
        let Some(table) = rows_against_reflectance(base, roughness, &rows) else { return };
        for (y, got, want) in table {
            println!("roughness {roughness}, row {y}: picture {got:.4}, reflectance {want:.4}");
            assert!((got - want).abs() <= 0.03 * want, "row {y}, roughness {roughness}: {got} against {want}");
        }
    }
}
