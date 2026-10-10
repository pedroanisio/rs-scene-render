//! SREP 70: validation of parametric geometry. V14 (version 1.6), PAR1 and PAR2 (exactly one definition, on the
//! right kind), and C69, which no longer asks a parametric shape for a width and height. The SREP's findings cases,
//! with the documents of the kit (`conformance/srep_cases/srep-0070.json`), and two of its pixel cases, which are
//! valid.

use sr_model::{validate_str, LoadOptions};

fn codes(xml: &str) -> Vec<String> {
    let r = validate_str(xml, &LoadOptions::without_assets());
    let mut c: Vec<String> =
        r.diagnostics.iter().filter(|d| d.severity == sr_model::Severity::Error).map(|d| d.code.clone()).collect();
    c.sort();
    c
}

const PATH_NEEDS_CURVE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.6">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<composition>
<shape id="e" shape="parametric" fill="#FF0000FF"/>
</composition>
</scene>
"##;

const PATH_VERSION_GATE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.5">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<composition>
<shape id="e" shape="parametric" fill="#FF0000FF"><parametricPath x="t" y="t"/></shape>
</composition>
</scene>
"##;

const SURFACE_NEEDS_CHILD: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.6">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<materials><material id="m-red" baseColor="#FF0000FF" unlit="true"/><material id="m-red2" baseColor="#FF0000FF" unlit="true" doubleSided="true"/></materials>
<composition>
<object3D id="s" primitive="parametric" material="m-red"/>
</composition>
</scene>
"##;

const PATH_ELLIPSE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.6">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<composition>
<shape id="e" shape="parametric" fill="#FF0000FF"><parametricPath x="320 + 100 * Math.cos(t)" y="180 + 50 * Math.sin(t)" t0="0" t1="6.283185307179586" samples="256" closed="true"/></shape>
</composition>
</scene>
"##;

const HEIGHTFIELD_FLAT: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.6">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<materials><material id="m-red" baseColor="#FF0000FF" unlit="true"/><material id="m-red2" baseColor="#FF0000FF" unlit="true" doubleSided="true"/></materials>
<composition>
<object3D id="h" primitive="heightfield" material="m-red" x="320" y="180" rotationX="90"><heightfield height="0" width="100" depth="50" xSamples="4" zSamples="4"/></object3D>
</composition>
</scene>
"##;

#[test]
fn srep_0070_path_needs_curve() {
    assert_eq!(codes(PATH_NEEDS_CURVE), ["PAR1"]);
}

#[test]
fn srep_0070_path_version_gate() {
    assert_eq!(codes(PATH_VERSION_GATE), ["V14"]);
}

#[test]
fn srep_0070_surface_needs_child() {
    assert_eq!(codes(SURFACE_NEEDS_CHILD), ["PAR2"]);
}

#[test]
fn the_pixel_cases_are_valid_and_a_parametric_shape_needs_no_box() {
    assert_eq!(codes(PATH_ELLIPSE), Vec::<String>::new());
    assert_eq!(codes(HEIGHTFIELD_FLAT), Vec::<String>::new());
    // C69 still asks every other shape for its width and height
    assert_eq!(
        codes(&PATH_ELLIPSE.replace(r#"shape="parametric""#, r#"shape="path" path="M0 0 L1 1""#)),
        ["C69", "PAR1"]
    );
}

#[test]
fn a_definition_on_another_kind_is_refused() {
    let surface_on_sphere = HEIGHTFIELD_FLAT.replace(r#"primitive="heightfield""#, r#"primitive="sphere""#);
    assert_eq!(codes(&surface_on_sphere), ["PAR2"]);
    let curve_on_rect = PATH_ELLIPSE.replace(r#"shape="parametric""#, r#"shape="rect" width="4" height="4""#);
    assert_eq!(codes(&curve_on_rect), ["PAR1"]);
}
