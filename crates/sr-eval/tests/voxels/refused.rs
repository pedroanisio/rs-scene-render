//! A body of cells in the scene is valid (the schema of 1.3 takes it) and is not evaluated yet: the evaluator says so by name, once, as one error, so that
//! a document that asks for one does not render as something else (a sphere of mass 1, a burst of no particles, a fracture of nothing).
//!
//! The wiring of bodies, cuts and fractures of cells deletes [`sr_eval::pending::BODIES_OF_CELLS`], and the documents of this file with it.

use sr_eval::{EvalOptions, Evaluator};

fn corpus(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/valid").join(format!("{name}.scene.xml"))
}

/// The codes of the errors that the evaluator refuses a document with, none if it takes it.
fn refusal(xml: &str) -> Option<Vec<(String, String)>> {
    let doc =
        sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    match Evaluator::new(&doc, &EvalOptions::default()) {
        Ok(_) => None,
        Err(report) => Some(report.diagnostics.iter().map(|d| (d.code.to_string(), d.message.clone())).collect()),
    }
}

const HEAD: &str = r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="1"/><assets><voxelAsset id="model" src="x.vox"/></assets><materials><material id="stone" baseColor="#808080"/></materials><composition><object3D id="ball" primitive="sphere" radius="1" y="-8"><rigidBody mass="1"/></object3D>"##;
const TAIL: &str = "</composition></scene>";

fn scene(object: &str) -> String {
    format!("{HEAD}{object}{TAIL}")
}

#[test]
fn the_valid_documents_of_the_corpus_that_make_bodies_of_cells_are_refused_by_name() {
    for name in [
        "voxel-body",
        "voxel-crater",
        "voxel-crater-signed-scale",
        "voxel-ejecta",
        "voxel-fracture",
        "voxel-fracture-planes",
        "voxel-fracture-labels",
    ] {
        let doc = sr_model::load_file(corpus(name), &sr_model::LoadOptions::default())
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let refused = match Evaluator::new(&doc, &EvalOptions::default()) {
            Ok(_) => panic!("{name}: was evaluated"),
            Err(report) => {
                report.diagnostics.iter().map(|d| (d.code.to_string(), d.message.clone())).collect::<Vec<_>>()
            }
        };
        // one error, one code, and the words that say what is not there
        assert_eq!(refused.len(), 1, "{name}: {refused:?}");
        assert_eq!(refused[0].0, "E23", "{name}");
        assert!(
            refused[0].1.contains("bodies of cells are not evaluated yet by this build"),
            "{name}: {}",
            refused[0].1
        );
        assert!(
            refused[0].1.contains("block") || refused[0].1.contains("ground"),
            "{name}: names the object: {}",
            refused[0].1
        );
    }
}

#[test]
fn an_object_of_cells_that_is_not_a_body_is_evaluated_as_it_was() {
    // drawn only
    assert_eq!(
        refusal(&scene(r#"<object3D id="b" primitive="voxels" voxels="model" cellSize="2" material="stone"/>"#)),
        None
    );
    // a body that is a box, as the schema allows and the evaluator has always read it
    assert_eq!(
        refusal(&scene(
            r#"<object3D id="b" primitive="voxels" voxels="model" cellSize="2" material="stone"><rigidBody shape="box" mass="3"/></object3D>"#
        )),
        None
    );
}

#[test]
fn the_cells_for_the_collider_said_or_by_default_are_refused_and_so_are_a_crater_and_a_fracture() {
    for (what, body) in [
        ("default", r#"<rigidBody density="2400"/>"#),
        ("auto", r#"<rigidBody shape="auto" density="2400"/>"#),
        ("voxels", r#"<rigidBody shape="voxels" density="2400"/>"#),
        (
            "crater",
            r#"<rigidBody type="static" density="2400"/><crater id="pit" source="ball" targetMaterial="softRock"/>"#,
        ),
        ("fracture", r#"<rigidBody density="2400"/><fracture source="ball" pieces="4"/>"#),
    ] {
        let xml = scene(&format!(
            r#"<object3D id="b" primitive="voxels" voxels="model" cellSize="2" material="stone">{body}</object3D>"#
        ));
        let refused = refusal(&xml).unwrap_or_else(|| panic!("{what}: was evaluated"));
        assert_eq!(refused.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(), ["E23"], "{what}: {refused:?}");
    }
}
