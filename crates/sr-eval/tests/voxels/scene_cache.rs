//! A physics cache holds poses, velocities and contacts, and has no place for the revisions and the cuts of bodies of cells: a world with one is not baked,
//! and a document with one does not name a cache, and says which attribute to take out.

use super::scene_body::{cube, evaluator, Dir};
use sr_3d::voxel::srvol;

const DOCUMENT: &str = r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="1"/>
    <assets><voxelAsset id="model" src="block.srvol"/></assets>
    <materials><material id="stone" baseColor="#808080"/></materials>
    <composition>
      <object3D id="block" primitive="voxels" voxels="model" material="stone"><rigidBody density="2400"/></object3D>
    </composition>
    <physics gravityY="0" pixelsPerMeter="1" CACHE/></scene>"##;

fn dir() -> Dir {
    let dir = Dir::new("cache");
    std::fs::write(dir.0.join("block.srvol"), srvol::write(&cube(4), 0.25).unwrap()).unwrap();
    dir
}

#[test]
fn a_world_with_a_body_of_cells_is_not_baked_into_a_physics_cache_and_the_message_says_why() {
    let dir = dir();
    let ev = evaluator(&dir, &DOCUMENT.replace("CACHE", ""));
    let error = ev.physics_cache().expect_err("a cache of cells is refused");
    assert!(error.contains("bodies of cells") && error.contains("revisions") && error.contains("cuts"), "{error}");
}

#[test]
fn a_document_with_a_body_of_cells_that_names_a_physics_cache_is_refused_by_the_attribute() {
    let dir = dir();
    std::fs::write(dir.0.join("world.cache"), b"not a cache").unwrap();
    let ev = evaluator(&dir, &DOCUMENT.replace("CACHE", r#"cache="world.cache""#));
    let frame = ev.evaluate(0.2);
    let said: Vec<&String> = frame.failures.iter().filter(|f| f.contains("physics@cache")).collect();
    assert_eq!(said.len(), 1, "{:?}", frame.failures);
    assert!(said[0].contains("bodies of cells") && said[0].contains("remove physics@cache"), "{}", said[0]);
    // it is the only failure: the file is not even read
    assert_eq!(frame.failures.len(), 1, "{:?}", frame.failures);
}
