//! A particle emitter whose fixed step is the interval between frames (24 fps, a step of 0.041666 s): the particles
//! of a frame are never older than the frame, so every one of them can be drawn.

use sr_eval::Evaluator;

fn scene(dt: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><materials><material id="gas" baseColor="#FFFFFF" unlit="true"/></materials><composition>
          <particles3D id="disk" shape="sphere" segments="4" material="gas" emitterShape="sphere" emitterRadius="9.5" scaleZ="0.03" rate="2000" lifetime="5.2" size="0.15" drag="4" forceFields="pull spin" seed="12" maxParticles="14000" dt="{dt}"/>
        </composition>
        <physics pixelsPerMeter="1"><forceField id="pull" type="radial" strength="8"/><forceField id="spin" type="vortex" strength="12"/></physics></scene>"##
    )
}

fn evaluator(xml: &str) -> Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn every_particle_of_a_frame_has_been_born_by_the_frame_for_any_fixed_step() {
    for dt in ["0.0166666667", "0.041666", "0.0416666667", "0.04166667", "0.0417", "0.0416666666666667"] {
        let ev = evaluator(&scene(dt));
        for k in 0..100 {
            let t = k as f64 / 24.0;
            let frame = ev.evaluate(t);
            assert!(
                frame.failures.is_empty() && frame.problems.is_empty(),
                "dt {dt}, t {t}: {:?} {:?}",
                frame.failures,
                frame.problems
            );
            let emitter = frame.nodes.iter().find(|n| &*n.id == "disk").unwrap().particles3d.as_ref().unwrap();
            for p in &emitter.frame.particles {
                assert!(
                    p.age(emitter.frame.time) >= 0.0,
                    "dt {dt}, frame {k}: particle {} born at {} after the frame at {}",
                    p.id,
                    p.birth,
                    emitter.frame.time
                );
                p.transform(emitter.frame.time).unwrap_or_else(|e| panic!("dt {dt}, frame {k}: {e}"));
            }
        }
    }
}
