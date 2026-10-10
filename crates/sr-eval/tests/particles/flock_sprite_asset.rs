//! A flock's `sprite` is an image asset of the program, as a particle emitter's is.

#[test]
fn a_flock_sprite_is_an_asset_of_the_program_without_a_layer() {
    let xml = r##"<scene version="1.2"><project width="64" height="64" fps="30" duration="2"/>
  <assets><image id="chip" src="chip.png" width="8" height="8"/></assets>
  <composition><flock id="swarm" width="64" height="64" count="10" shape="sprite" sprite="chip"/></composition></scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    assert!(ev.program().assets.contains_key("chip"), "{:?}", ev.program().assets.keys().collect::<Vec<_>>());
}
