//! The valid documents of the corpus that make bodies of cells are evaluated, and as what the document says: the evaluator does not refuse them by name, and a valid
//! document is one that does what it says: none of them evaluates to a failure or a problem.

use sr_eval::{EvalOptions, Evaluator};

fn corpus(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/valid").join(format!("{name}.scene.xml"))
}

#[test]
fn the_valid_documents_of_the_corpus_that_make_bodies_of_cells_are_evaluated_as_what_they_say() {
    // (document, the object, whether its frames say what it is: a body that a crater or a fracture can break does)
    // The asset of the corpus (tests/corpus/media/block.vox) has 1024 cells in 16 x 4 x 16, and the fractures of the corpus say fragmentMinCells="1": every piece is a body, and the
    // document breaks into them (with the old asset of five cells, and a least of two, every piece was dust).
    for (name, object, says) in [
        ("voxel-body", "block", false),
        ("voxel-crater", "ground", true),
        ("voxel-crater-signed-scale", "ground", true),
        ("voxel-ejecta", "ground", true),
        ("voxel-fracture", "block", true),
        ("voxel-fracture-planes", "block", true),
        ("voxel-fracture-labels", "block", true),
    ] {
        let doc = sr_model::load_file(corpus(name), &sr_model::LoadOptions::default())
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let ev = Evaluator::new(&doc, &EvalOptions::default()).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        // the first frame, the middle and the last (a frame is at its own instant: the end of the duration is not one)
        for t in [0.0, doc.duration() / 2.0, doc.fps().frame_time(doc.frame_count() - 1)] {
            let frame = ev.evaluate(t);
            assert!(
                frame.failures.is_empty() && frame.problems.is_empty(),
                "{name} at {t}: {:?} {:?}",
                frame.failures,
                frame.problems
            );
            let node = frame.nodes.iter().find(|n| &*n.id == object).unwrap_or_else(|| panic!("{name}: no {object}"));
            assert!(node.pose3.is_some(), "{name}: {object} is a body of the world");
            assert_eq!(node.voxels.is_some(), says, "{name} at {t}");
        }
    }
}

#[test]
fn every_valid_document_of_the_corpus_with_cells_evaluates_clean() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/valid");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().unwrap().to_string_lossy().starts_with("voxel") && p.to_string_lossy().ends_with(".scene.xml")
        })
        .collect();
    files.sort();
    assert!(files.len() >= 10, "the documents of cells of the corpus: {files:?}");
    let mut refused = 0;
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let doc =
            sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        // the one document that is refused is the fracture by stress, by its own code (E24), until the wiring of the mode
        let ev = match Evaluator::new(&doc, &EvalOptions::default()) {
            Ok(ev) => ev,
            Err(report) => {
                assert!(
                    name == "voxel-fracture-stress.scene.xml" && report.diagnostics.iter().all(|d| d.code == "E24"),
                    "{name} is refused by something other than the fracture by stress: {report}"
                );
                refused += 1;
                continue;
            }
        };
        for t in [0.0, doc.duration() / 2.0, doc.fps().frame_time(doc.frame_count() - 1)] {
            let g = ev.evaluate(t);
            assert!(
                g.failures.is_empty() && g.problems.is_empty(),
                "{name} at {t} s: {:?} {:?}",
                g.failures,
                g.problems
            );
        }
    }
    assert_eq!(refused, 1, "only the fracture by stress is refused");
}
