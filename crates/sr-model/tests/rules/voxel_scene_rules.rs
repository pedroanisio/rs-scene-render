//! A body of cells in the scene: the rigid body, the crater and the fracture of an object of primitive voxels, and the burst of its crater.

fn codes(object: &str, extra: &str) -> Vec<String> {
    let xml = format!(
        r##"<scene version="1.3"><project width="32" height="32" fps="30" duration="1"/><assets><voxelAsset id="model" src="castle.vox"/></assets><materials><material id="stone" baseColor="#808080"/></materials><composition><object3D id="ball" primitive="sphere" radius="1"><rigidBody mass="1"/></object3D>{object}{extra}</composition></scene>"##
    );
    sr_model::validate_str(&xml, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}

const CELLS: &str = r#"primitive="voxels" voxels="model" cellSize="2""#;

fn body(attributes: &str, children: &str, object: &str) -> Vec<String> {
    codes(&format!(r#"<object3D id="b" {CELLS} {object}><rigidBody {attributes}/>{children}</object3D>"#), "")
}

#[test]
fn a_body_of_cells_has_a_density_and_no_mass_and_nothing_that_belongs_to_another_collider() {
    assert!(body(r#"density="2400""#, "", "").is_empty());
    assert!(body(r#"shape="voxels" density="2400""#, "", "").is_empty());
    assert!(body(r#"shape="auto" density="2400" type="static""#, "", "").is_empty());
    assert!(body("", "", "").contains(&"VOX10".into()));
    assert!(body(r#"density="2400" mass="3""#, "", "").contains(&"VOX10".into()));
    // a box collider on an object of cells is a box: no density, no slots
    assert!(body(r#"shape="box""#, "", "").is_empty());
    assert!(body(r#"shape="box" density="2400""#, "", "").contains(&"VOX9".into()));
    // shape voxels belongs to the primitive
    let plain = codes(r#"<object3D id="b" primitive="box"><rigidBody shape="voxels"/></object3D>"#, "");
    assert!(plain.contains(&"VOX8".into()), "{plain:?}");
    // the enumerations and the range of the slots: the schema's own code for each, and no other, the body being otherwise right (static, so that CRT5 is
    // not what makes the list not empty). A density of 0 is refused when the number is read ("0 must be greater than 0", code XML), before any rule
    for (bad, code) in [
        (r#"anchor="top""#, "S06"),
        (r#"fragmentOverflow="drop""#, "S06"),
        (r#"maxFragments="0""#, "S06"),
        (r#"maxFragments="4097""#, "S06"),
        (r#"fragmentMinCells="0""#, "S06"),
        (r#"density="0""#, "XML"),
    ] {
        let c = body(
            &format!(r#"type="static" density="2400" {bad}"#),
            r#"<crater id="pit" source="ball" targetMaterial="softRock"/>"#,
            "",
        );
        assert_eq!(c, [code], "{bad}");
    }
}

#[test]
fn the_slots_the_overflow_and_the_anchor_belong_to_a_body_that_has_something_to_break() {
    let crater = r#"<crater id="pit" source="ball" targetMaterial="softRock"/>"#;
    let fracture = r#"<fracture source="ball" pieces="4"/>"#;
    let every = r#"density="2400" maxFragments="4096" fragmentMinCells="3" fragmentOverflow="dust""#;
    assert!(body(&format!(r#"type="static" {every} anchor="largest""#), crater, "").is_empty());
    assert!(body(every, fracture, "").is_empty());
    for lonely in [r#"maxFragments="8""#, r#"fragmentMinCells="2""#, r#"fragmentOverflow="error""#] {
        assert!(body(&format!(r#"density="2400" {lonely}"#), "", "").contains(&"VOX11".into()), "{lonely}");
    }
    // an anchor is for what a crater leaves, not a fracture
    assert!(body(r#"density="2400" anchor="base""#, fracture, "").contains(&"VOX12".into()));
    // a crater or a fracture needs the same scale on every axis, and a body has one or the other
    for stretch in [r#"scaleX="2""#, r#"scaleY="0.5" scaleZ="1""#] {
        assert!(body(r#"type="static" density="2400""#, crater, stretch).contains(&"VOX13".into()), "{stretch}");
        assert!(body("density=\"2400\"", fracture, stretch).contains(&"VOX13".into()), "{stretch}");
    }
    assert!(body(r#"type="static" density="2400""#, crater, r#"scaleX="3" scaleY="3" scaleZ="3""#).is_empty());
    // a number with a plus sign is a number (xs:double has it), and the same one written twice ways is the same scale
    for same in [r#"scaleX="+2" scaleY="2" scaleZ="2""#, r#"scaleX=" +2.0 " scaleY="2e0" scaleZ="+2""#] {
        assert!(body(r#"type="static" density="2400""#, crater, same).is_empty(), "{same}");
        assert!(body("density=\"2400\"", fracture, same).is_empty(), "{same}");
    }
    assert!(body(r#"type="static" density="2400""#, crater, r#"scaleX="+2" scaleY="3""#).contains(&"VOX13".into()));
    assert!(body(r#"type="static" density="2400""#, &format!("{crater}{fracture}"), "").contains(&"VOX14".into()));
}

#[test]
fn a_body_that_a_crater_or_a_fracture_breaks_has_the_cells_for_its_collider() {
    let crater = r#"<crater id="pit" source="ball" targetMaterial="softRock"/>"#;
    let fracture = r#"<fracture source="ball" pieces="4"/>"#;
    // a mesh or a box for the collider of an object of cells that something breaks is not a body of cells, and the cut would mean nothing: that the
    // scale was not uniform is not even looked at then (VOX13 is about the cells), so this is its own rule
    for shape in ["trimesh", "box", "sphere", "convex-hull"] {
        let ground = body(&format!(r#"type="static" shape="{shape}""#), crater, r#"scaleX="2""#);
        assert!(ground.contains(&"VOX15".into()), "crater, {shape}: {ground:?}");
        let block = body(&format!(r#"shape="{shape}""#), fracture, r#"scaleX="2""#);
        assert!(block.contains(&"VOX15".into()), "fracture, {shape}: {block:?}");
    }
    // no body at all is no body of cells either
    let bare = codes(&format!(r#"<object3D id="b" {CELLS} y="2">{crater}</object3D>"#), "");
    assert!(bare.contains(&"VOX15".into()), "{bare:?}");
    // the cells, said or by default, are fine, and an object of cells with nothing to break may have any collider
    for shape in ["", r#"shape="auto""#, r#"shape="voxels""#] {
        assert!(body(&format!(r#"type="static" density="2400" {shape}"#), crater, "").is_empty(), "{shape}");
    }
    assert!(body(r#"shape="box""#, "", "").is_empty());
}

#[test]
fn a_crater_in_cells_is_cut_by_an_impact_once_and_has_no_analytic_rim() {
    let ground = |crater: &str| body(r#"type="static" density="2400""#, crater, "");
    assert!(ground(r#"<crater id="pit" source="ball" targetMaterial="softRock" capture="true"/>"#).is_empty());
    for (extra, code) in [
        (r#"mantle="true""#, "CRT13"),
        (r#"bulking="1.2" mantle="true""#, "CRT13"),
        (r#"repose="30""#, "CRT13"),
        // start and end are the growth of a crater with no source (CRT6 refuses them with one, and CRT15 gives the crater of cells one), curve is
        // the curve of that growth, which only CRT14 refuses
        (r#"start="1""#, "CRT6"),
        (r#"end="2""#, "CRT6"),
        (r#"curve="linear""#, "CRT14"),
    ] {
        let c = ground(&format!(r#"<crater id="pit" source="ball" targetMaterial="softRock" {extra}/>"#));
        assert!(c.contains(&code.into()), "{extra}: {c:?}");
        // and nothing else of the three says it twice
        assert_eq!(c.iter().filter(|k| ["CRT6", "CRT13", "CRT14"].contains(&k.as_str())).count(), 1, "{extra}: {c:?}");
    }
    assert!(ground(r#"<crater id="pit" radius="4"/>"#).contains(&"CRT15".into()));
    // the rigid body of the owner may be the cells (CRT5), and still not a dynamic one: that, and only that, is what is wrong with this one
    let dynamic = body(r#"density="2400""#, r#"<crater id="pit" source="ball" targetMaterial="softRock"/>"#, "");
    assert_eq!(dynamic, ["CRT5"]);
}

#[test]
fn the_ejecta_of_a_crater_in_cells_are_its_cells_and_have_no_count() {
    let owner = |crater_id: &str| {
        format!(
            r#"<object3D id="g" {CELLS}><rigidBody type="static" density="2400"/><crater id="{crater_id}" source="ball" targetMaterial="softRock"/></object3D>"#
        )
    };
    let emitter = |burst: &str| format!(r#"<particles3D id="d" rate="0" lifetime="3">{burst}</particles3D>"#);
    assert!(codes(&owner("pit"), &emitter(r#"<burst crater="pit"/>"#)).is_empty());
    assert!(codes(&owner("pit"), &emitter(r#"<burst crater="pit" count="50"/>"#)).contains(&"CRT16".into()));
    // the particles are the cells that the cut throws, with the velocities of the cut: the launch angle and its spread have no meaning (CRT17)
    for angle in [r#"angle="30""#, r#"angleSpread="5""#, r#"angle="60" angleSpread="10""#] {
        let c = codes(&owner("pit"), &emitter(&format!(r#"<burst crater="pit" {angle}/>"#)));
        assert_eq!(c, ["CRT17"], "{angle}: {c:?}");
    }
    // a crater of a surface that is not cells keeps them, as it had
    let plane_pit = r#"<object3D id="p" primitive="plane" width="9" height="9"><rigidBody type="static"/><crater id="pit" source="ball" targetMaterial="softRock"/></object3D>"#;
    assert!(codes(plane_pit, &emitter(r#"<burst crater="pit" count="50" angle="30" angleSpread="5"/>"#)).is_empty());
    // a burst that is not of a crater of cells has its count, as it always had
    assert!(codes(&owner("pit"), &emitter(r#"<burst time="0.5" count="50"/>"#)).is_empty());
    assert!(codes(&owner("pit"), &emitter(r#"<burst time="0.5"/>"#)).contains(&"CRT16".into()));
    // the crater of a surface that is not cells
    let plane = r#"<object3D id="p" primitive="plane" width="9" height="9"><rigidBody type="static"/><crater id="pit" source="ball" targetMaterial="softRock"/></object3D>"#;
    assert!(codes(plane, &emitter(r#"<burst crater="pit" count="50"/>"#)).is_empty());
    assert!(codes(plane, &emitter(r#"<burst crater="pit"/>"#)).contains(&"CRT16".into()));
}

#[test]
fn a_fracture_of_cells_is_cut_by_seeds_planes_or_materials_and_has_no_interior() {
    let cut = |fracture: &str| body(r#"density="2400""#, fracture, "");
    assert!(cut(r#"<fracture source="ball"/>"#).is_empty());
    assert!(cut(r#"<fracture source="ball" partition="voronoi" pieces="12" seed="4"/>"#).is_empty());
    assert!(cut(r#"<fracture source="ball" partition="planes" planes="1 0 0 4"/>"#).is_empty());
    assert!(
        cut(r#"<fracture source="ball" partition="planes" planes="1 0 0 4 0 -1 0 2.5 0.5 0.5 0.5 -1"/>"#).is_empty()
    );
    assert!(cut(r#"<fracture source="ball" partition="labels" labels="material"/>"#).is_empty());
    // the planes: 63 are the most, a plane is four numbers and numbers are numbers
    let many = |n: usize| (0..n).map(|i| format!("1 0 0 {i}")).collect::<Vec<_>>().join(" ");
    assert!(cut(&format!(r#"<fracture source="ball" partition="planes" planes="{}"/>"#, many(63))).is_empty());
    // a plane has a direction (a normal that is all zeros cuts nothing: every cell is on one side of it) and numbers that are finite
    for planes in [
        many(64),
        "1 0 0".to_string(),
        "1 0 0 4 0".to_string(),
        "1 0 x 4".to_string(),
        String::new(),
        "0 0 0 4".to_string(),
        "1 0 0 4 0 0 0 2".to_string(),
        "1 0 0 1e999".to_string(),
        format!("1 0 0 {}", "9".repeat(400)),
    ] {
        let c = cut(&format!(r#"<fracture source="ball" partition="planes" planes="{planes}"/>"#));
        assert!(c.contains(&"FRX11".into()), "{planes:?}: {c:?}");
    }
    // the directions that are fine: any axis, a slanted normal, a zero offset, a normal that is not a unit vector
    for planes in ["0 0 1 0", "0 -1 0 -3.5", "3 4 0 12", "0 0 0.001 1"] {
        let c = cut(&format!(r#"<fracture source="ball" partition="planes" planes="{planes}"/>"#));
        assert!(c.is_empty(), "{planes:?}: {c:?}");
    }
    // what belongs to which
    assert!(cut(r#"<fracture source="ball" partition="planes"/>"#).contains(&"FRX11".into()));
    assert!(cut(r#"<fracture source="ball" planes="1 0 0 4"/>"#).contains(&"FRX11".into()));
    assert!(cut(r#"<fracture source="ball" partition="labels"/>"#).contains(&"FRX12".into()));
    assert!(cut(r#"<fracture source="ball" labels="material"/>"#).contains(&"FRX12".into()));
    assert!(cut(r#"<fracture source="ball" partition="planes" planes="1 0 0 4" seed="3"/>"#).contains(&"FRX10".into()));
    assert!(
        cut(r#"<fracture source="ball" partition="labels" labels="material" pieces="6"/>"#).contains(&"FRX10".into())
    );
    // no interior material for cells, and a mesh has to have one and has no partition
    assert!(cut(r#"<fracture source="ball" interiorMaterial="stone"/>"#).contains(&"FRX8".into()));
    assert!(cut(r#"<fracture source="ball" interiorUvScale="2"/>"#).contains(&"FRX8".into()));
    let mesh = |fracture: &str| {
        codes(&format!(r#"<object3D id="m" primitive="box"><rigidBody mass="2"/>{fracture}</object3D>"#), "")
    };
    assert!(mesh(r#"<fracture source="ball" interiorMaterial="stone"/>"#).is_empty());
    assert!(mesh(r#"<fracture source="ball"/>"#).contains(&"FRX3".into()));
    assert!(mesh(r#"<fracture source="ball" interiorMaterial="stone" partition="voronoi"/>"#).contains(&"FRX9".into()));
    assert!(mesh(r#"<fracture source="ball" interiorMaterial="stone" planes="1 0 0 4"/>"#).contains(&"FRX9".into()));
}

#[test]
fn a_fracture_by_stress_has_a_strength_a_dynamic_body_of_cells_and_no_time_or_impact() {
    let stress =
        |extra: &str| body(r#"density="2400""#, &format!(r#"<fracture mode="stress" strength="2e6" {extra}/>"#), "");
    // the fracture of cells by stress: no source (the loads are every load), a strength in pascals, the cuts of the partition as before
    assert!(stress("").is_empty());
    assert!(stress(r#"partition="planes" planes="1 0 0 4""#).is_empty());
    assert!(stress(r#"partition="voronoi" pieces="40" seed="3""#).is_empty());
    assert!(body(r#"density="2400""#, r#"<fracture mode="impact" source="ball"/>"#, "").is_empty());
    // the strength is required with the stress, and belongs to it
    for (fracture, what) in [
        (r#"<fracture mode="stress"/>"#, "no strength"),
        (r#"<fracture source="ball" strength="2e6"/>"#, "a strength with no mode"),
        (r#"<fracture mode="impact" source="ball" strength="2e6"/>"#, "a strength with the impact"),
    ] {
        let c = body(r#"density="2400""#, fracture, "");
        assert!(c.contains(&"FRX13".into()), "{what}: {c:?}");
    }
    // what fires the other fracture is not what fires this one
    for extra in [
        r#"source="ball""#,
        r#"at="1""#,
        r#"radialImpulse="5""#,
        r#"impulseX="1""#,
        r#"minImpulse="2" source="ball""#,
        r#"energyFraction="0.3" source="ball""#,
    ] {
        let c = stress(extra);
        assert!(c.contains(&"FRX15".into()), "{extra}: {c:?}");
    }
    // a body of cells and a dynamic one: a mesh, and a static body of cells, have nothing for it to break by
    let mesh = codes(
        r#"<object3D id="m" primitive="box"><rigidBody mass="2"/><fracture mode="stress" strength="2e6" interiorMaterial="stone"/></object3D>"#,
        "",
    );
    assert!(mesh.contains(&"FRX14".into()), "{mesh:?}");
    let fixed = body(r#"type="static" density="2400""#, r#"<fracture mode="stress" strength="2e6"/>"#, "");
    assert!(fixed.contains(&"FRX14".into()), "{fixed:?}");
    let moving = body(r#"type="kinematic" density="2400""#, r#"<fracture mode="stress" strength="2e6"/>"#, "");
    assert!(moving.contains(&"FRX14".into()), "{moving:?}");
    // the values the schema takes
    assert!(body(r#"density="2400""#, r#"<fracture mode="plastic" strength="2e6"/>"#, "").contains(&"S06".into()));
    assert!(!body(r#"density="2400""#, r#"<fracture mode="stress" strength="0"/>"#, "").is_empty());
    assert!(!body(r#"density="2400""#, r#"<fracture mode="stress" strength="-3"/>"#, "").is_empty());
}
