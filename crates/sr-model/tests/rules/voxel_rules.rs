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

#[test]
fn a_voxels_object_with_cells_smaller_than_half_a_unit_has_a_warning_because_the_raster_shadow_does_not_scale() {
    let warned = |size: &str| {
        codes(&scene(FILE, &format!(r#"primitive="voxels" voxels="model" cellSize="{size}""#))).contains(&"W09".into())
    };
    for small in ["0.49", "0.25", "0.1", "0.02"] {
        assert!(warned(small), "{small}");
    }
    for fine in ["0.5", "1", "2", "10"] {
        assert!(!warned(fine), "{fine}");
    }
    assert!(!codes(&scene(FILE, r#"primitive="voxels" voxels="model""#)).contains(&"W09".into()), "the default is 1");
}
