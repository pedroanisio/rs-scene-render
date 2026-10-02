//! Golden-frame regression: a scene exercising the core compositing features is
//! rendered at several times and compared with PNGs in tests/golden
//! (PSNR ≥ 50 dB and max ΔE2000 ≤ 1). `SR_BLESS=1` rewrites the goldens.

mod common;
use common::*;

const SCENE: &str = r##"
  <group id="cards" x="8" y="8" width="112" height="24" layout="row" gap="4" justify="space-between" alignItems="center">
    <layer id="c1" asset="quad" scaleX="10" scaleY="10"><mask type="rounded-rect" x="0" y="0" width="2" height="2" radius="0.5" feather="0.2"/></layer>
    <layer id="c2" asset="checker" blend="overlay" opacity="0.8"/>
    <layer id="c3" asset="noise" blend="screen"/>
    <layer id="c4" asset="src" scaleX="5" scaleY="5" blend="color"/>
  </group>
  <group id="iso" isolate="true" width="64" height="32" x="8" y="36" opacity="0.7" blend="multiply">
    <layer id="w1" asset="wide" fit="cover" boxWidth="32" boxHeight="32"/>
    <layer id="w2" asset="wide" x="24" fit="contain-blur" boxWidth="32" boxHeight="32" flipX="true"/>
  </group>
  <layer id="matte" asset="half" x="80" y="36" scaleX="10" scaleY="8"/>
  <layer id="lit" asset="white" x="80" y="36" scaleX="10" scaleY="8" matte="matte" matteMode="luma"/>
  <layer id="card3d" asset="quad" x="104" y="52" anchorX="1" anchorY="1" scaleX="8" scaleY="8" threeD="true" rotationX="20">
    <animate property="rotationY"><key time="0" value="-40"/><key time="2" value="40"/></animate>
  </layer>
  <layer id="star" asset="white" x="4" y="50" scaleX="5" scaleY="5" blend="difference">
    <mask type="star" x="0" y="0" width="4" height="4" points="6" innerRadius="0.5"/>
    <animate property="rotation"><key time="0" value="0"/><key time="2" value="90"/></animate>
  </layer>"##;

const PAINTS: &str = r##"<paints><linearGradient id="bg" x1="0" y1="0" x2="1" y2="1" interpolationSpace="oklch" dither="false">
  <stop offset="0" color="#1B2A49"/><stop offset="0.6" color="#6B3FA0" midpoint="0.4"/><stop offset="1" color="#F2A65A"/>
</linearGradient></paints>"##;

#[test]
fn kitchen_scene_matches_goldens() {
    let d = doc(r#"width="128" height="72" background="url(#bg)""#, PAINTS, SCENE);
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let bless = std::env::var("SR_BLESS").is_ok_and(|v| v == "1");
    let out = std::env::var_os("SR_GOLDEN_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/golden-output"));
    std::fs::create_dir_all(&out).unwrap();
    let mut failures = Vec::new();
    let mut reports = Vec::new();
    for (k, t) in [0.0, 0.7, 1.4].into_iter().enumerate() {
        let Some(r) = render_times(&d, &[t]) else {
            assert!(std::env::var_os("CI").is_none(), "CI requires a working GPU adapter");
            return;
        };
        let rgba = r.renderer.to_srgb8(&r.px);
        let actual = image::RgbaImage::from_raw(r.size[0], r.size[1], rgba.clone()).unwrap();
        actual.save(out.join(format!("kitchen_{k}.actual.png"))).unwrap();
        // Unsupported content and render errors must fail even when blessing.
        assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
        assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
        let info = &r.renderer.gpu().info;
        let profile = reference_profile(&info.name, info.vendor, info.backend);
        let dir = profile.map_or_else(|| base.clone(), |name| base.join(name));
        let path = dir.join(format!("kitchen_{k}.png"));
        let golden = match reference(&path, &actual, bless, std::env::var_os("CI").is_some()) {
            Ok(Some(golden)) => golden,
            Ok(None) => continue,
            Err(e) => {
                failures.push(e);
                continue;
            }
        };
        golden.save(out.join(format!("kitchen_{k}.expected.png"))).unwrap();
        let got = sr_gpu::golden::from_rgba8(&rgba);
        let want = sr_gpu::golden::from_rgba8(golden.as_raw());
        let c = sr_gpu::golden::compare(&got, &want);
        let diff = image::RgbaImage::from_fn(r.size[0], r.size[1], |x, y| {
            let a = actual.get_pixel(x, y).0;
            let b = golden.get_pixel(x, y).0;
            image::Rgba([
                a[0].abs_diff(b[0]).saturating_mul(16),
                a[1].abs_diff(b[1]).saturating_mul(16),
                a[2].abs_diff(b[2]).saturating_mul(16),
                255,
            ])
        });
        diff.save(out.join(format!("kitchen_{k}.diff-x16.png"))).unwrap();
        reports.push(serde_json::json!({"frame":k,"time":t,"comparison":c,"reference_profile":profile,
                              "adapter":format!("{:?}",r.renderer.gpu().info)}));
        if !c.passes() {
            failures.push(format!("frame {k} (t = {t}): {c:?}"));
        }
    }
    std::fs::write(out.join("report.json"), serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
    assert!(failures.is_empty(), "{}; images and metrics: {}", failures.join("\n"), out.display());
}

fn reference_profile(name: &str, vendor: u32, backend: wgpu::Backend) -> Option<&'static str> {
    // This adapter reproduces the reviewed historical references byte for byte.
    // Select by hardware identity, never by whichever image gives the lowest error.
    (vendor == 0x10de && name == "NVIDIA RTX 6000 Ada Generation" && backend == wgpu::Backend::Vulkan)
        .then_some("nvidia-rtx6000-ada-vulkan")
}

