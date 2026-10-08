//! A fracture by stress is valid (FRX13 to FRX15) and is not evaluated yet: the evaluator refuses it by name (E24), as one error naming the object, and does not make it as a
//! fracture by impact with no source, which would come apart at the first frame. The wiring of the mode deletes the refusal and this file.

use sr_eval::{EvalOptions, Evaluator};

fn refusal(xml: &str) -> Option<Vec<(String, String)>> {
    let doc =
        sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    match Evaluator::new(&doc, &EvalOptions::default()) {
        Ok(_) => None,
        Err(report) => Some(report.diagnostics.iter().map(|d| (d.code.to_string(), d.message.clone())).collect()),
    }
}

const HEAD: &str = r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="1"/><assets><voxelAsset id="model" src="x.vox"/></assets><composition><object3D id="ball" primitive="sphere" radius="1" y="-8"><rigidBody mass="1"/></object3D>"##;
const TAIL: &str = "</composition></scene>";

fn block(fracture: &str) -> String {
    format!(
        r#"{HEAD}<object3D id="block" primitive="voxels" voxels="model" cellSize="2"><rigidBody density="2400"/>{fracture}</object3D>{TAIL}"#
    )
}

#[test]
fn a_fracture_by_stress_is_refused_by_name_once_and_one_by_impact_is_not() {
    let refused = refusal(&block(r#"<fracture mode="stress" strength="2e6" pieces="8"/>"#)).expect("refused");
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_eq!(refused[0].0, "E24");
    assert!(refused[0].1.contains("fracture mode stress is not evaluated yet by this build"), "{}", refused[0].1);
    // the others are as they were
    assert_eq!(refusal(&block(r#"<fracture source="ball" pieces="8"/>"#)), None);
    assert_eq!(refusal(&block(r#"<fracture mode="impact" source="ball" pieces="8"/>"#)), None);
}

#[test]
fn a_fracture_by_stress_in_an_included_document_is_refused_too() {
    // an include is templated apart and its own diagnostics are not the main document's: the refusal has to cross it, or the nodes that the include makes would be built as a fracture by impact
    let dir = std::env::temp_dir().join(format!("sr-eval-stress-include-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("part.scene.xml"),
        r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="1"/><assets><voxelAsset id="model" src="x.vox"/></assets><composition><object3D id="inner" primitive="voxels" voxels="model" cellSize="2"><rigidBody density="2400"/><fracture mode="stress" strength="2e6" pieces="8"/></object3D></composition></scene>"#,
    )
    .unwrap();
    let main = r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="1"/><composition><include id="inc" src="part.scene.xml"/></composition></scene>"#;
    let doc = sr_model::load_str(
        main,
        &sr_model::LoadOptions { base_dir: Some(dir.clone()), ..sr_model::LoadOptions::without_assets() },
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let refused = match Evaluator::new(&doc, &EvalOptions::default()) {
        Ok(_) => panic!("the fracture by stress in the include was evaluated"),
        Err(report) => report.diagnostics.iter().map(|d| (d.code.to_string(), d.message.clone())).collect::<Vec<_>>(),
    };
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_eq!(refused[0].0, "E24");
    let _ = std::fs::remove_dir_all(&dir);
}
