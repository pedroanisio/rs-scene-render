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

#[test]
fn a_window_that_follows_its_plume_needs_an_open_domain_its_own_attributes_and_room_for_its_margin() {
    let follows = |attributes: &str| {
        format!(
            r#"<object3D id="cloud" primitive="volume">{}</object3D>"#,
            PYRO.replace("<pyro ", &format!("<pyro {attributes} "))
        )
    };
    let open = r#"boundary="open" follow="true""#;
    assert!(codes(&follows(&format!(r#"{open} followMargin="2" followLoss="0.001""#))).is_empty());
    assert!(codes(&follows(open)).is_empty());
    // follow is refused in a closed domain, the default one and the one that says so
    assert!(codes(&follows(r#"follow="true""#)).contains(&"PYRO9".into()));
    assert!(codes(&follows(r#"boundary="closed" follow="1""#)).contains(&"PYRO9".into()));
    // a false follow has nothing to say about the boundary, and its attributes are orphans
    assert!(codes(&follows(r#"follow="false""#)).is_empty());
    assert!(codes(&follows(r#"follow="false" followMargin="2""#)).contains(&"PYRO10".into()));
    assert!(codes(&follows(r#"boundary="open" followLoss="0.1""#)).contains(&"PYRO10".into()));
    // 8 cells across: a margin of 4 leaves no cell between the faces, a margin of 3 leaves two
    assert!(codes(&follows(&format!(r#"{open} followMargin="4""#))).contains(&"PYRO11".into()));
    assert!(codes(&follows(&format!(r#"{open} followMargin="3""#))).is_empty());
    // the loss is a share
    assert!(!codes(&follows(&format!(r#"{open} followLoss="1.5""#))).is_empty());
}

#[test]
fn a_source_near_an_open_face_is_not_warned_about_when_the_window_follows_its_plume() {
    let object = |attributes: &str| {
        format!(
            r#"<object3D id="cloud" primitive="volume"><pyro width="64" height="64" depth="64" voxelSize="1" dt="0.1" boundary="open" {attributes}><pyroSource shape="sphere" radius="2" densityRate="2" y="-26"/></pyro></object3D>"#
        )
    };
    assert!(codes(&object("")).contains(&"W02".into()), "{:?}", codes(&object("")));
    assert!(!codes(&object(r#"follow="true""#)).contains(&"W02".into()));
}

#[test]
fn a_blast_needs_an_open_domain_a_place_in_it_and_a_gas_that_is_one() {
    let blast = |pyro_attributes: &str, blast_attributes: &str| {
        codes(&format!(
            r#"<object3D id="cloud" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" {pyro_attributes}><pyroBlast time="0.3" energy="1000000" {blast_attributes}/></pyro><medium blackbody="true"/></object3D>"#
        ))
    };
    let open = r#"boundary="open""#;
    assert!(blast(open, "").is_empty(), "{:?}", blast(open, ""));
    assert!(blast(open, r#"x="4" y="-4" z="0" ambientDensity="0.9" ambientPressure="90000" gamma="1.67""#).is_empty());
    // a closed pyro (the default) cannot let the divergence of a blast out
    assert!(blast("", "").contains(&"PYC5".into()));
    assert!(blast(r#"boundary="closed""#, "").contains(&"PYC5".into()));
    // the place of the blast is in the domain: half the width, the height and the depth either way
    for place in [r#"x="4.5""#, r#"y="-4.01""#, r#"z="9""#] {
        assert!(blast(open, place).contains(&"PYC6".into()), "{place}");
    }
    // the types: no energy that is negative, no gas that is not one, no air of nothing
    for bad in [r#"energy="-1""#, r#"gamma="1""#, r#"gamma="3.5""#, r#"ambientDensity="0""#, r#"ambientPressure="-5""#]
    {
        let object = format!(
            r#"<object3D id="cloud" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" boundary="open"><pyroBlast time="0.3" energy="1000000" {bad}/></pyro><medium blackbody="true"/></object3D>"#
        )
        .replace(r#"energy="1000000" energy="-1""#, r#"energy="-1""#);
        assert!(!codes(&object).is_empty(), "{bad}");
    }
    // time and energy are required
    let missing = r#"<object3D id="cloud" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" boundary="open"><pyroBlast energy="1"/></pyro><medium blackbody="true"/></object3D>"#;
    assert!(!codes(missing).is_empty());
}