#[test]
fn hardware_reference_profile_requires_the_recorded_adapter_and_backend() {
    let name = "NVIDIA RTX 6000 Ada Generation";
    assert_eq!(reference_profile(name, 0x10de, wgpu::Backend::Vulkan), Some("nvidia-rtx6000-ada-vulkan"));
    assert_eq!(reference_profile(name, 0x10de, wgpu::Backend::Gl), None);
    assert_eq!(reference_profile("llvmpipe", 0x10005, wgpu::Backend::Vulkan), None);
}

fn reference(
    path: &std::path::Path,
    actual: &image::RgbaImage,
    bless: bool,
    ci: bool,
) -> Result<Option<image::RgbaImage>, String> {
    if ci && bless {
        return Err("SR_BLESS is forbidden in CI".into());
    }
    if bless {
        actual.save(path).map_err(|e| e.to_string())?;
        return Ok(None);
    }
    if !path.is_file() {
        return Err(format!(
            "missing golden {}; review the image and explicitly set SR_BLESS=1 locally",
            path.display()
        ));
    }
    let expected = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    if expected.dimensions() != actual.dimensions() {
        return Err(format!(
            "golden dimensions {:?} differ from rendered {:?}",
            expected.dimensions(),
            actual.dimensions()
        ));
    }
    Ok(Some(expected))
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sr-golden-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("reference.png")
}

#[test]
fn missing_reference_fails_without_writing_it() {
    let path = scratch("missing");
    let _ = std::fs::remove_file(&path);
    let actual = image::RgbaImage::new(2, 2);
    assert!(reference(&path, &actual, false, false).is_err());
    assert!(!path.exists());
}

#[test]
fn ci_cannot_bless_a_reference() {
    let path = scratch("ci");
    let old = image::RgbaImage::from_pixel(2, 2, image::Rgba([255; 4]));
    old.save(&path).unwrap();
    assert!(reference(&path, &image::RgbaImage::new(2, 2), true, true).is_err());
    assert_eq!(image::open(path).unwrap().to_rgba8(), old);
}

#[test]
fn reference_dimensions_must_match_even_with_equal_pixel_counts() {
    let path = scratch("size");
    image::RgbaImage::new(4, 1).save(&path).unwrap();
    assert!(reference(&path, &image::RgbaImage::new(2, 2), false, false).is_err());
}

#[test]
fn explicit_local_bless_is_required_to_create_or_replace_a_reference() {
    let path = scratch("bless");
    let _ = std::fs::remove_file(&path);
    let actual = image::RgbaImage::from_pixel(2, 2, image::Rgba([127; 4]));
    assert!(reference(&path, &actual, true, false).unwrap().is_none());
    assert_eq!(reference(&path, &actual, false, false).unwrap().unwrap(), actual);
}
