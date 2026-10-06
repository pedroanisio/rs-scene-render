fn xml(pyro: &str) -> String {
    format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><composition>
          <object3D id="rock" primitive="sphere" radius="1" y="-8"><rigidBody mass="5"/></object3D>
          <object3D id="ground" primitive="plane" width="100" height="100" segments="32" y="2">
            <crater id="pit" source="rock" targetMaterial="softRock"/><rigidBody type="static"/></object3D>
          <object3D id="plain" primitive="plane" width="10" height="10" segments="8" y="2">
            <crater id="old" radius="4" rimWidth="1"/><rigidBody type="static"/></object3D>
          <object3D id="cloud" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1">{pyro}</pyro></object3D>
        </composition><physics pixelsPerMeter="1"/></scene>"#
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
fn smoke_can_be_a_consequence_of_a_crater_with_engine_parameters() {
    let valid = xml(
        r#"<pyroSource crater="pit" heatFraction="0.2" dustFraction="0.02" specificHeat="800" maxTemperature="3000"/>
           <pyroImpulse crater="pit" velocityY="-5"/>"#,
    );
    assert!(codes(&valid).is_empty(), "{:?}", codes(&valid));
    // an authored source and an authored impulse are as they were
    assert!(codes(&xml(r#"<pyroSource densityRate="1" start="0.5"/><pyroImpulse time="1" density="1"/>"#)).is_empty());
}

#[test]
fn a_source_from_a_crater_derives_what_would_otherwise_be_authored() {
    for attr in [
        "start",
        "end",
        "densityRate",
        "temperatureRate",
        "expansion",
        "radius",
        "x",
        "y",
        "z",
        "rotationX",
        "scaleZ",
        r#"shape="box""#,
    ] {
        let attr = if attr.contains('=') { attr.to_string() } else { format!(r#"{attr}="1""#) };
        let got = codes(&xml(&format!(r#"<pyroSource crater="pit" {attr}/>"#)));
        assert!(got.contains(&"PYC1".into()), "{attr}: {got:?}");
    }
    for attr in ["time", "density", "temperature", "expansion", "radius"] {
        let got = codes(&xml(&format!(r#"<pyroImpulse crater="pit" {attr}="1"/>"#)));
        assert!(got.contains(&"PYC1".into()), "{attr}: {got:?}");
    }
}

#[test]
fn the_engine_parameters_belong_to_a_source_from_a_crater() {
    for attr in [r#"heatFraction="0.1""#, r#"dustFraction="0.1""#, r#"specificHeat="1000""#, r#"maxTemperature="5000""#]
    {
        assert!(codes(&xml(&format!("<pyroSource {attr}/>"))).contains(&"PYC2".into()), "{attr}");
        assert!(codes(&xml(&format!(r#"<pyroImpulse time="1" {attr}/>"#))).contains(&"PYC2".into()), "{attr}");
    }
    for bad in [r#"heatFraction="1.5""#, r#"dustFraction="0""#, r#"specificHeat="0""#, r#"maxTemperature="60000""#] {
        assert!(!codes(&xml(&format!(r#"<pyroSource crater="pit" {bad}/>"#))).is_empty(), "{bad}");
    }
}

#[test]
fn an_impulse_needs_a_time_unless_a_crater_gives_it() {
    assert!(codes(&xml(r#"<pyroImpulse density="1"/>"#)).contains(&"PYC3".into()));
    assert!(codes(&xml(r#"<pyroImpulse crater="pit"/>"#)).is_empty());
}

#[test]
fn the_crater_must_exist_and_grow_from_an_impact() {
    assert!(codes(&xml(r#"<pyroSource crater="old"/>"#)).contains(&"PYC4".into()), "an authored crater has no impact");
    assert!(codes(&xml(r#"<pyroSource crater="ground"/>"#)).contains(&"PYC4".into()), "an object is not a crater");
    assert!(!codes(&xml(r#"<pyroSource crater="nothing"/>"#)).is_empty(), "a dangling reference");
}
