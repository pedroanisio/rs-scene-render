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
        ("voxel-fracture-stress", "block", true),
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
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let doc =
            sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let ev = Evaluator::new(&doc, &EvalOptions::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
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
}

#[test]
fn the_ball_of_the_stress_document_hits_the_block_and_the_block_does_not_break_in_any_frame() {
    // the ball of 500 kg at 120 m/s hits the free block of 38 400 kg at 0.067 s and pushes it along (it gains about 1.5 m/s, the ball's momentum over the block's mass);
    // the block is free and gravity is a uniform field, which makes no stress: what loads it is the contact, and its principal tension stays under the strength of 2e6 Pa in
    // every frame (it is over 2e5: see the next test), so the block falls whole. (A rough estimate that supposes the block held at its ends reads 1e7 Pa: it is not held.)
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/valid");
    let doc =
        sr_model::load_file(&dir.join("voxel-fracture-stress.scene.xml"), &sr_model::LoadOptions::default()).unwrap();
    let ev = Evaluator::new(&doc, &EvalOptions::default()).unwrap();
    let mut last = 0.0;
    for n in 0..ev.frame_count() {
        let g = ev.evaluate_frame(n);
        assert!(g.failures.is_empty() && g.problems.is_empty(), "frame {n}: {:?} {:?}", g.failures, g.problems);
        let block = g.nodes.iter().find(|x| &*x.id == "block").unwrap();
        let v = block.voxels.as_ref().expect("cells");
        assert_eq!((v.revision, v.grid.count(), v.pieces.len()), (0, 1024, 0), "frame {n}: the block is whole");
        last = block.pose3.unwrap()[13];
    }
    let t = doc.fps().frame_time(doc.frame_count() - 1);
    let gain = last - 0.5 * 9.80665 * t * t;
    assert!((3.0..9.5).contains(&gain), "the block was pushed along by the ball: {gain} m over free fall in {t} s");
}

#[test]
fn the_block_of_the_stress_document_breaks_into_its_eight_pieces_at_a_tenth_of_its_strength() {
    // the impact's principal tension is between 2e5 and 2e6 Pa (measured: the document's strength is 2e6 and the block is whole; at 2e5 every joint gives at once and the
    // eight pieces are the 1024 cells, each once)
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/valid");
    let xml = std::fs::read_to_string(dir.join("voxel-fracture-stress.scene.xml")).unwrap();
    let weak = xml.replace("strength=\"2e6\"", "strength=\"2e5\"");
    assert_ne!(weak, xml);
    let doc = sr_model::load_str(&weak, &sr_model::LoadOptions { base_dir: Some(dir), ..Default::default() }).unwrap();
    let ev = Evaluator::new(&doc, &EvalOptions::default()).unwrap();
    let mut last = None;
    for n in 0..ev.frame_count() {
        let g = ev.evaluate_frame(n);
        assert!(g.failures.is_empty() && g.problems.is_empty(), "frame {n}: {:?} {:?}", g.failures, g.problems);
        last = Some(g);
    }
    let g = last.unwrap();
    let v = g.nodes.iter().find(|x| &*x.id == "block").unwrap().voxels.clone().expect("cells");
    assert_eq!(v.revision, 1, "all the joints that give do so together: one edit");
    assert_eq!(v.pieces.len(), 7, "the part with most cells stays the body and seven fall");
    let all: u64 = v.grid.count() + v.pieces.iter().map(|p| p.grid.count()).sum::<u64>();
    assert_eq!(all, 1024, "the body and its pieces are the cells of the block (no dust: fragmentMinCells is 1)");
}
