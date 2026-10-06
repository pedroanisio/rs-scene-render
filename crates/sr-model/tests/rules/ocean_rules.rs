fn codes(node: &str, version: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="24" duration="2"/><assets><image id="bed" src="bed.png" width="8" height="8"/><mesh id="terrain" src="terrain.obj"/><audio id="sound" src="sound.wav"/></assets><composition>{node}</composition></scene>"#
    );
    sr_model::validate_str(&xml, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn ocean_has_typed_versioned_schema_and_owned_impulses_and_waves() {
    let node = r#"<ocean id="sea" width="16" depth="12" cellSize="0.5" bottomDepth="2"><waterImpulse time="0.3" radius="2" amplitude="0.1"/><wave wavelength="8" amplitude="0.1" speed="2"/></ocean>"#;
    assert!(codes(node, "1.3").is_empty(), "{:?}", codes(node, "1.3"));
    assert!(codes(node, "1.2").contains(&"OCN1".into()));
}

#[test]
fn ocean_order_is_a_one_or_two_enumeration() {
    for order in ["1", "2"] {
        let node = format!(r#"<ocean id="o" order="{order}"/>"#);
        assert!(codes(&node, "1.3").is_empty(), "{:?}", codes(&node, "1.3"));
    }
    for order in ["0", "3", "second", "2.0", ""] {
        let node = format!(r#"<ocean id="o" order="{order}"/>"#);
        assert!(codes(&node, "1.3").contains(&"S06".into()), "{order:?}: {:?}", codes(&node, "1.3"));
    }
}

#[test]
fn ocean_work_allowance_is_a_positive_integer_up_to_one_trillion() {
    for work in ["1", "100000000", "1000000000", "1000000000000"] {
        let node = format!(r#"<ocean id="o" maxWork="{work}"/>"#);
        assert!(codes(&node, "1.3").is_empty(), "{work}: {:?}", codes(&node, "1.3"));
    }
    for work in ["0", "-1", "1000000000001", "1e12", "1.5"] {
        let node = format!(r#"<ocean id="o" maxWork="{work}"/>"#);
        assert!(codes(&node, "1.3").contains(&"S06".into()), "{work}: {:?}", codes(&node, "1.3"));
    }
}

#[test]
fn whitewater_work_allowance_has_the_same_trillion_ceiling() {
    for work in ["1", "1000000000", "1000000000000"] {
        let node = format!(r#"<ocean id="o"><whitewater maxWork="{work}"/></ocean>"#);
        assert!(codes(&node, "1.3").is_empty(), "{work}: {:?}", codes(&node, "1.3"));
    }
    for work in ["0", "-1", "1000000000001", "1e12", "1.5"] {
        let node = format!(r#"<ocean id="o"><whitewater maxWork="{work}"/></ocean>"#);
        assert!(codes(&node, "1.3").contains(&"S06".into()), "{work}: {:?}", codes(&node, "1.3"));
    }
}

const SEABED: &str = r#"<object3D id="seabed" primitive="plane" width="40" height="40" segments="8" y="2" rotationX="-90"><crater radius="3" depth="1" rimHeight="0.2" rimWidth="1"/></object3D>"#;
const ROCK: &str = r#"<object3D id="rock" primitive="sphere" radius="1" y="-3"/>"#;
const SHEET: &str = r#"<object3D id="sheet" primitive="plane" width="40" height="40" y="2" rotationX="-90"/>"#;

#[test]
fn ocean_colliders_name_deformable_beds_and_closed_bodies() {
    let ocean = |colliders: &str| format!(r#"{SEABED}{ROCK}{SHEET}<ocean id="o" colliders="{colliders}"/>"#);
    for ok in ["seabed", "rock", "seabed rock"] {
        assert!(codes(&ocean(ok), "1.3").is_empty(), "{ok}: {:?}", codes(&ocean(ok), "1.3"));
    }
    // A plane needs a crater to be a bed, and a sphere with none is a body: a plane
    // without one is neither. Repeats and objects that are not objects are errors.
    for bad in ["sheet", "seabed sheet", "seabed seabed", "o"] {
        assert!(
            codes(&ocean(bad), "1.3").contains(&"OCN6".into())
                || codes(&ocean(bad), "1.3").iter().any(|c| c.starts_with('S')),
            "{bad}: {:?}",
            codes(&ocean(bad), "1.3")
        );
    }
    assert!(codes(&ocean("sheet"), "1.3").contains(&"OCN6".into()));
    assert!(codes(&ocean("seabed seabed"), "1.3").contains(&"OCN6".into()));
    // Collider geometry is built once, so its shape is not animated.
    let animated = format!(
        r#"{SEABED}<object3D id="rock" primitive="sphere" radius="1" y="-3"><animate property="radius"><key time="0" value="1"/><key time="1" value="2"/></animate></object3D><ocean id="o" colliders="rock"/>"#
    );
    assert!(codes(&animated, "1.3").contains(&"OCN7".into()), "{:?}", codes(&animated, "1.3"));
    // Pose animation is fine.
    let moving = ROCK.replace(
        "/>",
        r#"><animate property="y"><key time="0" value="-3"/><key time="1" value="3"/></animate></object3D>"#,
    );
    let scene = format!(r#"{moving}<ocean id="o" colliders="rock"/>"#);
    assert!(codes(&scene, "1.3").is_empty(), "{:?}", codes(&scene, "1.3"));
}

#[test]
fn whitewater_checkpoint_memory_is_a_nonnegative_integer_up_to_4096_mib() {
    for mib in ["0", "1", "64", "4096"] {
        let node = format!(r#"<ocean id="o"><whitewater checkpointMemoryMiB="{mib}"/></ocean>"#);
        assert!(codes(&node, "1.3").is_empty(), "{mib}: {:?}", codes(&node, "1.3"));
    }
    for mib in ["-1", "4097", "1.5", "64MiB", ""] {
        let node = format!(r#"<ocean id="o"><whitewater checkpointMemoryMiB="{mib}"/></ocean>"#);
        assert!(codes(&node, "1.3").contains(&"S06".into()), "{mib:?}: {:?}", codes(&node, "1.3"));
    }
}

#[test]
fn ocean_references_dimensions_and_static_inputs_are_checked() {
    for node in [
        r#"<ocean id="o" bathymetry="sound"/>"#,
        r#"<ocean id="o" material="bed"/>"#,
        r#"<ocean id="o" bathymetryEncoding="terrarium"/>"#,
    ] {
        assert!(codes(node, "1.3").contains(&"OCN2".into()), "{:?}", codes(node, "1.3"));
    }
    for node in [
        r#"<ocean id="o" width="0.5" cellSize="1"/>"#,
        r#"<ocean id="o" width="4.2" cellSize="1"/>"#,
        r#"<ocean id="o"><wave wavelength="1"/></ocean>"#,
        r#"<ocean id="o"><waterImpulse type="add-water" amplitude="-1"/></ocean>"#,
    ] {
        assert!(codes(node, "1.3").contains(&"OCN3".into()), "{:?}", codes(node, "1.3"));
    }
    let node = r#"<ocean id="o"><animate property="cellSize"><key time="0" value="1"/></animate></ocean>"#;
    assert!(codes(node, "1.3").contains(&"OCN4".into()));
    assert!(codes(&node.replace("cellSize", "y"), "1.3").is_empty());
}

#[test]
fn whitewater_is_a_single_owned_source_with_typed_materials_and_time_window() {
    assert!(codes(
        r#"<ocean id="o"><whitewater emissionRate="2" threshold="0.2" seed="18446744073709551615"/></ocean>"#,
        "1.3"
    )
    .is_empty());
    for child in [
        r#"<whitewater/><whitewater/>"#,
        r#"<whitewater start="2" end="1"/>"#,
        r#"<whitewater foamMaterial="bed"/>"#,
        r#"<whitewater sprayMaterial="terrain"/>"#,
    ] {
        let c = codes(&format!(r#"<ocean id="o">{child}</ocean>"#), "1.3");
        assert!(c.contains(&"OCN5".into()), "{c:?}");
    }
}

#[test]
fn numeric_bathymetry_does_not_silently_ignore_an_image_layer_selection() {
    let xml = r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="1"/><assets><image id="bed" src="elevation.exr" width="8" height="8" layer="depth"/></assets><composition><ocean id="sea" bathymetry="bed"/></composition></scene>"#;
    let result = sr_model::validate_str(xml, &sr_model::LoadOptions::without_assets());
    assert!(result.diagnostics.iter().any(|d| d.code == "OCN2"));
    let primary = xml.replace(r#" layer="depth""#, "");
    assert!(!sr_model::validate_str(&primary, &sr_model::LoadOptions::without_assets()).has_errors());
}

#[test]
fn whitewater_foam_can_be_drawn_as_particles_or_mixed_into_the_water_albedo() {
    for mode in ["particles", "albedo"] {
        let node = format!(r#"<ocean id="o"><whitewater foamMode="{mode}"/></ocean>"#);
        assert!(codes(&node, "1.3").is_empty(), "{mode}: {:?}", codes(&node, "1.3"));
    }
    for mode in ["", "mix", "Albedo", "none"] {
        let node = format!(r#"<ocean id="o"><whitewater foamMode="{mode}"/></ocean>"#);
        assert!(codes(&node, "1.3").contains(&"S06".into()), "{mode:?}: {:?}", codes(&node, "1.3"));
    }
    // the mix's own attributes: a coverage radius over zero, an albedo and a roughness in 0 to 1
    let all = r#"<ocean id="o"><whitewater foamMode="albedo" foamRadius="1.5" foamAlbedo="0.9" foamRoughness="0.8"/></ocean>"#;
    assert!(codes(all, "1.3").is_empty(), "{:?}", codes(all, "1.3"));
    for bad in [
        r#"foamRadius="0""#,
        r#"foamRadius="-1""#,
        r#"foamAlbedo="1.5""#,
        r#"foamAlbedo="-0.1""#,
        r#"foamRoughness="2""#,
    ] {
        let node = format!(r#"<ocean id="o"><whitewater foamMode="albedo" {bad}/></ocean>"#);
        assert!(codes(&node, "1.3").contains(&"S06".into()), "{bad}: {:?}", codes(&node, "1.3"));
    }
}
