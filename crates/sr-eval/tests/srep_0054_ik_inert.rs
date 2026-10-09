//! SREP 54, Semantics 3: a pole or soft reach on an IK chain that is not of two bones (FABRIK, or a single bone that
//! aims) has no effect and is reported as inert, at information severity; on a chain of two bones it is read.

fn findings(skeleton: &str) -> Vec<sr_model::Diagnostic> {
    let xml = format!(
        r#"<scene version="1.2"><project width="400" height="400" fps="10" duration="1"/><composition>
  <shape id="goal" shape="ellipse" x="50" y="60" width="6" height="6"/><shape id="knee-pole" shape="ellipse" x="0" y="100" width="6" height="6"/>
  <skeleton id="rig">{skeleton}</skeleton></composition></scene>"#
    );
    let d =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    ev.warnings().iter().filter(|w| w.message.contains("SREP 54")).cloned().collect()
}

const TWO: &str = r#"<bone id="a" length="10"/><bone id="b" parent="a" x="10" length="10"/>"#;
const THREE: &str = r#"<bone id="a" length="10"/><bone id="b" parent="a" x="10" length="10"/><bone id="c" parent="b" x="10" length="10"/>"#;
const IK: &str = r#"<transformConstraint type="ik" target="goal" pole="knee-pole" softness="0.3"/>"#;

#[test]
fn a_chain_of_two_reads_pole_and_softness() {
    assert!(findings(&format!("{TWO}{IK}")).is_empty());
}

#[test]
fn a_chain_of_three_ignores_them_and_says_so() {
    let f = findings(&format!("{THREE}{IK}"));
    assert_eq!(f.len(), 2, "{f:?}");
    assert!(f.iter().all(|d| d.severity == sr_model::Severity::Info && d.code == "E19"));
    assert!(f[0].message.contains("@pole") && f[0].message.contains("3 bone"), "{}", f[0].message);
    assert!(f[1].message.contains("@softness"), "{}", f[1].message);
}

#[test]
fn a_zero_length_base_bone_does_not_lengthen_the_chain() {
    // the solver drops leading bones of zero length: root at the origin of a, then a two-bone chain
    let chain = r#"<bone id="root" length="0"/><bone id="a" parent="root" length="10"/><bone id="b" parent="a" x="10" length="10"/>"#;
    assert!(findings(&format!("{chain}{IK}")).is_empty());
}

#[test]
fn a_single_bone_and_other_constraint_types_ignore_them() {
    let one = r#"<bone id="a" length="10"/>"#;
    assert_eq!(findings(&format!("{one}{IK}")).len(), 2);
    let look = r#"<transformConstraint type="look-at" target="goal" softness="0.3"/>"#;
    assert_eq!(findings(&format!("{TWO}{look}")).len(), 1);
    // the default softness is not worth a finding
    let plain = r#"<transformConstraint type="ik" target="goal" softness="0"/>"#;
    assert!(findings(&format!("{THREE}{plain}")).is_empty());
}
