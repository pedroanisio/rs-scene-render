//! SREP 73: `flock/@orientToVelocity` is valid on every flock (schema 1.6); `false` on a streak flock does nothing,
//! since a streak is drawn along the velocity by definition, and is reported as `INERT-I16` at information severity.
//! The SREP's findings cases `srep-0073-streak-inert` and `srep-0073-default-no-finding`, and their neighbours.

use sr_model::{validate_str, Diagnostic, LoadOptions, Severity};

fn report(flock_attrs: &str, children: &str) -> Vec<Diagnostic> {
    let xml = format!(
        r##"<scene version="1.6"><project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
  <assets><image id="spr" src="srep73-halves.png" width="8" height="8"/></assets>
  <composition><flock id="f" count="1" seed="3" width="1" height="1" x="320" y="180" sprite="spr" size="40" {flock_attrs}>{children}</flock></composition></scene>"##
    );
    let r = validate_str(&xml, &LoadOptions::without_assets());
    assert!(!r.has_errors(), "the document must be valid:\n{r}");
    r.diagnostics
}

fn i16(flock_attrs: &str) -> Vec<Diagnostic> {
    report(flock_attrs, "").into_iter().filter(|d| d.code == "INERT-I16").collect()
}

#[test]
fn srep_0073_streak_inert() {
    let f = i16(r#"shape="streak" orientToVelocity="false""#);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].severity, Severity::Info);
    assert_eq!(f[0].path, "/scene/composition/flock");
    assert!(f[0].message.contains("orientToVelocity"), "{}", f[0].message);
    // streak is the default shape
    assert_eq!(i16(r#"orientToVelocity="false""#).len(), 1);
    assert_eq!(i16(r#"orientToVelocity="0""#).len(), 1);
}

#[test]
fn srep_0073_default_no_finding() {
    assert!(i16(r#"shape="sprite""#).is_empty());
    assert!(i16(r#"shape="streak""#).is_empty());
    // true on a streak is what a streak does anyway: it asks for nothing more
    assert!(i16(r#"shape="streak" orientToVelocity="true""#).is_empty());
    // false acts on discs and sprites
    assert!(i16(r#"shape="sprite" orientToVelocity="false""#).is_empty());
    assert!(i16(r#"shape="disc" orientToVelocity="false""#).is_empty());
}

#[test]
fn srep_0073_no_finding_when_the_shape_can_change() {
    // a variant may make the flock a sprite flock, where the attribute acts
    let xml = r##"<scene version="1.6"><project width="640" height="360" fps="24" duration="1"/>
  <parameters><variant id="v"><override target="f" property="shape" value="sprite"/></variant></parameters>
  <assets><image id="spr" src="srep73-halves.png" width="8" height="8"/></assets>
  <composition><flock id="f" count="1" width="1" height="1" sprite="spr" shape="streak" orientToVelocity="false"/></composition></scene>"##;
    let r = validate_str(xml, &LoadOptions::without_assets());
    assert!(!r.has_errors(), "{r}");
    assert!(r.diagnostics.iter().all(|d| d.code != "INERT-I16"), "{r}");
}
