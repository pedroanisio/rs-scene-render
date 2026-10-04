//! Particles dragged along by a smoke: with `gas` an emitter's particles feel the velocity of the
//! smoke volume it names, at their place and time, through the emitter's own drag.

use sr_eval::Evaluator;

struct Setup {
    /// Speed the whole domain is pushed to along x at the start, m/s.
    push: f64,
    /// The emitter's drag, 1/s.
    drag: f64,
    /// Attributes of the emitter besides the ones it always has.
    emitter: &'static str,
    /// Attributes of the pyro volume.
    volume: &'static str,
    /// Where the emitter is along x.
    x: f64,
    /// Domain cells on a side.
    cells: u32,
}

impl Default for Setup {
    fn default() -> Self {
        Setup { push: 5.0, drag: 2.0, emitter: r#"gas="cloud""#, volume: "", x: 0.0, cells: 16 }
    }
}

impl Setup {
    fn xml(&self) -> String {
        let n = self.cells;
        format!(
            r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="4"/><composition>
              <object3D id="cloud" primitive="volume">
                <pyro width="{n}" height="{n}" depth="{n}" voxelSize="1" dt="0.1" boundary="open" pressureIterations="200" pressureTolerance="1e-6" {volume}>
                  <pyroImpulse shape="box" width="{n}" height="{n}" depth="{n}" time="0" velocityX="{push}"/>
                </pyro>
              </object3D>
              <particles3D id="dust" x="{x}" rate="0" speed="0" lifetime="5" dt="0.1" drag="{drag}" maxParticles="10" {emitter}>
                <burst time="0.5" count="1"/>
              </particles3D>
            </composition><physics pixelsPerMeter="1"/></scene>"##,
            push = self.push,
            drag = self.drag,
            emitter = self.emitter,
            volume = self.volume,
            x = self.x,
        )
    }

