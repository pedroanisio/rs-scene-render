//! SREP 71 in the schema: the subsurface attributes of materials, the procedural sky of dome lights, and material maps
//! that name a generator or image asset as `#id`, with the rules SKY1, SKY2 and MTX1-*attribute*. The cases follow
//! sr-core's kit (conformance/srep_cases/srep-0071.json); the three validation cases are copied verbatim, the pixel
//! cases are checked here for validity only (their pictures are sr-gpu's srep_0071_* tests).

/// The kit's document: the frame, an optional asset list, materials, composition and lights.
fn kit(assets: &str, materials: &str, body: &str, lights: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<scene version=\"1.6\">\n<project width=\"640\" height=\"360\" fps=\"24\" duration=\"1\" background=\"#000000FF\" seed=\"1\"/>\n<output id=\"still\" path=\"out/frame_%04d.png\" codec=\"png-sequence\"/>\n{assets}{materials}<composition>\n{body}\n</composition>\n{lights}</scene>\n"
    )
}

/// The error codes of a document, sorted and deduplicated; empty when it is valid.
fn codes(xml: &str) -> Vec<String> {
    match sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()) {
        Ok(_) => Vec::new(),
        Err(sr_model::LoadError::Invalid(r)) => {
            let mut c: Vec<String> = r.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.code.clone()).collect();
            c.sort();
            c.dedup();
            c
        }
        Err(e) => panic!("{e:?}"),
    }
}

const GRADIENT: &str = "<lights><light id=\"sky\" type=\"dome\" sky=\"gradient\" skyZenith=\"#2050A0\" skyHorizon=\"#C0D0E0\" skyGround=\"#403020\" skyExponent=\"1\" environmentVisible=\"true\"/></lights>\n";
const SUN: &str = "<lights><light id=\"sky\" type=\"dome\" sky=\"gradient\" skyZenith=\"#000000\" skyHorizon=\"#000000\" skyGround=\"#000000\" sunAzimuth=\"10\" sunElevation=\"5\" sunSize=\"5\" sunColor=\"#FF0000\" sunIntensity=\"1\" environmentVisible=\"true\"/></lights>\n";
const GEN_ASSETS: &str = "<assets><generator id=\"g\" kind=\"solid\" width=\"16\" height=\"16\" paint=\"#00FF00FF\"/><image id=\"im\" src=\"../assets/tile.png\" width=\"8\" height=\"8\"/></assets>\n";
const PLANE: &str =
    "<object3D id=\"o\" primitive=\"plane\" width=\"100\" height=\"100\" x=\"320\" y=\"180\" material=\"m\"/>";
const BALL: &str = "<object3D id=\"o\" primitive=\"sphere\" radius=\"80\" x=\"320\" y=\"180\" material=\"m\"/>";

fn map_material(attr: &str, value: &str) -> String {
    format!("<materials><material id=\"m\" baseColor=\"#FFFFFFFF\" unlit=\"true\" {attr}=\"{value}\"/></materials>\n")
}

fn subsurface(w: &str) -> String {
    format!("<materials><material id=\"m\" baseColor=\"#B0B0B0FF\" roughness=\"0.5\" subsurface=\"{w}\" subsurfaceColor=\"#E0F0D0FF\" subsurfaceRadius=\"20\"/></materials>\n")
}

#[test]
fn srep_0071_sky_cases_are_valid() {
    // srep-0071-sky-gradient and srep-0071-sky-horizon share their document; srep-0071-sky-sun-azimuth
    assert_eq!(codes(&kit("", "", "", GRADIENT)), Vec::<String>::new());
    assert_eq!(codes(&kit("", "", "", SUN)), Vec::<String>::new());
    // srep-0071-sky-sun
    let ahead = SUN.replace("sunAzimuth=\"10\" sunElevation=\"5\"", "sunAzimuth=\"0\" sunElevation=\"0\"");
    assert_eq!(codes(&kit("", "", "", &ahead)), Vec::<String>::new());
}

