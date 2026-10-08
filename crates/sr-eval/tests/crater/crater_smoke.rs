//! Smoke that an impact causes: nothing in the document says when, where or how much, and it
//! grows with the energy of the impact.

use sr_eval::Evaluator;

struct Setup {
    /// Speed toward the ground, metres per second.
    down: f64,
    mass: f64,
    /// Attributes of the pyroSource that comes from the crater.
    source: &'static str,
    /// Extra children of the pyro volume.
    extra: &'static str,
    /// Attributes of the pyro volume.
    volume: &'static str,
}

impl Default for Setup {
    fn default() -> Self {
        Setup { down: 100.0, mass: 1500.0, source: "", extra: "", volume: "" }
    }
}

impl Setup {
    fn xml(&self) -> String {
        format!(
            r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
              <object3D id="rock" primitive="sphere" radius="0.5" y="40">
                <rigidBody shape="sphere" mass="{mass}" velocityY="{down}" restitution="0" linearDamping="0" angularDamping="0"/>
              </object3D>
              <object3D id="ground" primitive="plane" width="40" height="40" segments="64" y="60" rotationX="-90">
                <crater id="pit" source="rock" targetMaterial="softRock"/>
                <rigidBody type="static" shape="auto"/>
              </object3D>
              <object3D id="cloud" primitive="volume" y="55">
                <pyro width="16" height="16" depth="16" voxelSize="1" dt="0.05" boundary="open" {volume}>
                  <pyroSource crater="pit" {source}/>{extra}
                </pyro>
              </object3D>
            </composition>
            <physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"##,
            mass = self.mass,
            down = self.down,
            source = self.source,
            extra = self.extra,
            volume = self.volume,
        )
    }

