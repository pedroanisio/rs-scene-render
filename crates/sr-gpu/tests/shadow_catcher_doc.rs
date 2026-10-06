//! `shadowCatcher` on an object reaches the 3D renderer: the ground keeps only the shadow.

mod common;
use common::*;

fn scene(catcher: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="128" height="128" fps="10" duration="2" background="#00000000"/>
        <composition>
          <object3D id="ground" primitive="plane" width="400" height="400" x="64" y="64" z="60" {catcher}/>
          <object3D id="ball" primitive="sphere" radius="15" x="64" y="64"/>
        </composition>
        <lights><light id="sun" type="directional" intensity="3" castShadow="true" yaw="45"/>
                <light id="fill" type="ambient" intensity="0.5"/></lights></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

#[test]
fn a_catcher_ground_keeps_only_the_shadow() {
    let Some(plain) = render(&scene("")) else { return };
    let Some(catcher) = render(&scene(r#"shadowCatcher="true""#)) else { return };
    let problems: Vec<_> = catcher.stats.unsupported.iter().chain(&catcher.stats.errors).collect();
    assert!(problems.is_empty(), "{problems:?}");
    let opaque = |r: &Rendered| r.px.iter().filter(|p| p[3] > 0.97).count();
    let clear = |r: &Rendered| r.px.iter().filter(|p| p[3] < 0.02).count();
    assert_eq!(clear(&plain), 0, "the ordinary ground covers the frame");
    assert!(clear(&catcher) > 1000, "lit ground is transparent: {}", clear(&catcher));
    // the ball itself is drawn; the shadow is dark and partly covering
    assert!(opaque(&catcher) > 300, "the ball: {}", opaque(&catcher));
    let shadow = catcher.px.iter().filter(|p| p[3] > 0.1 && p[3] < 0.97).count();
    assert!(shadow > 100, "partly covering shadow pixels: {shadow}");
}