#[test]
fn srep_0071_sky_not_dome() {
    let lights = "<lights><light id=\"k\" type=\"directional\" sky=\"gradient\"/></lights>\n";
    assert_eq!(codes(&kit("", "", "", lights)), ["SKY1"]);
    // sky="none" is the default and allowed on any light
    let none = "<lights><light id=\"k\" type=\"directional\" sky=\"none\"/></lights>\n";
    assert_eq!(codes(&kit("", "", "", none)), Vec::<String>::new());
}

#[test]
fn srep_0071_sky_and_environment() {
    let lights = "<lights><light id=\"sky\" type=\"dome\" sky=\"gradient\" environment=\"x.hdr\"/></lights>\n";
    assert_eq!(codes(&kit("", "", "", lights)), ["SKY2"]);
    let image_only = "<lights><light id=\"sky\" type=\"dome\" environment=\"x.hdr\"/></lights>\n";
    assert_eq!(codes(&kit("", "", "", image_only)), Vec::<String>::new());
}

#[test]
fn srep_0071_map_generator_is_valid() {
    assert_eq!(codes(&kit(GEN_ASSETS, &map_material("baseColorMap", "#g"), PLANE, "")), Vec::<String>::new());
    // an image asset is the other image source
    assert_eq!(codes(&kit(GEN_ASSETS, &map_material("baseColorMap", "#im"), PLANE, "")), Vec::<String>::new());
}

#[test]
fn srep_0071_map_not_an_image_source() {
    assert_eq!(codes(&kit(GEN_ASSETS, &map_material("baseColorMap", "#m"), PLANE, "")), ["MTX1-baseColorMap"]);
}

#[test]
fn every_map_attribute_has_its_own_rule() {
    for attr in ["baseColorMap", "normalMap", "metallicRoughnessMap", "occlusionMap", "emissiveMap", "displacementMap"]
    {
        let code = format!("MTX1-{attr}");
        assert_eq!(codes(&kit(GEN_ASSETS, &map_material(attr, "#nothing"), PLANE, "")), [code.as_str()], "{attr}");
        assert_eq!(codes(&kit(GEN_ASSETS, &map_material(attr, "#g"), PLANE, "")), Vec::<String>::new(), "{attr}");
        // a file name is a file, as before
        assert_eq!(codes(&kit(GEN_ASSETS, &map_material(attr, "tile.png"), PLANE, "")), Vec::<String>::new());
    }
    // two bad maps on one material: one finding each
    let two = "<materials><material id=\"m\" baseColorMap=\"#x\" normalMap=\"#y\"/></materials>\n";
    assert_eq!(codes(&kit(GEN_ASSETS, two, PLANE, "")), ["MTX1-baseColorMap", "MTX1-normalMap"]);
}

#[test]
fn srep_0071_subsurface_cases_are_valid() {
    let lit = "<lights><light id=\"sun\" type=\"directional\" yaw=\"60\" intensity=\"3\"/></lights>\n";
    // srep-0071-subsurface-zero-neutral and srep-0071-subsurface-wraps-terminator
    assert_eq!(codes(&kit("", &subsurface("0"), BALL, lit)), Vec::<String>::new());
    assert_eq!(codes(&kit("", &subsurface("1"), BALL, lit)), Vec::<String>::new());
}

