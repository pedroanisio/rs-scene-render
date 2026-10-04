fn xml(ocean: &str) -> String {
    format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition>
          <object3D id="float" primitive="sphere" radius="0.5" y="-1"><rigidBody shape="sphere" mass="200"/></object3D>
          <ocean id="sea" width="8" depth="8" cellSize="0.5" bottomDepth="4" {ocean}/></composition>
          <physics pixelsPerMeter="1"/></scene>"#
    )
}
fn codes(xml: &str) -> Vec<String> {
    sr_model::validate_str(xml, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn the_water_can_act_on_the_bodies_it_carries() {
    assert!(codes(&xml(r#"colliders="float""#)).is_empty());
    assert!(codes(&xml(r#"colliders="float" bodyCoupling="none""#)).is_empty());
    assert!(codes(&xml(r#"colliders="float" bodyCoupling="buoyancy""#)).is_empty());
    assert!(codes(&xml(r#"colliders="float" bodyCoupling="buoyancy" bodyDrag="1.5""#)).is_empty());
    assert!(codes(&xml(r#"colliders="float" bodyCoupling="buoyancy" bodyDrag="0""#)).is_empty(), "no drag is allowed");
    assert!(!codes(&xml(r#"colliders="float" bodyCoupling="sinking""#)).is_empty());
    assert!(!codes(&xml(r#"colliders="float" bodyCoupling="buoyancy" bodyDrag="-1""#)).is_empty());
}

#[test]
fn coupling_needs_bodies_to_couple_and_drag_needs_coupling() {
    assert!(codes(&xml(r#"bodyCoupling="buoyancy""#)).contains(&"OCN8".into()));
    assert!(codes(&xml(r#"colliders="float" bodyDrag="1""#)).contains(&"OCN9".into()));
    assert!(codes(&xml(r#"colliders="float" bodyCoupling="none" bodyDrag="1""#)).contains(&"OCN9".into()));
}
