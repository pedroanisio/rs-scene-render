//! A document of the corpus without 1.6 syntax evaluates to the same frame graph as 1.5 and as 1.6; one with 1.6 syntax
//! (added by an SREP branch) is refused as 1.5 by its version rule only, one listed in GATES_1_6.

use std::path::PathBuf;

/// The version rules that refuse 1.6 syntax in earlier documents, one per SREP that adds syntax to 1.6.
const GATES_1_6: &[&str] = &["V13"];

fn with_version(xml: &str, v: &str) -> Option<String> {
    let at = xml.find("<scene")?;
    let head = &xml[at..at + xml[at..].find('>')?];
    let i = head.find("version=\"")? + 9;
    let j = i + head[i..].find('"')?;
    Some(format!("{}{v}{}", &xml[..at + i], &xml[at + j..]))
}

#[test]
fn the_corpus_evaluates_alike_at_1_5_and_1_6() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/valid");
    let mut files: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".scene.xml"))
        .collect();
    files.sort();
    let mut compared = 0;
    for f in files {
        let xml = std::fs::read_to_string(&f).unwrap();
        let (Some(a), Some(b)) = (with_version(&xml, "1.5"), with_version(&xml, "1.6")) else { continue };
        let opts = sr_model::LoadOptions { verify_assets: false, base_dir: f.parent().map(Into::into) };
        let (Ok(da), Ok(db)) = (sr_model::load_str(&a, &opts), sr_model::load_str(&b, &opts)) else {
            // a document that is not valid as 1.5 (version gates such as V8) is not valid as 1.6 either, unless it
            // holds 1.6 syntax, which 1.5 refuses by a version rule of GATES_1_6 only
            if sr_model::load_str(&b, &opts).is_ok() {
                let r = sr_model::validate_str(&a, &opts);
                let errors: Vec<&str> = r
                    .diagnostics
                    .iter()
                    .filter(|d| d.severity == sr_model::Severity::Error)
                    .map(|d| d.code.as_str())
                    .collect();
                assert!(
                    !errors.is_empty() && errors.iter().all(|c| GATES_1_6.contains(c)),
                    "{}: valid as 1.6, refused as 1.5 by {errors:?}",
                    f.display()
                );
            } else {
                assert!(sr_model::load_str(&a, &opts).is_err(), "{}", f.display());
            }
            continue;
        };
        let (Ok(ea), Ok(eb)) =
            (sr_eval::Evaluator::new(&da, &Default::default()), sr_eval::Evaluator::new(&db, &Default::default()))
        else {
            continue;
        };
        let d = ea.program().duration;
        for t in [0.0, d * 0.5] {
            let (ga, gb) = (ea.evaluate_layout(t), eb.evaluate_layout(t));
            assert_eq!(
                serde_json::to_string(&ga).unwrap(),
                serde_json::to_string(&gb).unwrap(),
                "{} at {t} s",
                f.display()
            );
        }
        compared += 1;
    }
    // most corpus documents are 1.3 documents with volumes or voxels, which V8 and VOX1 keep to 1.3: 29 of them are
    // valid and compile at 1.5 (CI, 2026-10-10)
    assert!(compared >= 25, "{compared} documents compared");
}
