use super::common;

/// Luminance of every pixel (linear, row-major) of a flat plane of the given material under a
/// uniform visible dome, path traced with `spp` samples per pixel and no denoising.
fn plane_luminance(material: &str, pitch: f64, size: [u32; 2], spp: u32) -> Option<Vec<f32>> {
    let gpu = common::gpu()?;
    // the fixture image gray.png is uniform, so the dome is a constant sky of linear radiance ~0.2158
    let xml = format!(
        r##"<scene version="1.3"><project width="{}" height="{}" fps="10" duration="1" background="#00000000"/>
        <materials><material id="m" {material}/></materials>
        <composition><camera id="camera" x="0" y="-38" z="-300" pitch="{pitch}" fov="42" renderer="pathtrace" pathSamples="{spp}" maxBounces="3" denoise="false"/>
        <object3D id="plane" primitive="plane" width="40000" height="40000" y="0" rotationX="-90" material="m"/></composition>
        <lights><light id="sky" type="dome" environment="gray.png" environmentVisible="true" intensity="1"/></lights></scene>"##,
        size[0], size[1]
    );
    let doc =
        sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(common::fixtures()) })
            .unwrap();
    let shot = common::render_times_on(gpu, &doc, &[0.0])?;
    assert!(shot.stats.errors.is_empty(), "{:?}", shot.stats.errors);
    Some(shot.px.iter().map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]).collect())
}

const WATER: &str = r##"baseColor="#06212C" roughness="0.06" ior="1.333" doubleSided="true""##;

/// Rows below the horizon (the horizon is at about a third of the frame at this pitch).
fn below_horizon(lum: &[f32], size: [u32; 2]) -> &[f32] {
    &lum[(size[0] * (size[1] * 2 / 5)) as usize..]
}

/// The water is almost purely specular (Fresnel on a dark albedo), so the lobe that is sampled
/// decides the noise. Choosing it with a fixed probability gave a binomial estimate: with 8
/// samples, 0.75^8 = 10% of pixels held no specular sample at all. Relative error against a
/// 512-sample render of the same scene was 0.59 and 9-10% of pixels were dark.
#[test]
fn water_at_eight_samples_is_close_to_its_converged_value() {
    let size = [128, 72];
    let Some(few) = plane_luminance(WATER, -3.0, size, 8) else { return };
    let many = plane_luminance(WATER, -3.0, size, 512).unwrap();
    let (few, many) = (below_horizon(&few, size), below_horizon(&many, size));
    let mean = many.iter().sum::<f32>() / many.len() as f32;
    let rmse = (few.iter().zip(many).map(|(a, b)| (a - b).powi(2)).sum::<f32>() / many.len() as f32).sqrt();
    let dark = few.iter().zip(many).filter(|(a, b)| **a < 0.05 * **b).count() as f32 / many.len() as f32;
    if std::env::var("PRINT_STATS").is_ok() {
        eprintln!("water: mean {mean:.4} relative RMSE {:.4} dark fraction {dark:.4}", rmse / mean);
    }
    assert!(rmse / mean < 0.20, "relative error of 8 samples against 512: {}", rmse / mean);
    assert!(dark < 0.01, "fraction of pixels that lost the specular lobe entirely: {dark}");
}

/// Mean luminance (below the horizon) of a high-sample render of each material, measured with the
/// sampler that chose the lobe with a fixed probability (an unbiased estimator of the same
/// integral). Any better lobe choice must reproduce them: it may only lower the noise.
const EXPECTED: [(&str, &str, f64, f64); 3] = [
    ("water", WATER, -3.0, 0.11999),
    ("plastic", r##"baseColor="#CC3333" roughness="0.3" ior="1.5""##, -20.0, 0.05562),
    ("metal", r##"baseColor="#D0D0D0" metallic="1" roughness="0.3""##, -20.0, 0.13709),
];

#[test]
fn lobe_choice_does_not_change_the_expectation() {
    let size = [48, 27];
    for (name, material, pitch, expected) in EXPECTED {
        let Some(lum) = plane_luminance(material, pitch, size, 4096) else { return };
        let lum = below_horizon(&lum, size);
        let mean = lum.iter().sum::<f32>() as f64 / lum.len() as f64;
        if std::env::var("PRINT_STATS").is_ok() {
            eprintln!("expectation {name}: {mean:.7}");
        }
        if expected > 0.0 {
            assert!((mean - expected).abs() <= 0.005 * expected, "{name}: mean {mean} vs {expected}");
        }
    }
}
