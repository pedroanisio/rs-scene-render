fn codes(object: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="1.3"><project width="32" height="32" fps="30" duration="1"/><assets><volume id="cache" src="unused.srvol"/></assets><composition>{object}</composition></scene>"#
    );
    sr_model::validate_str(&xml, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}

const PYRO: &str = r#"<pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1"><pyroSource shape="sphere" radius="2" densityRate="2" temperatureRate="1000" start="0.15" end="0.35"/><pyroImpulse shape="box" width="2" height="2" depth="2" time="0.5" density="1"/></pyro>"#;

#[test]
fn pyro_is_an_exclusive_native_volume_source_with_thermal_output() {
    let object = format!(r#"<object3D id="cloud" primitive="volume">{PYRO}<medium blackbody="true"/></object3D>"#);
    assert!(codes(&object).is_empty(), "{:?}", codes(&object));
    assert!(codes(&object.replace("primitive=\"volume\"", "primitive=\"box\"")).contains(&"VOL3".into()));
    assert!(codes(&object.replace("primitive=\"volume\"", "primitive=\"volume\" volume=\"cache\""))
        .contains(&"VOL1".into()));
    assert!(codes(&object.replace(PYRO, &format!("{PYRO}{PYRO}"))).contains(&"VOL1".into()));
}

#[test]
fn pyro_grid_shape_and_source_windows_are_validated() {
    let object = format!(r#"<object3D id="cloud" primitive="volume">{PYRO}</object3D>"#);
    for (from, to, code) in [
        ("voxelSize=\"1\"", "voxelSize=\"3\"", "PYRO1"),
        ("end=\"0.35\"", "end=\"0.1\"", "PYRO2"),
        ("shape=\"sphere\" radius=\"2\"", "shape=\"box\"", "PYRO3"),
    ] {
        assert!(codes(&object.replace(from, to)).contains(&code.into()), "{:?}", codes(&object.replace(from, to)));
    }
    for bad in ["dt=\"0\"", "dt=\"NaN\""] {
        assert!(!codes(&object.replace("dt=\"0.1\"", bad)).is_empty());
    }
}

#[test]
fn impulses_have_fixed_event_times_and_ambient_temperature_is_bounded() {
    let object = format!(r#"<object3D id="cloud" primitive="volume">{PYRO}</object3D>"#);
    let animated=object.replace("time=\"0.5\" density=\"1\"/>",r#"time="0.5" density="1"><animate property="time"><key time="0" value="0.5"/><key time="1" value="1.5"/></animate></pyroImpulse>"#);
    assert!(!codes(&animated).is_empty(), "event time cannot be animated to fire an impulse repeatedly");
    assert!(!codes(&object.replace("dt=\"0.1\"", "dt=\"0.1\" ambientTemperature=\"50001\"")).is_empty());
}

#[test]
fn pyro_numeric_rules_follow_xsd_whitespace_lexical_rules() {
    let object = format!(r#"<object3D id="cloud" primitive="volume">{PYRO}</object3D>"#);
    assert!(codes(&object.replace("voxelSize=\"1\"", "voxelSize=\" 1 \"")).is_empty());
    assert!(codes(&object.replace("end=\"0.35\"", "end=\" 0.1 \"")).contains(&"PYRO2".into()));
    assert!(codes(&object.replace("radius=\"2\"", "radius=\"2\" scaleX=\" 0 \"")).contains(&"PYRO4".into()));
}

#[test]
fn mesh_sources_require_mesh_assets_and_static_topology_selection() {
    let object = format!(r#"<object3D id="cloud" primitive="volume">{PYRO}</object3D>"#);
    let object = object.replace("shape=\"sphere\" radius=\"2\"", "shape=\"mesh\" mesh=\"shape\"");
    let xml = format!(
        r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/><assets><mesh id="shape" src="shape.obj"/></assets><composition>{object}</composition></scene>"#
    );
    let check = |xml: &str| {
        sr_model::validate_str(xml, &sr_model::LoadOptions::without_assets())
            .diagnostics
            .into_iter()
            .map(|d| d.code)
            .collect::<Vec<_>>()
    };
    assert!(check(&xml).is_empty(), "{:?}", check(&xml));
    assert!(check(&xml.replace(" mesh=\"shape\"", "")).contains(&"PYRO5".into()));
    assert!(check(&xml.replace("<mesh ", "<image ")).contains(&"PYRO5".into()));
    let animated = xml.replace(
        "end=\"0.35\"/>",
        r#"end="0.35"><animate property="mesh"><key time="0" value="shape"/></animate></pyroSource>"#,
    );
    assert!(check(&animated).contains(&"PYRO6".into()));
}

#[test]
fn pyro_force_selection_uses_typed_physics_references() {
    let xml = format!(
        r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/><composition>
      <object3D id="cloud" primitive="volume">{PYRO}</object3D></composition>
      <physics><forceField id="wind" type="wind" forceX="1"/></physics></scene>"#
    )
    .replace("<pyro ", "<pyro forceFields=\"wind\" useForceFields=\"true\" ");
    let valid = sr_model::validate_str(&xml, &sr_model::LoadOptions::without_assets());
    assert!(!valid.has_errors(), "{:?}", valid.diagnostics);
    let invalid = sr_model::validate_str(
        &xml.replace("forceFields=\"wind\"", "forceFields=\"cloud\""),
        &sr_model::LoadOptions::without_assets(),
    );
    assert!(invalid.diagnostics.iter().any(|d| d.code == "R34"), "{:?}", invalid.diagnostics);
}

#[test]
fn pyro_colliders_require_geometry_and_static_shape_parameters() {
    let object = format!(
        r#"<object3D id="solid" primitive="box" width="2" height="2" depth="2"/>
      <object3D id="cloud" primitive="volume">{PYRO}</object3D>"#
    )
    .replace("<pyro ", "<pyro colliders=\"solid\" ");
    assert!(codes(&object).is_empty(), "{:?}", codes(&object));
    assert!(codes(&object.replace("colliders=\"solid\"", "colliders=\"cloud\"")).contains(&"PYRO7".into()));
    let animated = object.replace("depth=\"2\"/>", r#"depth="2"><animate property="width"><key time="0" value="2"/><key time="1" value="4"/></animate></object3D>"#);
    assert!(codes(&animated).contains(&"PYRO8".into()));
    assert!(codes(&animated.replace("property=\"width\"", "property=\"scaleX\"")).is_empty());
}

#[test]
fn procedural_solids_are_colliders_but_shape_changes_are_not_silently_frozen() {
    let check = |solid: &str| {
        codes(&format!(
            r#"{solid}<particles3D id="dust" colliders="solid"/>
        <object3D id="cloud" primitive="volume"><pyro width="4" height="4" depth="4" voxelSize="1" colliders="solid"/></object3D>"#
        ))
    };
    for solid in [
        r#"<object3D id="solid" primitive="text" text="O"/>"#,
        r#"<object3D id="solid" primitive="extrude" path="M0 0 L2 0 L2 2 Z"/>"#,
        r#"<object3D id="solid" primitive="clay"><blob/></object3D>"#,
    ] {
        assert!(check(solid).is_empty(), "{solid}: {:?}", check(solid));
    }
    for solid in [
        r#"<object3D id="solid" primitive="text" text="O"><animate property="text"><key time="0" value="OO"/></animate></object3D>"#,
        r#"<object3D id="solid" primitive="extrude" path="M0 0 L2 0 L2 2 Z"><animate property="bevel"><key time="0" value="1"/></animate></object3D>"#,
        r#"<object3D id="solid" primitive="clay"><blob><animate property="x"><key time="0" value="2"/></animate></blob></object3D>"#,
        r#"<object3D id="solid" primitive="clay" boil="12" fingerprints="0.5"><blob/></object3D>"#,
        r#"<object3D id="solid" primitive="clay" boil=" +12 " fingerprints=" +0.5 "><blob/></object3D>"#,
    ] {
        let errors = check(solid);
        assert!(errors.contains(&"PYRO8".into()) && errors.contains(&"P3D6".into()), "{solid}: {errors:?}");
    }
    assert!(check(
        r#"<object3D id="solid" primitive="clay"><blob/>
        <animate property="x"><key time="0" value="0"/><key time="1" value="3"/></animate></object3D>"#
    )
    .is_empty());
}

#[test]
fn pyro_solver_is_a_jacobi_or_multigrid_enumeration() {
    let object = format!(r#"<object3D id="cloud" primitive="volume">{PYRO}</object3D>"#);
    for solver in ["jacobi", "multigrid"] {
        let c = codes(&object.replace("<pyro ", &format!(r#"<pyro solver="{solver}" "#)));
        assert!(c.is_empty(), "{solver}: {c:?}");
    }
    for solver in ["", "cg", "Multigrid", "jacobi multigrid"] {
        let c = codes(&object.replace("<pyro ", &format!(r#"<pyro solver="{solver}" "#)));
        assert!(c.contains(&"S06".into()), "{solver:?}: {c:?}");
    }
}

#[test]
fn pyro_advection_is_a_semilagrangian_or_maccormack_enumeration() {
    let object = format!(r#"<object3D id="cloud" primitive="volume">{PYRO}</object3D>"#);
    for advection in ["semilagrangian", "maccormack"] {
        let c = codes(&object.replace("<pyro ", &format!(r#"<pyro advection="{advection}" "#)));
        assert!(c.is_empty(), "{advection}: {c:?}");
    }
    let both = object.replace("<pyro ", r#"<pyro solver="multigrid" advection="maccormack" "#);
    assert!(codes(&both).is_empty(), "{:?}", codes(&both));
    for advection in ["", "MacCormack", "semi-lagrangian", "rk4"] {
        let c = codes(&object.replace("<pyro ", &format!(r#"<pyro advection="{advection}" "#)));
        assert!(c.contains(&"S06".into()), "{advection:?}: {c:?}");
    }
}