#[test]
fn the_attribute_types() {
    let m = |attrs: &str| kit("", &format!("<materials><material id=\"m\" {attrs}/></materials>\n"), BALL, "");
    for ok in [
        "subsurfaceRadiusScale=\"1 0.5 0.25\"",
        "subsurfaceRadiusScale=\" 0 0  1 \"",
        "subsurfaceRadius=\"0\"",
        "subsurface=\"0.5\" subsurfaceColor=\"#FFFFFF\"",
    ] {
        assert_eq!(codes(&m(ok)), Vec::<String>::new(), "{ok}");
    }
    for bad in [
        "subsurfaceRadiusScale=\"1 0.5\"",
        "subsurfaceRadiusScale=\"1 0.5 0.25 1\"",
        "subsurfaceRadiusScale=\"1 0.5 2\"",
        "subsurfaceRadiusScale=\"\"",
        "subsurface=\"1.5\"",
        "subsurfaceRadius=\"-1\"",
    ] {
        assert_eq!(codes(&m(bad)), ["S06"], "{bad}");
    }
    let l = |attrs: &str| {
        kit("", "", "", &format!("<lights><light id=\"d\" type=\"dome\" sky=\"gradient\" {attrs}/></lights>\n"))
    };
    assert_eq!(
        codes(&l("skyExponent=\"2\" sunAzimuth=\"-30\" sunElevation=\"-5\" sunSize=\"0\"")),
        Vec::<String>::new()
    );
    for bad in ["skyExponent=\"0\"", "sunSize=\"-1\"", "sunIntensity=\"-1\"", "skyZenith=\"blue\""] {
        assert_eq!(codes(&l(bad)), ["S06"], "{bad}");
    }
    let sky = |v: &str| kit("", "", "", &format!("<lights><light id=\"d\" type=\"dome\" sky=\"{v}\"/></lights>\n"));
    assert_eq!(codes(&sky("physical")), ["S06"]);
}

#[test]
fn the_model_reads_the_new_attributes() {
    let xml = kit(
        GEN_ASSETS,
        "<materials><material id=\"m\" subsurface=\"0.25\" subsurfaceRadiusScale=\"0.5 0.5 1\" baseColorMap=\"#g\"/></materials>\n",
        BALL,
        "<lights><light id=\"d\" type=\"dome\" sky=\"gradient\" sunIntensity=\"2\"/></lights>\n",
    );
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let m = &d.scene.materials.as_ref().unwrap().materials[0];
    assert_eq!(m.subsurface.get(), 0.25);
    assert_eq!(m.subsurface_radius_scale, vec![0.5, 0.5, 1.0]);
    assert_eq!(m.subsurface_radius.get(), 10.0);
    assert_eq!(m.base_color_map.as_deref(), Some("#g"));
    let l = &d.scene.lights.as_ref().unwrap().lights[0];
    assert_eq!(l.sky, sr_model::model::Sky::Gradient);
    assert_eq!(l.sky_exponent.get(), 0.5);
    assert_eq!(l.sun_elevation, 45.0);
    assert_eq!(l.sun_size.get(), 0.53);
    assert_eq!(l.sun_intensity.get(), 2.0);
}

#[test]
fn a_document_reference_is_not_a_file() {
    // with asset verification on, #g is not looked for on disk (no A01), while a missing file still is
    let dir = std::env::temp_dir();
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir) };
    let gen_only =
        "<assets><generator id=\"g\" kind=\"solid\" width=\"16\" height=\"16\" paint=\"#00FF00FF\"/></assets>\n";
    let xml = kit(gen_only, &map_material("baseColorMap", "#g"), PLANE, "");
    assert!(sr_model::load_str(&xml, &opts).is_ok());
    let xml = kit(gen_only, &map_material("baseColorMap", "no-such-file-srep71.png"), PLANE, "");
    let r = sr_model::validate_str(&xml, &opts);
    assert!(r.diagnostics.iter().any(|d| d.code == "A01"), "{:?}", r.diagnostics);
    assert!(sr_model::assets::is_document_reference("material", "normalMap", "#g"));
    // the generated material element reports its type's name
    let xml = kit(gen_only, &map_material("baseColorMap", "#g"), PLANE, "");
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let m = &d.scene.materials.as_ref().unwrap().materials[0];
    let name = sr_model::element::Element::element_name(m);
    assert!(sr_model::assets::is_document_reference(name, "baseColorMap", "#g"), "{name}");
    assert!(!sr_model::assets::is_document_reference("material", "materialX", "#g"));
    assert!(!sr_model::assets::is_document_reference("light", "environment", "#g"));
}
