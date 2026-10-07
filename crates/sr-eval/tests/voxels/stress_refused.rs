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
