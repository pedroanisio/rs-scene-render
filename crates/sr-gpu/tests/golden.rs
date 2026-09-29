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
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let bless = std::env::var("SR_BLESS").is_ok_and(|v| v == "1");
    for (k, t) in [0.0, 0.7, 1.4].into_iter().enumerate() {
        let Some(r) = render_times(&d, &[t]) else { return };
        let rgba = r.renderer.to_srgb8(&r.px);
        let path = dir.join(format!("kitchen_{k}.png"));
        if bless || !path.exists() {
            image::RgbaImage::from_raw(r.size[0], r.size[1], rgba).unwrap().save(&path).unwrap();
            eprintln!("wrote {}", path.display());
            continue;
        }
        let golden = image::open(&path).unwrap().to_rgba8();
        let got = sr_gpu::golden::from_rgba8(&rgba);
        let want = sr_gpu::golden::from_rgba8(golden.as_raw());
        let c = sr_gpu::golden::compare(&got, &want);
        assert!(c.passes(), "frame {k} (t = {t}) differs from {}: {c:?}", path.display());
        assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
    }
}
