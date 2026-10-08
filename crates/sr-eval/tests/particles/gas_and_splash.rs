//! An emitter that is dragged by a smoke and falls into an ocean reads the smoke at steps beyond the frame's own:
//! the smoke must still be simulated once for each of its steps, in whatever order the frame and the particles ask.

use sr_eval::Evaluator;

const GRAVITY: f64 = 9.80665;
const FLIGHT: f64 = 1.4888;
const MASS: f64 = 90478.0;

/// The impact scene of the splash tests, with a smoke the ejecta are dragged by.
fn scene() -> String {
    let (vx, vy) = (100.0 * 60f64.to_radians().cos(), 100.0 * 60f64.to_radians().sin());
    let launch = vy - GRAVITY * FLIGHT;
    let (x, y) = (-vx * FLIGHT, -2.0 - launch * FLIGHT - 0.5 * GRAVITY * FLIGHT * FLIGHT);
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="8"/><composition>
          <object3D id="rock" primitive="sphere" radius="2" x="{x}" y="{y}">
            <rigidBody shape="sphere" mass="{MASS}" velocityX="{vx}" velocityY="{launch}" restitution="0" linearDamping="0" angularDamping="0"/>
          </object3D>
          <object3D id="ground" primitive="plane" width="160" height="160" segments="160" y="0" rotationX="-90">
            <crater id="pit" source="rock" targetMaterial="softRock"/>
            <rigidBody type="static" shape="auto"/>
          </object3D>
          <object3D id="cloud" primitive="volume" y="-8">
            <pyro width="16" height="16" depth="16" voxelSize="2" dt="0.1" boundary="open" pressureIterations="100" pressureTolerance="0.001"/>
          </object3D>
          <particles3D id="debris" rate="0" lifetime="8" dt="0.0416666666666667" gravityY="9.80665" drag="0.05" gas="cloud" maxParticles="1000" size="0.34">
            <burst crater="pit" count="1000"/>
          </particles3D>
          <ocean id="sea" y="1" width="400" depth="400" cellSize="4" dt="0.0416666666666667" boundary="open" splash="debris"/>
        </composition>
        <physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none" fixInternalEdges="true"/></scene>"##
    )
}

fn evaluator(xml: &str) -> Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn the_smoke_is_simulated_once_for_each_of_its_steps_when_particles_read_it_and_fall_into_an_ocean() {
    let ev = evaluator(&scene());
    let steps = sr_eval::pyro::steps_on_this_thread;
    let before = steps();
    let last = 3.0;
    let mut t = 0.0;
    while t <= last + 1e-9 {
        let frame = ev.evaluate(t);
        assert!(
            frame.problems.is_empty() && frame.failures.is_empty(),
            "t = {t}: {:?} {:?}",
            frame.problems,
            frame.failures
        );
        t += 0.25;
    }
    let simulated = steps() - before;
    // the smoke's steps are 0.1 s, and a frame at `last` needs those up to it and one more for the particles' last step
    let needed = (last / 0.1_f64).ceil() as u64 + 2;
    println!("GAS+SPLASH smoke steps simulated {simulated}, distinct steps needed up to {needed}");
    assert!(simulated <= needed, "{simulated} smoke steps simulated for {needed} distinct ones");
}