    fn evaluator(&self) -> Evaluator {
        evaluator(&self.xml())
    }
}

fn evaluator(xml: &str) -> Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

/// The one particle at `t`: position, velocity, and the frame's hash.
fn particle(ev: &Evaluator, t: f64) -> ([f64; 3], [f64; 3], u64) {
    let frame = ev.evaluate(t);
    assert!(
        frame.problems.is_empty() && frame.failures.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.problems,
        frame.failures
    );
    let dust = frame.nodes.iter().find(|n| &*n.id == "dust").unwrap().particles3d.as_ref().expect("particles");
    let p = dust.frame.particles.first().expect("a particle");
    (p.position, p.velocity, dust.key)
}

fn bits(p: &([f64; 3], [f64; 3], u64)) -> Vec<u64> {
    p.0.iter().chain(&p.1).map(|v| v.to_bits()).chain([p.2]).collect()
}

#[test]
fn a_particle_in_a_gas_follows_p_double_prime_equals_k_times_the_gass_velocity_less_its_own() {
    // The smoke settles to speeds of its own after the push (the open faces take some of it, and it falls off
    // slowly), so the gas's speed is read from a particle with a drag so strong that it follows the gas at
    // once, and the closed form is integrated against that history.
    let follower = Setup { drag: 40.0, cells: 32, ..Setup::default() }.evaluator();
    let times: Vec<f64> = (0..16).map(|k| 0.6 + 0.1 * k as f64).collect();
    let gas: Vec<f64> = times.iter().map(|&t| particle(&follower, t).1[0]).collect();
    println!("GAS the gas's speed along the path: {:.4} to {:.4}", gas[gas.len() - 1], gas[0]);
    assert!(gas.iter().all(|&u| u > 3.0 && u <= 5.0), "{gas:?}");
    let u = |t: f64| {
        let x = ((t - 0.6) / 0.1).clamp(0.0, 15.0);
        let (k, f) = (x.floor() as usize, x - x.floor());
        gas[k] * (1.0 - f) + gas[(k + 1).min(15)] * f
    };
    let setup = Setup { cells: 32, ..Setup::default() };
    let ev = setup.evaluator();
    // integrate v' = k (u(t) - v), x' = v from the birth at 0.5 s by small steps (fourth order)
    let (mut v, mut x, mut t) = (0.0, 0.0, 0.5);
    let h = 0.001;
    let mut by_age = Vec::new();
    while t < 2.0 + 1e-9 {
        let f = |t: f64, v: f64| setup.drag * (u(t) - v);
        let (k1, k2) = (f(t, v), f(t + 0.5 * h, v + 0.5 * h * f(t, v)));
        let k3 = f(t + 0.5 * h, v + 0.5 * h * k2);
        let k4 = f(t + h, v + h * k3);
        let next = v + h / 6.0 * (k1 + 2.0 * k2 + 2.0 * k3 + k4);
        x += 0.5 * (v + next) * h;
        v = next;
        t += h;
        if ((t - 0.5) * 10.0 - ((t - 0.5) * 10.0).round()).abs() < 1e-6 {
            by_age.push((t, v, x));
        }
    }
    for (t, want_v, want_x) in by_age.into_iter().filter(|(t, ..)| [1.2, 1.5, 2.0].iter().any(|s| (s - t).abs() < 1e-6))
    {
        let (position, velocity, _) = particle(&ev, t);
        println!(
            "GAS t {t}: v {:.4} (integrated {want_v:.4}), x {:.4} (integrated {want_x:.4})",
            velocity[0], position[0]
        );
        assert!((velocity[0] - want_v).abs() < 0.03 * want_v, "t {t}: {} against {want_v}", velocity[0]);
        assert!((position[0] - want_x).abs() < 0.03 * want_x, "t {t}: {} against {want_x}", position[0]);
        assert!(velocity[1].abs() < 1e-6 && velocity[2].abs() < 1e-6, "{velocity:?}");
    }
}

#[test]
fn without_the_reference_or_the_drag_the_gas_does_nothing_and_the_world_is_as_it_was() {
    // no gas: still air, the particle never moves
    let free = Setup { emitter: "", ..Setup::default() }.evaluator();
    assert_eq!(particle(&free, 2.0).1, [0.0; 3]);
    // gas and no drag: no coupling
    let none = Setup { drag: 0.0, ..Setup::default() }.evaluator();
    assert_eq!(particle(&none, 2.0).1, [0.0; 3]);
    // an emitter 500 units away never meets the volume: the same bits with and without the reference
    let away = |emitter: &'static str| Setup { x: 500.0, emitter, ..Setup::default() }.evaluator();
    let (with, without) = (away(r#"gas="cloud""#), away(""));
    for t in [0.7, 1.5, 2.5] {
        assert_eq!(bits(&particle(&with, t)), bits(&particle(&without, t)), "t = {t}");
    }
}

#[test]
fn the_particle_goes_farther_with_the_gass_speed_and_with_the_drag() {
    let by_speed: Vec<f64> =
        [1.0, 2.5, 5.0, 10.0].map(|push| particle(&Setup { push, ..Setup::default() }.evaluator(), 1.5).0[0]).to_vec();
    assert!(by_speed.windows(2).all(|w| w[0] < w[1]), "by the gas's speed: {by_speed:?}");
    let by_drag: Vec<f64> =
        [0.5, 1.0, 2.0, 4.0].map(|drag| particle(&Setup { drag, ..Setup::default() }.evaluator(), 1.5).0[0]).to_vec();
    assert!(by_drag.windows(2).all(|w| w[0] < w[1]), "by the drag: {by_drag:?}");
}

#[test]
fn any_order_a_fresh_evaluator_and_a_smoke_that_kept_no_checkpoint_give_the_same_bits() {
    let times = [1.5, 0.7, 2.5, 1.0, 1.5, 3.0, 0.5];
    let setup = Setup::default();
    let ev = setup.evaluator();
    let first: Vec<_> = times.iter().map(|&t| bits(&particle(&ev, t))).collect();
    let fresh = setup.evaluator();
    let discarding = Setup { volume: r#"checkpointMemoryMiB="1""#, ..Setup::default() }.evaluator();
    for k in [4, 0, 6, 2, 5, 1, 3] {
        assert_eq!(first[k], bits(&particle(&fresh, times[k])), "fresh, t = {}", times[k]);
        assert_eq!(first[k], bits(&particle(&discarding, times[k])), "no smoke checkpoint, t = {}", times[k]);
    }
    assert_eq!(first[0], first[4], "the same instant twice");
}

#[test]
fn a_particle_that_leaves_the_volume_is_dragged_to_rest_by_the_air_outside_it() {
    // born 2 units inside the +x face of a 16-cell volume, it is carried out at the gas's 5 m/s and, outside, slowed
    // by still air with the same coefficient, with no jump in what it feels at the face
    let ev = Setup { x: 6.0, ..Setup::default() }.evaluator();
    let series: Vec<([f64; 3], [f64; 3])> =
        (0..25).map(|k| particle(&ev, 0.5 + 0.1 * k as f64)).map(|(p, v, _)| (p, v)).collect();
    let left = series.iter().position(|(p, _)| p[0] > 8.0 + 1.0).expect("it leaves the volume");
    assert!(left > 3, "it spent some time inside first: {left}");
    let fastest = series.iter().map(|(_, v)| v[0]).fold(0.0, f64::max);
    assert!(fastest > 2.5 && fastest <= 5.0 + 1e-6, "{fastest}");
    // after the instant it is a cell outside it only slows
    assert!(series[left..].windows(2).all(|w| w[1].1[0] <= w[0].1[0] + 1e-9), "{:?}", &series[left..]);
    // and the change of its velocity from step to step is smooth through the boundary: no step is bigger
    // than the largest acceleration, k times the gas's speed, can make in a step
    for pair in series.windows(2) {
        let change = (pair[1].1[0] - pair[0].1[0]).abs();
        assert!(change <= 2.0 * 5.0 * 0.1 + 1e-6, "a jump of {change} in a step");
    }
}

#[test]
fn a_smoke_that_fails_fails_the_particles_by_name_and_does_not_leave_them_in_still_air() {
    // 64 cells a side cannot be held in a mebibyte
    let ev = Setup { volume: r#"maxMemoryMiB="1""#, cells: 64, ..Setup::default() }.evaluator();
    let frame = ev.evaluate(1.5);
    let said = frame.failures.iter().chain(&frame.problems).any(|m| m.contains("dust") && m.contains("cloud"));
    assert!(said, "{:?} {:?}", frame.failures, frame.problems);
}

/// Time to evaluate 20 000 particles to 3 s with and without the gas: `cargo test --release -p sr-eval --test
/// particles_gas cost -- --ignored --nocapture`.
#[test]
#[ignore = "timing measurement"]
fn cost_of_the_gas_per_particle_step() {
    let run = |emitter: &'static str, drag: f64| {
        let xml = Setup { emitter, drag, ..Setup::default() }
            .xml()
            .replace(r#"maxParticles="10""#, r#"maxParticles="30000""#)
            .replace(r#"<burst time="0.5" count="1"/>"#, r#"<burst time="0.5" count="20000"/>"#);
        let ev = evaluator(&xml.replace(r#"speed="0""#, r#"speed="2" spread="360""#));
        let started = std::time::Instant::now();
        let frame = ev.evaluate(3.0);
        assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
        started.elapsed().as_secs_f64()
    };
    let (without, idle, with) = (run("", 2.0), run(r#"gas="cloud""#, 0.0), run(r#"gas="cloud""#, 2.0));
    // 20 000 particles, 25 canonical steps of 0.1 s after the burst, two queries each (and one more per hit)
    let queries = 20_000.0 * 25.0 * 2.0;
    println!(
        "GAS cost: {without:.2} s without, {idle:.2} s naming it with no drag, {with:.2} s with the gas (the smoke included); about {:.0} ns a query beyond the smoke",
        1e9 * (with - idle).max(0.0) / queries
    );
}

#[test]
fn a_gas_is_found_in_the_scope_of_the_emitter_and_only_there() {
    // a symbol holding a volume and an emitter that names it, instantiated twice: each instance's emitter reads its own
    // volume, and the same document gives the same particle in both
    let inner = Setup::default().xml();
    let (open, end) =
        (inner.find("<composition>").unwrap() + "<composition>".len(), inner.find("</composition>").unwrap());
    let body = &inner[open..end];
    let wrapped = format!(
        "{}<symbols><symbol id=\"assembly\" width=\"64\" height=\"64\">{body}</symbol></symbols><composition><instance id=\"a\" symbol=\"assembly\"/><instance id=\"b\" symbol=\"assembly\" x=\"100\"/>{}",
        &inner[..inner.find("<composition>").unwrap()],
        &inner[end..]
    );
    let ev = evaluator(&wrapped);
    let frame = ev.evaluate(1.5);
    assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
    let moved: Vec<f64> = ["a/dust", "b/dust"]
        .iter()
        .map(|id| {
            let dust = frame.nodes.iter().find(|n| &*n.id == *id).unwrap_or_else(|| panic!("no {id}"));
            dust.particles3d.as_ref().unwrap().frame.particles[0].velocity[0]
        })
        .collect();
    assert!(
        moved[0] > 1.0 && (moved[0] - moved[1]).abs() < 1e-9 * moved[0],
        "each instance drags its own particle: {moved:?}"
    );
    // a gas that is only inside a symbol is not instantiated in the scope of an emitter outside it
    // the volume inside a symbol that nothing instantiates, the emitter outside it
    let volume_start = inner.find("<object3D id=\"cloud\"").unwrap();
    let volume_end = inner.find("</object3D>").unwrap() + "</object3D>".len();
    let volume = &inner[volume_start..volume_end];
    let outside = format!(
        "{}<symbols><symbol id=\"s\" width=\"8\" height=\"8\">{volume}</symbol></symbols><composition>{}",
        &inner[..inner.find("<composition>").unwrap()],
        &inner[volume_end..]
    );
    let message = match sr_model::load_str(&outside, &sr_model::LoadOptions::without_assets()) {
        Err(error) => format!("the document: {error}"),
        Ok(doc) => match Evaluator::new(&doc, &Default::default()) {
            Err(error) => format!("the evaluator: {error}"),
            Ok(_) => panic!("a gas that is not in the scope of its emitter must be refused"),
        },
    };
    println!("GAS scope: {message}");
    assert!(message.contains("cloud") || message.contains("gas"), "{message}");
}

#[test]
fn a_frame_between_two_canonical_steps_asks_for_a_partial_segment_and_simulates_nothing() {
    let ev = Setup::default().evaluator();
    let count = sr_eval::particles3d::gas_queries_on_this_thread;
    let steps = sr_eval::pyro::steps_on_this_thread;
    // blur samples go forward through the shutter: the first one simulates up to its step
    let _ = particle(&ev, 1.46);
    let smoke = steps();
    assert!(count() > 0);
    // a later sample in the same canonical step takes the one particle on by a partial segment of it: a handful of
    // queries (two for a short span, five when the span is split) from the step's own context, no smoke step
    // simulated and no field fetched; the same sample again is the same bits
    for t in [1.47, 1.48, 1.49] {
        let before = count();
        let first = particle(&ev, t);
        let asked = count() - before;
        assert!((1..=8).contains(&asked), "t = {t}: {asked} queries for one particle");
        assert_eq!(steps(), smoke, "t = {t}: no smoke step simulated");
        assert_eq!(bits(&first), bits(&particle(&ev, t)), "t = {t}");
    }
}

#[test]
fn a_volume_that_moves_drags_the_particles_where_it_is_when_they_ask() {
    // the emitter stands at x = 20, outside a volume 16 across at the origin; the volume is carried to x = 20 by 0.5 s
    let xml = |moving: bool| {
        let setup = Setup { x: 20.0, ..Setup::default() }.xml();
        if moving {
            setup.replace(
                r#"<object3D id="cloud" primitive="volume">"#,
                r#"<object3D id="cloud" primitive="volume"><animate property="x"><key time="0" value="0"/><key time="0.4" value="20"/></animate>"#,
            )
        } else {
            setup
        }
    };
    let (still, moving) = (evaluator(&xml(false)), evaluator(&xml(true)));
    // the gas is an impulse of the smoke, which the carried volume takes along as it was made in its own axes
    assert_eq!(particle(&still, 1.5).1, [0.0; 3], "the volume at the origin never meets the particle");
    let v = particle(&moving, 1.5).1;
    assert!(v[0] > 1.0, "the volume carried to the particle drags it: {v:?}");
}
