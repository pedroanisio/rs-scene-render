//! SREP 72: the default precision (`f16`) renders byte for byte as before `precision` existed, on the reference host.
//!
//! The documents below (masks, blend modes, isolation, mattes, 2.5D, vectors with strokes and gradients, effects, a
//! custom shader, particles and a 3D object) are rendered at the default precision, and the SHA-256 of each frame's
//! texels is compared with `tests/fixtures/precision_f16_hashes.json`. Those hashes were recorded on the reference
//! host from the engine before SREP 72 (the file names the commit and the adapter).
//!
//! The contract is "identical on the reference host", not "identical everywhere": CPU rasterisers differ across CPU
//! models (plan A7). The comparison runs only where `SR_REFERENCE_HOST=1`; elsewhere the test renders the documents
//! and checks that nothing failed. `SR_RECORD_F16_HASHES=<path>` writes the hashes of this build to `path` instead.

mod common;
use common::*;
use sha2::{Digest, Sha256};

const GOLDEN: &str = r##"
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
  <layer id="card3d" asset="quad" x="104" y="52" anchorX="1" anchorY="1" scaleX="8" scaleY="8" threeD="true" rotationX="20" rotationY="-25"/>
  <layer id="star" asset="white" x="4" y="50" scaleX="5" scaleY="5" blend="difference" rotation="30">
    <mask type="star" x="0" y="0" width="4" height="4" points="6" innerRadius="0.5"/>
  </layer>"##;

const PAINTS: &str = r##"<paints><linearGradient id="bg" x1="0" y1="0" x2="1" y2="1" interpolationSpace="oklch" dither="false">
  <stop offset="0" color="#1B2A49"/><stop offset="0.6" color="#6B3FA0" midpoint="0.4"/><stop offset="1" color="#F2A65A"/>
</linearGradient></paints>"##;

const VECTORS: &str = r##"
  <shape id="e" shape="ellipse" x="10.3" y="6.6" width="50.5" height="31" fill="url(#bg)" stroke="#FFFFFFCC" strokeWidth="2.5"/>
  <shape id="s" shape="star" x="70" y="4" width="40" height="40" points="7" fill="#20C0A0" opacity="0.8" rotation="12"/>
  <shape id="p" shape="path" x="4" y="40" width="120" height="30" path="M0 20 C20 -10 40 50 60 10 S100 30 120 0" fill="#00000000" stroke="#FF6020" strokeWidth="3" strokeCap="round" dash="8 4"/>
  <shape id="r" shape="rounded-rect" x="88.5" y="44.25" width="30" height="20" radius="6" fill="#3050FF80"/>"##;

fn shader(name: &str) -> String {
    format!("{}/tests/shaders/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// `(name, project attributes, extra sections, composition)` of each document.
fn documents() -> Vec<(&'static str, String, String, String)> {
    vec![
        ("golden", r#"width="128" height="72" background="url(#bg)""#.into(), PAINTS.into(), GOLDEN.into()),
        ("vectors", r##"width="128" height="72" background="#101018""##.into(), PAINTS.into(), VECTORS.into()),
        (
            "effects",
            r##"width="128" height="72" background="#000000""##.into(),
            format!(
                r#"<effects><effect id="b" type="blur" radius="3"/><effect id="g" type="glow" radius="4"/><effect id="sh" type="shader" src="{}"/></effects>"#,
                shader("invert.glsl")
            ),
            r##"<shape id="a" shape="rect" x="8" y="8" width="40" height="30" fill="#FF8000" effects="b"/>
                <shape id="b2" shape="ellipse" x="60" y="10" width="30" height="30" fill="#40A0FF" effects="g"/>
                <layer id="c" asset="quad" x="96" y="40" scaleX="12" scaleY="12" effects="sh"/>"##
                .into(),
        ),
        (
            "particles",
            r##"width="128" height="72" background="#000000""##.into(),
            String::new(),
            r#"<particleEmitter id="sparks" preset="sparks" x="64" y="50" seed="3"/>"#.into(),
        ),
        (
            "object3d",
            r##"width="128" height="72" background="#000000""##.into(),
            r##"<materials><material id="matte-gray" baseColor="#888888" roughness="0.6"/></materials>"##.into(),
            r#"<object3D id="ball" primitive="sphere" radius="20" x="64" y="36" material="matte-gray"/>"#.into(),
        ),
    ]
}

fn hash(r: &Rendered) -> String {
    let mut h = Sha256::new();
    h.update(r.size[0].to_le_bytes());
    h.update(r.size[1].to_le_bytes());
    for p in &r.px {
        for c in p {
            h.update(c.to_bits().to_le_bytes());
        }
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn the_default_precision_renders_as_before_on_the_reference_host() {
    let mut got = serde_json::Map::new();
    let mut adapter = String::new();
    for (name, project, extra, body) in documents() {
        let (pre, post) = if extra.starts_with("<effects>") { ("", extra.as_str()) } else { (extra.as_str(), "") };
        let xml = format!(
            r#"<scene version="1.1"><project fps="10" duration="2" {project}/>{ASSETS}{pre}<composition>{body}</composition>{post}</scene>"#
        );
        let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
        let d = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{name}: {e:?}\n{xml}"));
        let Some(r) = render_times(&d, &[1.0]) else { return };
        assert!(
            r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
            "{name}: {:?} {:?}",
            r.stats.errors,
            r.stats.unsupported
        );
        assert_eq!(r.renderer.format(), sr_gpu::resources::FORMAT, "{name}: the default working format is f16");
        adapter = format!(
            "{} ({:?}, {})",
            r.renderer.gpu().info.name,
            r.renderer.gpu().info.backend,
            r.renderer.gpu().info.driver_info
        );
        got.insert(name.to_string(), serde_json::Value::String(hash(&r)));
    }
    if let Some(path) = std::env::var_os("SR_RECORD_F16_HASHES") {
        let out = serde_json::json!({ "adapter": adapter, "hashes": got });
        std::fs::write(path, serde_json::to_string_pretty(&out).unwrap() + "\n").unwrap();
        return;
    }
    if std::env::var("SR_REFERENCE_HOST").as_deref() != Ok("1") {
        eprintln!("not the reference host: rendered {} documents, hashes not compared", got.len());
        return;
    }
    let stored: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(format!("{}/tests/fixtures/precision_f16_hashes.json", env!("CARGO_MANIFEST_DIR")))
            .expect("stored hashes"),
    )
    .unwrap();
    assert_eq!(stored["adapter"].as_str(), Some(adapter.as_str()), "the reference adapter");
    for (name, h) in &got {
        assert_eq!(stored["hashes"][name].as_str(), h.as_str(), "{name}: the default precision changed the frame");
    }
}
