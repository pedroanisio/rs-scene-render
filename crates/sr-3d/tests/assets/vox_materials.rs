//! The materials of a `.vox` file as numbers, and the hash of them that a cache of surfaces keys on.

use sr_3d::occupancy::Limits;
use sr_3d::voxel::material::{fingerprint, parse_all, Kind, Material};
use sr_3d::voxel::vox;
use std::collections::BTreeMap;

fn dictionary(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn the_materials_of_the_fixture_are_read_as_numbers_by_palette_index() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vox");
    let bytes = std::fs::read(dir.join("materials.vox")).unwrap();
    let imported = vox::import(&bytes, None, Limits::default(), &vox::Bounds::default()).unwrap();
    let materials = parse_all(&imported.materials).unwrap();
    // the values that tools/make_vox.py wrote: 1 metal 0.9 rough 0.25; 2 glass ior 0.3 trans 0.5; 7 emit 0.75 flux 2
    assert_eq!(materials.keys().copied().collect::<Vec<_>>(), vec![1, 2, 7]);
    let metal = &materials[&1];
    assert_eq!((metal.kind.clone(), metal.metal, metal.rough, metal.trans), (Kind::Metal, Some(0.9), Some(0.25), None));
    let glass = &materials[&2];
    assert_eq!((glass.kind.clone(), glass.ior, glass.trans), (Kind::Glass, Some(0.3), Some(0.5)));
    let lamp = &materials[&7];
    assert_eq!((lamp.kind.clone(), lamp.emit, lamp.flux), (Kind::Emit, Some(0.75), Some(2.0)));
    // every key that the file had was read: nothing is left over
    assert!(materials.values().all(|m| m.rest.is_empty()));
}

#[test]
fn a_kind_that_is_not_there_is_diffuse_and_a_key_that_is_not_read_is_kept_as_spelt() {
    let m = Material::parse(9, &dictionary(&[("_rough", " 0.5 "), ("_name", "Stone"), ("_media_type", "_scatter")]))
        .unwrap();
    assert_eq!(m.kind, Kind::Diffuse);
    assert_eq!(m.rough, Some(0.5));
    assert_eq!(m.media_type.as_deref(), Some("_scatter"));
    assert_eq!(m.rest, dictionary(&[("_name", "Stone")]));
    let odd = Material::parse(9, &dictionary(&[("_type", "_plasma")])).unwrap();
    assert_eq!(odd.kind, Kind::Other("_plasma".into()));
}

#[test]
fn the_index_of_refraction_is_the_ri_or_one_plus_the_ior() {
    let ri = Material::parse(1, &dictionary(&[("_ri", "1.5")])).unwrap();
    let ior = Material::parse(1, &dictionary(&[("_ior", "0.5")])).unwrap();
    assert_eq!(ri.refractive_index(), Some(1.5));
    assert_eq!(ior.refractive_index(), Some(1.5));
    assert_eq!(Material::parse(1, &dictionary(&[])).unwrap().refractive_index(), None);
    // a file that has both: `_ri` is the index itself
    let both = Material::parse(1, &dictionary(&[("_ri", "1.33"), ("_ior", "0.9")])).unwrap();
    assert_eq!(both.refractive_index(), Some(1.33));
}

#[test]
fn a_value_that_is_not_a_finite_number_is_an_error_that_names_the_index_the_key_and_the_value() {
    for bad in ["abc", "", "NaN", "inf", "1,5"] {
        let error = Material::parse(85, &dictionary(&[("_rough", bad)])).unwrap_err();
        assert!(
            error.contains("85") && error.contains("_rough") && error.contains(&format!("{bad:?}")),
            "{bad}: {error}"
        );
    }
}

#[test]
fn the_fingerprint_of_the_materials_changes_with_every_property_and_with_nothing_else() {
    let base: BTreeMap<u8, Material> = parse_all(&BTreeMap::from([
        (1, dictionary(&[("_type", "_metal"), ("_metal", "0.9"), ("_rough", "0.25")])),
        (7, dictionary(&[("_type", "_emit"), ("_emit", "0.75"), ("_note", "lamp")])),
    ]))
    .unwrap();
    let again: BTreeMap<u8, Material> = parse_all(&BTreeMap::from([
        (7, dictionary(&[("_note", "lamp"), ("_emit", "0.750"), ("_type", "_emit")])),
        (1, dictionary(&[("_rough", "0.25"), ("_metal", "0.9"), ("_type", "_metal")])),
    ]))
    .unwrap();
    // the same numbers spelt another way, in another order: the same hash
    assert_eq!(fingerprint(&base), fingerprint(&again));
    let mut seen = std::collections::BTreeSet::from([fingerprint(&base)]);
    let mut changed = |edit: &dyn Fn(&mut BTreeMap<u8, Material>)| {
        let mut m = base.clone();
        edit(&mut m);
        assert!(seen.insert(fingerprint(&m)), "an edit did not change the fingerprint");
    };
    changed(&|m| m.get_mut(&1).unwrap().rough = Some(0.25 + f64::EPSILON));
    changed(&|m| m.get_mut(&1).unwrap().rough = None);
    changed(&|m| m.get_mut(&1).unwrap().metal = Some(0.91));
    changed(&|m| m.get_mut(&1).unwrap().kind = Kind::Glass);
    changed(&|m| m.get_mut(&7).unwrap().rest.insert("_note".into(), "lamps".into()).map(|_| ()).unwrap());
    changed(&|m| m.get_mut(&7).unwrap().media_type = Some("_sss".into()));
    changed(&|m| {
        let lamp = m.remove(&7).unwrap();
        m.insert(8, lamp);
    });
    changed(&|m| {
        m.remove(&7);
    });
    changed(&|m| m.get_mut(&1).unwrap().g = Some(0.0));
}
