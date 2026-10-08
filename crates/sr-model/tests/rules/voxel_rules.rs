//! The voxel asset and the voxels primitive: what the rules VOX1 to VOX7 and the types of the schema accept and refuse.

fn codes(xml: &str) -> Vec<String> {
    sr_model::validate_str(xml, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}

fn scene(asset: &str, object: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="32" height="32" fps="30" duration="1"/><assets><mesh id="shape" src="shape.glb"/>{asset}</assets><materials><material id="stone" baseColor="#808080"/><material id="moss" baseColor="#406040"/></materials><composition><object3D id="build" {object}/></composition></scene>"##
    )
}

const FILE: &str = r#"<voxelAsset id="model" src="castle.vox"/>"#;
const SOLID: &str = r#"primitive="voxels" voxels="model" cellSize="2""#;

fn codes_of(asset: &str, object: &str) -> Vec<String> {
    codes(&scene(asset, object)).into_iter().filter(|c| c.starts_with("VOX") || c.starts_with('S')).collect()
}

#[test]
fn a_file_asset_and_a_mesh_asset_are_valid_and_the_format_is_found_from_the_extension() {
    assert_eq!(codes_of(FILE, SOLID), Vec::<String>::new());
    assert_eq!(codes_of(r#"<voxelAsset id="model" fromMesh="shape" cellSize="0.5"/>"#, SOLID), Vec::<String>::new());
    assert_eq!(codes_of(r#"<voxelAsset id="model" src="castle.vox" model="2"/>"#, SOLID), Vec::<String>::new());
    assert_eq!(
        codes_of(r#"<voxelAsset id="model" src="grid.srvol" voxelGrid="labels"/>"#, SOLID),
        Vec::<String>::new()
    );
    // model belongs to a vox file and voxelGrid to an srvol file, by format or by extension
    assert!(codes_of(r#"<voxelAsset id="model" src="grid.srvol" model="1"/>"#, SOLID).contains(&"VOX2".into()));
    assert!(codes_of(r#"<voxelAsset id="model" src="castle.vox" voxelGrid="labels"/>"#, SOLID).contains(&"VOX2".into()));
    assert!(codes_of(r#"<voxelAsset id="model" src="castle.bin" format="vox" model="1"/>"#, SOLID).is_empty());
    assert!(codes_of(r#"<voxelAsset id="model" src="castle.bin" model="1"/>"#, SOLID).contains(&"VOX2".into()));
}

#[test]
fn the_limits_and_the_sizes_of_the_asset_are_those_of_its_types() {
    let bad = |attributes: &str| {
        !codes(&scene(&format!(r#"<voxelAsset id="model" src="castle.vox" {attributes}/>"#), SOLID)).is_empty()
    };
    assert!(!bad(r#"maxCells="67108864" maxMemoryMiB="4096""#));
    assert!(bad(r#"maxCells="67108865""#));
    assert!(bad(r#"maxMemoryMiB="4097""#));
    assert!(bad(r#"maxCells="0""#));
    assert!(bad(r#"format="obj""#));
    let from_mesh = |size: &str| {
        !codes(&scene(&format!(r#"<voxelAsset id="model" fromMesh="shape" cellSize="{size}"/>"#), SOLID)).is_empty()
    };
    assert!(!from_mesh("0.5") && from_mesh("0") && from_mesh("-1"));
}

#[test]
fn the_object_needs_its_asset_keeps_its_attributes_to_itself_and_excludes_the_other_sources() {
    assert!(codes_of(FILE, r#"primitive="voxels""#).contains(&"VOX4".into()));
    assert!(codes_of(FILE, r#"primitive="voxels" voxels="shape""#).contains(&"VOX4".into()));
    for orphan in [r#"voxels="model""#, r#"cellSize="2""#, r#"palette="file""#, r#"surface="blocks""#] {
        assert!(codes_of(FILE, &format!(r#"primitive="box" {orphan}"#)).contains(&"VOX5".into()), "{orphan}");
    }
    for excluded in [r#"mesh="shape""#, r#"volume="model""#, r#"text="a""#, r#"path="M0 0""#] {
        assert!(codes_of(FILE, &format!("{SOLID} {excluded}")).contains(&"VOX7".into()), "{excluded}");
    }
    assert!(!codes(&scene(FILE, &format!(r#"{SOLID} surface="smooth""#))).is_empty());
}

#[test]
fn the_palette_is_the_word_file_or_materials() {
    for ok in [r#"palette="file""#, r#"palette="stone""#, r#"palette="stone moss stone""#, r#"palette="  file ""#] {
        assert!(codes_of(FILE, &format!("{SOLID} {ok}")).is_empty(), "{ok}");
    }
    for bad in [r#"palette="stone nothing""#, r#"palette="file stone""#, r#"palette="shape""#] {
        assert!(codes_of(FILE, &format!("{SOLID} {bad}")).contains(&"VOX6".into()), "{bad}");
    }
    let many = vec!["stone"; 256].join(" ");
    assert!(codes_of(FILE, &format!(r#"{SOLID} palette="{many}""#)).contains(&"VOX6".into()));
    let fits = vec!["stone"; 255].join(" ");
    assert!(codes_of(FILE, &format!(r#"{SOLID} palette="{fits}""#)).is_empty());
}

#[test]
fn the_voxels_need_version_one_point_three() {
    let old = scene(FILE, SOLID).replace(r#"version="1.3""#, r#"version="1.2""#);
    assert!(codes(&old).contains(&"VOX1".into()));
}

#[test]
fn the_surface_memory_of_a_voxels_object_is_a_whole_number_of_mib_from_1_to_4096_and_belongs_to_it() {
    for ok in ["1", "128", "4096"] {
        assert!(codes_of(FILE, &format!(r#"{SOLID} surfaceMemoryMiB="{ok}""#)).is_empty(), "{ok}");
    }
    for bad in ["0", "4097", "1.5", "-3", "big", ""] {
        assert!(!codes_of(FILE, &format!(r#"{SOLID} surfaceMemoryMiB="{bad}""#)).is_empty(), "{bad:?}");
    }
    assert!(codes_of(FILE, r#"primitive="box" surfaceMemoryMiB="64""#).contains(&"VOX5".into()));
}

fn warned(asset: &str, object: &str) -> bool {
    codes(&scene(asset, object)).contains(&"W09".into())
}

#[test]
fn a_voxels_object_whose_cells_are_smaller_than_half_a_unit_in_the_scene_has_a_warning_because_the_raster_shadow_does_not_scale(
) {
    for small in ["0.49", "0.25", "0.1", "0.02"] {
        assert!(warned(FILE, &format!(r#"primitive="voxels" voxels="model" cellSize="{small}""#)), "{small}");
    }
    for fine in ["0.5", "1", "2", "10"] {
        assert!(!warned(FILE, &format!(r#"primitive="voxels" voxels="model" cellSize="{fine}""#)), "{fine}");
    }
    assert!(!warned(FILE, r#"primitive="voxels" voxels="model""#), "the default is 1");
}

#[test]
fn the_size_that_is_measured_is_the_cell_in_the_scene_the_size_of_the_cell_times_the_scale_of_the_object() {
    // cells of 1 on an object scaled to a quarter are cells of 0.25 in the scene; cells of 0.25 on an object scaled by 4 are cells of 1
    let object = |size: &str, scale: &str| {
        format!(
            r#"primitive="voxels" voxels="model" cellSize="{size}" scaleX="{scale}" scaleY="{scale}" scaleZ="{scale}""#
        )
    };
    assert!(warned(FILE, &object("1", "0.25")));
    assert!(!warned(FILE, &object("0.25", "4")));
    // the smallest side is the cell's: a squashed object has small cells along one axis
    assert!(warned(FILE, r#"primitive="voxels" voxels="model" cellSize="1" scaleY="0.25""#));
    // the size the asset gives (a model made from a mesh) is the one of the object that has none of its own
    let from_mesh = |size: &str| format!(r#"<voxelAsset id="model" fromMesh="shape" cellSize="{size}"/>"#);
    assert!(warned(&from_mesh("0.25"), r#"primitive="voxels" voxels="model""#));
    assert!(!warned(&from_mesh("0.25"), r#"primitive="voxels" voxels="model" scaleX="4" scaleY="4" scaleZ="4""#));
    assert!(
        !warned(&from_mesh("0.25"), r#"primitive="voxels" voxels="model" cellSize="2""#),
        "the object's own size wins"
    );
}

#[test]
fn the_warning_says_the_size_is_in_the_scene_and_does_not_advise_what_would_only_silence_it() {
    let xml = scene(FILE, r#"primitive="voxels" voxels="model" cellSize="0.25""#);
    let report = sr_model::validate_str(&xml, &sr_model::LoadOptions::without_assets());
    let text = report.diagnostics.iter().find(|d| d.code == "W09").map(|d| d.message.clone()).expect("W09");
    assert!(text.contains("in the scene"), "{text}");
    assert!(!text.contains("scale the object"), "{text}");
}

#[test]
fn the_numbers_of_the_size_are_read_as_the_schema_reads_a_double_and_the_limit_is_exact() {
    // xs:double takes a sign and spaces around the number: the cells are of 0.25 in all of these
    for object in [
        r#"primitive="voxels" voxels="model" cellSize="+0.25""#,
        r#"primitive="voxels" voxels="model" cellSize=" 0.25 ""#,
        r#"primitive="voxels" voxels="model" cellSize="1" scaleX="+0.25" scaleY="+0.25" scaleZ=" 0.25""#,
        r#"primitive="voxels" voxels="model" cellSize="0.49999""#,
    ] {
        assert!(warned(FILE, object), "{object}");
    }
    assert!(!warned(
        FILE,
        r#"primitive="voxels" voxels="model" cellSize="0.25" scaleX="+3" scaleY="3" scaleZ=" 3.0 ""#
    ));
    assert!(!warned(FILE, r#"primitive="voxels" voxels="model" cellSize="0.5""#), "0.5 is the limit and does not warn");
}
