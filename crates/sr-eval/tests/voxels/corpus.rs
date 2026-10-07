//! The valid documents of the corpus that make bodies of cells are evaluated, and as what the document says: the evaluator no longer refuses them by name.

use sr_eval::{EvalOptions, Evaluator};

fn corpus(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/valid").join(format!("{name}.scene.xml"))
}

#[test]
fn the_valid_documents_of_the_corpus_that_make_bodies_of_cells_are_evaluated_as_what_they_say() {
    // (document, the object, whether its frames say what it is (a body that a crater or a fracture can break does), whether its fracture makes no body)
    // The asset of the corpus has five cells: cut by seeds or by planes or by material with two cells the least of a body, none of the pieces is one (by material
    // the five cells have five palette indices), and the evaluator says so by name (the document is valid, and a fracture that makes no body is not one).
    for (name, object, says, no_body) in [
        ("voxel-body", "block", false, false),
        ("voxel-crater", "ground", true, false),
        ("voxel-crater-signed-scale", "ground", true, false),
        ("voxel-ejecta", "ground", true, false),
        ("voxel-fracture", "block", true, true),
        ("voxel-fracture-planes", "block", true, true),
        ("voxel-fracture-labels", "block", true, true),
    ] {
        let doc = sr_model::load_file(corpus(name), &sr_model::LoadOptions::default())
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let ev = Evaluator::new(&doc, &EvalOptions::default()).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        for t in [0.0, doc.duration() / 2.0] {
            let frame = ev.evaluate(t);
            let none =
                frame.failures.iter().any(|f| f.contains(object) && f.contains("no piece of the fracture is a body"));
            assert_eq!(none, no_body, "{name} at {t}: {:?}", frame.failures);
            assert_eq!(frame.failures.len(), usize::from(no_body), "{name} at {t}: {:?}", frame.failures);
            if no_body {
                continue;
            }
            let node = frame.nodes.iter().find(|n| &*n.id == object).unwrap_or_else(|| panic!("{name}: no {object}"));
            assert!(node.pose3.is_some(), "{name}: {object} is a body of the world");
            assert_eq!(node.voxels.is_some(), says, "{name} at {t}");
        }
    }
}