    fn evaluator(&self) -> Evaluator {
        let doc =
            sr_model::load_str(&self.xml(), &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
        Evaluator::new(&doc, &Default::default()).unwrap()
    }
}

/// Total volume fraction of solids in the smoke, and its hottest temperature, at `t`.
fn smoke(ev: &Evaluator, t: f64) -> (f64, f64) {
    let frame = ev.evaluate(t);
    assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
    let node = frame.nodes.iter().find(|n| &*n.id == "cloud").unwrap();
    let volume = &node.sim_volume.as_ref().expect("native pyro volume").data;
    let density: f64 = volume.grid("density").unwrap().bricks().flat_map(|(_, b)| b.iter()).map(|v| *v as f64).sum();
    let hottest =
        volume.grid("temperature").unwrap().bricks().flat_map(|(_, b)| b.iter()).map(|v| *v as f64).fold(0.0, f64::max);
    (density, hottest)
}

#[test]
fn nothing_is_emitted_until_the_body_arrives_and_then_the_impact_fills_the_volume() {
    let ev = Setup { source: r#"heatFraction="1" dustFraction="0.0001""#, ..Setup::default() }.evaluator();
    // the rock lands near 0.19 s
    let (dust, hot) = smoke(&ev, 0.1);
    assert_eq!(dust, 0.0, "no smoke before the impact");
    assert!(hot <= 300.0 + 1e-6, "{hot}");
    let (dust, hot) = smoke(&ev, 1.5);
    assert!(dust > 0.0, "the impact makes dust");
    assert!(hot > 300.0, "and heats it: {hot}");
}

/// The volume of dust the law gives for the 1500 kg, 0.5 m rock arriving at `speed`.
fn law_dust(speed: f64, fraction: f64) -> f64 {
    use sr_sim::cratering::{crater, Impact, Material, Target};
    let volume = 4.0 / 3.0 * std::f64::consts::PI * 0.125;
    let c = crater(
        &Impact { mass: 1500.0, density: 1500.0 / volume, normal_speed: speed },
        &Target { material: Material::SoftRock, density: None, strength: None, gravity: 9.80665 },
    )
    .unwrap();
    fraction * c.ejecta_volume
}

/// The speed the rock reaches the ground at after falling 19.5 m from `down`.
fn arrival(down: f64) -> f64 {
    (down * down + 2.0 * 9.80665 * 19.5).sqrt()
}

#[test]
fn the_dust_is_the_fraction_of_the_volume_thrown_out_and_grows_with_the_energy() {
    // with no heat nothing blows the dust away, so the dust in the volume is the dust injected
    let dust: Vec<f64> = [60.0, 100.0, 150.0]
        .iter()
        .map(|&down| {
            smoke(
                &Setup { down, source: r#"heatFraction="0" dustFraction="0.01""#, ..Setup::default() }.evaluator(),
                1.5,
            )
            .0
        })
        .collect();
    println!("SMOKE dust for 60, 100, 150 m/s: {dust:?}");
    assert!(dust.windows(2).all(|w| w[0] < w[1]), "dust by speed: {dust:?}");
    for (d, down) in dust.iter().zip([60.0, 100.0, 150.0]) {
        // one metre voxels: the dust fraction summed over cells is the dust volume in cubic metres
        let want = law_dust(arrival(down), 0.01);
        assert!((d - want).abs() < 0.2 * want, "{down} m/s: {d} against {want}");
    }
    let heavier: Vec<f64> = [600.0, 1500.0, 4000.0]
        .iter()
        .map(|&mass| smoke(&Setup { mass, source: r#"heatFraction="0""#, ..Setup::default() }.evaluator(), 1.5).0)
        .collect();
    assert!(heavier.windows(2).all(|w| w[0] < w[1]), "dust by mass: {heavier:?}");
}

/// The hottest the smoke is above ambient shortly after a one-shot impulse from the crater.
fn heated_by(setup: Setup, attributes: &str) -> f64 {
    let xml =
        setup.xml().replace(r#"<pyroSource crater="pit" />"#, &format!(r#"<pyroImpulse crater="pit" {attributes}/>"#));
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    smoke(&Evaluator::new(&doc, &Default::default()).unwrap(), 0.4).1 - 300.0
}

#[test]
fn the_heat_grows_with_the_energy_and_is_what_the_dust_can_hold() {
    use sr_sim::cratering::{crater, smoke as law_smoke, Impact, Material, Target};
    let rises: Vec<f64> = [60.0, 100.0, 150.0]
        .iter()
        .map(|&down| heated_by(Setup { down, ..Setup::default() }, r#"heatFraction="1""#))
        .collect();
    println!("SMOKE temperature rise for 60, 100, 150 m/s: {rises:?}");
    assert!(rises.windows(2).all(|w| w[0] < w[1]), "heat by speed: {rises:?}");
    // the rise the law gives for the dust and the heat, before the smoke mixes
    let speed = arrival(100.0);
    let volume = 4.0 / 3.0 * std::f64::consts::PI * 0.125;
    let impact = Impact { mass: 1500.0, density: 1500.0 / volume, normal_speed: speed };
    let target = Target { material: Material::SoftRock, density: None, strength: None, gravity: 9.80665 };
    let c = crater(&impact, &target).unwrap();
    let params = sr_sim::cratering::SmokeParams { heat_fraction: 1.0, ..Default::default() };
    let made = law_smoke(&impact, speed, &c, 2100.0, &params).unwrap();
    assert!(
        rises[1] > 0.5 * made.temperature_rise && rises[1] < 1.05 * made.temperature_rise,
        "{} against the law's {}",
        rises[1],
        made.temperature_rise
    );
}

#[test]
fn a_slow_impact_in_physical_units_barely_heats_anything() {
    // at the engine's defaults a body that falls a few metres makes dust and next to no heat
    let slow = heated_by(Setup { down: 5.0, ..Setup::default() }, "");
    assert!(slow < 20.0, "the dust warms by a few kelvin at most: {slow}");
    let fast = heated_by(Setup { down: 150.0, ..Setup::default() }, "");
    assert!(fast > slow);
}

#[test]
fn the_expansion_comes_from_the_heating_so_hot_smoke_pushes_the_air_out() {
    // a hot, thin dust: the heat drives a divergence the solver projects, and it moves
    let ev = Setup { source: r#"heatFraction="1" dustFraction="0.0001""#, ..Setup::default() }.evaluator();
    let cold = Setup { source: r#"heatFraction="0" dustFraction="0.0001""#, ..Setup::default() }.evaluator();
    let moved = |ev: &Evaluator| {
        let frame = ev.evaluate(0.6);
        assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?}", frame.problems);
        let node = frame.nodes.iter().find(|n| &*n.id == "cloud").unwrap();
        let volume = &node.sim_volume.as_ref().unwrap().data;
        volume.grids().map(|(name, g)| (name.to_string(), g.sample_world([0.0, 5.0, 0.0]) as f64)).collect::<Vec<_>>()
    };
    let (hot, still) = (moved(&ev), moved(&cold));
    assert_ne!(hot, still, "heating changes the flow");
}

#[test]
fn an_impulse_from_the_crater_fires_once_at_the_impact() {
    let ev = Setup {
        source: r#"densityRate="0""#, // refused: derived
        ..Setup::default()
    };
    // a source that gives a derived attribute is not a valid document
    assert!(sr_model::load_str(&ev.xml(), &sr_model::LoadOptions::without_assets()).is_err());
    let only_impulse = Setup::default().xml().replace(
        r#"<pyroSource crater="pit" />"#,
        r#"<pyroImpulse crater="pit" heatFraction="1" dustFraction="0.0001"/>"#,
    );
    let doc = sr_model::load_str(&only_impulse, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = Evaluator::new(&doc, &Default::default()).unwrap();
    let (before, _) = smoke(&ev, 0.1);
    let (after, hot) = smoke(&ev, 1.0);
    assert_eq!(before, 0.0);
    assert!(after > 0.0 && hot > 300.0, "{after} {hot}");
}

#[test]
fn any_order_of_requests_and_a_fresh_evaluator_give_the_same_smoke_bit_for_bit() {
    let setup = Setup { source: r#"heatFraction="1" dustFraction="0.0001""#, ..Setup::default() };
    let ev = setup.evaluator();
    let times = [1.5, 0.1, 0.8, 0.3, 2.0, 0.5, 1.5];
    let first: Vec<_> = times.iter().map(|&t| smoke(&ev, t)).collect();
    let fresh = setup.evaluator();
    for (k, &t) in times.iter().enumerate() {
        let (a, b) = (first[k], smoke(&fresh, t));
        assert_eq!((a.0.to_bits(), a.1.to_bits()), (b.0.to_bits(), b.1.to_bits()), "t = {t}");
        let c = smoke(&ev, t);
        assert_eq!((a.0.to_bits(), a.1.to_bits()), (c.0.to_bits(), c.1.to_bits()), "again, t = {t}");
    }
}

#[test]
fn a_baked_cache_gives_the_same_smoke_as_the_live_simulation() {
    let setup = Setup { source: r#"heatFraction="1" dustFraction="0.0001""#, ..Setup::default() };
    let live = setup.evaluator();
    let bytes = live.physics_cache().unwrap();
    let path = std::env::temp_dir().join(format!("sr-crater-smoke-{}.physics", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    let xml = setup.xml().replace("<physics ", &format!("<physics cache=\"{}\" ", path.display()));
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let cached = Evaluator::new(&doc, &Default::default()).unwrap();
    for t in [0.1, 0.5, 1.5] {
        let (a, b) = (smoke(&live, t), smoke(&cached, t));
        assert_eq!((a.0.to_bits(), a.1.to_bits()), (b.0.to_bits(), b.1.to_bits()), "t = {t}");
    }
    std::fs::remove_file(path).ok();
}

#[test]
fn smoke_and_crater_are_found_inside_every_instance_of_a_symbol() {
    let xml = Setup { source: r#"heatFraction="1" dustFraction="0.0001""#, ..Setup::default() }.xml();
    let open = xml.find("<composition>").unwrap() + "<composition>".len();
    let end = xml.find("</composition>").unwrap();
    let inner = xml[open..end].to_string();
    let wrapped = format!(
        "{}<symbols><symbol id=\"assembly\" width=\"64\" height=\"64\">{inner}</symbol></symbols><composition><instance id=\"a\" symbol=\"assembly\"/><instance id=\"b\" symbol=\"assembly\" x=\"100\"/>{}",
        &xml[..xml.find("<composition>").unwrap()],
        &xml[end..]
    );
    let doc = sr_model::load_str(&wrapped, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let ev = Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(1.5);
    assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
    let clouds: Vec<_> = frame.nodes.iter().filter(|n| n.id.ends_with("cloud")).collect();
    assert_eq!(clouds.len(), 2);
    let dust: Vec<f64> = clouds
        .iter()
        .map(|n| {
            n.sim_volume
                .as_ref()
                .unwrap()
                .data
                .grid("density")
                .unwrap()
                .bricks()
                .flat_map(|(_, b)| b.iter())
                .map(|v| *v as f64)
                .sum()
        })
        .collect();
    assert!(dust[0] > 0.0 && (dust[0] - dust[1]).abs() < 1e-6 * dust[0], "each instance's smoke is its own: {dust:?}");
}

#[test]
fn a_ground_that_the_smoke_cannot_enter_does_not_take_any_of_its_dust() {
    // 0.25 m cells in an 8 m volume whose bottom is the ground, so that the source sphere (a metre or so) is
    // centred on the ground and about half of its cells are inside it
    let finer = |xml: String| {
        xml.replace(r#"y="55""#, r#"y="56""#).replace(
            r#"width="16" height="16" depth="16" voxelSize="1""#,
            r#"width="8" height="8" depth="8" voxelSize="0.25""#,
        )
    };
    let dust = |volume: &'static str| {
        let setup = Setup { source: r#"heatFraction="0" dustFraction="0.01""#, volume, ..Setup::default() };
        let doc = sr_model::load_str(&finer(setup.xml()), &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = Evaluator::new(&doc, &Default::default()).unwrap();
        // The ground's surface moves while the crater opens and pushes the air, and the dust with it out of
        // this small volume, so the dust to compare is the most there has been: the cells are a quarter of a
        // metre, a sixty-fourth of a cubic metre each.
        (0..20).map(|k| smoke(&ev, 0.3 + 0.05 * k as f64).0 / 64.0).fold(0.0, f64::max)
    };
    let (open, closed) = (dust(""), dust(r#"colliders="ground""#));
    let want = law_dust(arrival(100.0), 0.01);
    println!("SMOKE dust with and without the ground as a collider: {closed} and {open}, the law's {want}");
    assert!((open - want).abs() < 0.2 * want, "with nothing in the way: {open} against {want}");
    assert!((closed - want).abs() < 0.2 * want, "with the ground in the way: {closed} against {want}");
}

#[test]
fn smoke_from_a_crater_in_a_volume_that_is_stretched_on_any_axis_or_sheared_is_refused() {
    let xml = Setup::default().xml();
    let after = arrival(100.0) + 0.5;
    let refused = |xml: String| {
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
        let frame = Evaluator::new(&doc, &Default::default()).unwrap().evaluate(after);
        frame.failures.iter().chain(&frame.problems).any(|f| f.contains("uniformly scaled"))
    };
    // the same volume at the same scale on every axis is fine (a scale of 2), and a stretch of any one axis is refused, z included (the check looked at x and y)
    assert!(!refused(xml.replace(
        r#"<object3D id="cloud" primitive="volume" y="55">"#,
        r#"<object3D id="cloud" primitive="volume" y="55" scaleX="2" scaleY="2" scaleZ="2">"#
    )));
    for stretch in [r#"scaleX="2""#, r#"scaleY="2""#, r#"scaleZ="2""#] {
        let object = format!(r#"<object3D id="cloud" primitive="volume" y="55" {stretch}>"#);
        assert!(refused(xml.replace(r#"<object3D id="cloud" primitive="volume" y="55">"#, &object)), "{stretch}");
    }
    // a shear: a parent turned by 45 degrees and a child scaled by 0.8 on x and 1.5118578920369088 on y, which makes the three images of the axes of the volume
    // one length (1) and not at right angles
    let sheared = xml
        .replace(
            r#"<object3D id="cloud" primitive="volume" y="55">"#,
            r#"<group id="turn" rotation="45"><object3D id="cloud" primitive="volume" y="55" scaleX="0.8" scaleY="1.5118578920369088">"#,
        )
        .replace("</object3D>\n            </composition>", "</object3D></group>\n            </composition>");
    assert!(refused(sheared));
}
