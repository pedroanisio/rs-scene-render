//! Ejecta that an impact causes: a `burst` that names a crater is born at the instants of the
//! impact's launches, each particle with its own place, velocity and mass.

use sr_eval::Evaluator;

struct Setup {
    /// Speed toward the ground, metres per second.
    down: f64,
    material: &'static str,
    /// Attributes of the particles3D.
    emitter: &'static str,
    /// Children of the particles3D besides the burst from the crater.
    extra: &'static str,
    /// Attributes of the burst from the crater.
    burst: &'static str,
}

impl Default for Setup {
    fn default() -> Self {
        Setup { down: 100.0, material: "softRock", emitter: "", extra: "", burst: r#"count="300""# }
    }
}

impl Setup {
    fn xml(&self) -> String {
        format!(
            r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
              <object3D id="rock" primitive="sphere" radius="0.5" y="40">
                <rigidBody shape="sphere" mass="1500" velocityY="{down}" restitution="0" linearDamping="0" angularDamping="0"/>
              </object3D>
              <object3D id="ground" primitive="plane" width="40" height="40" segments="64" y="60" rotationX="-90">
                <crater id="pit" source="rock" targetMaterial="{material}"/>
                <rigidBody type="static" shape="auto"/>
              </object3D>
              <particles3D id="debris" rate="0" lifetime="6" dt="0.01" gravityY="9.80665" maxParticles="2000" {emitter}>
                <burst crater="pit" {burst}/>{extra}
              </particles3D>
            </composition>
            <physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"##,
            down = self.down,
            material = self.material,
            emitter = self.emitter,
            extra = self.extra,
            burst = self.burst,
        )
    }

    fn evaluator(&self) -> Evaluator {
        let doc =
            sr_model::load_str(&self.xml(), &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
        Evaluator::new(&doc, &Default::default()).unwrap()
    }
}

fn debris(ev: &Evaluator, t: f64) -> std::sync::Arc<sr_eval::particles3d::SimParticles3D> {
    let frame = ev.evaluate(t);
    assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
    frame.nodes.iter().find(|n| &*n.id == "debris").unwrap().particles3d.clone().expect("particles")
}

/// The crater the law gives for the 1500 kg, 0.5 m rock that has fallen 19.5 m from `down`.
fn law(down: f64) -> sr_sim::cratering::Crater {
    use sr_sim::cratering::{crater, Impact, Material, Target};
    let volume = 4.0 / 3.0 * std::f64::consts::PI * 0.125;
    let speed = (down * down + 2.0 * 9.80665 * 19.5).sqrt();
    crater(
        &Impact { mass: 1500.0, density: 1500.0 / volume, normal_speed: speed },
        &Target { material: Material::SoftRock, density: None, strength: None, gravity: 9.80665 },
    )
    .unwrap()
}

#[test]
fn nothing_is_thrown_before_the_impact_and_then_every_particle_has_its_own_birth() {
    let ev = Setup::default().evaluator();
    // the rock lands near 0.19 s
    assert_eq!(debris(&ev, 0.1).frame.emitted, 0, "no ejecta before the impact");
    let early = debris(&ev, 0.2).frame.emitted;
    let after = debris(&ev, 1.5);
    assert_eq!(after.frame.emitted, 300, "every particle of the burst is born once the crater has formed");
    assert!(early < 300, "the launches are spread over the formation of the crater: {early} at 0.2 s");
    let particles = &after.frame.particles;
    assert_eq!(particles.len(), 300);
    assert!(particles.iter().all(|p| p.mass > 0.0 && p.birth > 0.15), "each has a mass and is born after the impact");
    // ids follow the order of birth, and the first ones are launched fastest and from the nearest
    let mut ordered = particles.clone();
    ordered.sort_by_key(|p| p.id);
    assert!(ordered.windows(2).all(|w| w[0].birth <= w[1].birth));
    let c = law(100.0);
    let last = ordered.last().unwrap().birth;
    let first = ordered[0].birth;
    assert!(last - first <= c.duration * 1.1, "the launches span the formation time {}: {first}..{last}", c.duration);
}

#[test]
fn the_particles_hold_four_fifths_of_the_crater_mass_and_leave_its_surface_upward() {
    let ev = Setup::default().evaluator();
    let frame = debris(&ev, 0.6);
    let total: f64 = frame.frame.particles.iter().map(|p| p.mass).sum();
    // softRock: 2100 kg/m3, and the share thrown out is 0.8 of the crater's volume
    let want = 2100.0 * law(100.0).ejecta_volume;
    assert!((total - want).abs() < 0.2 * want, "ejecta mass {total} against {want}");
    // the ground is at y = 60 under the rock: ejecta leave it toward smaller y
    // (gravity has acted on each since its birth, and nothing else has)
    for p in &frame.frame.particles {
        let launched = p.velocity[1] - 9.80665 * (0.6 - p.birth);
        assert!(launched < 0.0, "launched away from the ground: {:?} at {}", p.velocity, p.birth);
    }
    // and each was launched from the impact point's neighbourhood: between 1.2 body radii and the
    // crater's radius from where the rock hit (the origin of the ground)
    let c = law(100.0);
    for p in &frame.frame.particles {
        let dt = 0.6 - p.birth;
        let (x, z) = (p.position[0] - p.velocity[0] * dt, p.position[2] - p.velocity[2] * dt);
        let from = (x * x + z * z).sqrt();
        assert!((1.2 * 0.5 * 0.99..=c.radius * 1.01).contains(&from), "launched {from} m from the impact point");
    }
}

#[test]
fn scrubbing_back_and_forth_gives_the_same_particles() {
    let ev = Setup::default().evaluator();
    let a = debris(&ev, 1.2);
    let _ = debris(&ev, 0.1);
    let _ = debris(&ev, 0.3);
    let b = debris(&ev, 1.2);
    assert_eq!(a.key, b.key);
    assert_eq!(a.frame, b.frame);
    let fresh = Setup::default().evaluator();
    assert_eq!(debris(&fresh, 1.2).key, a.key, "an evaluator that has not scrubbed agrees");
}

#[test]
fn a_burst_in_time_next_to_the_crater_one_is_unchanged() {
    let plain = Setup { burst: r#"count="1""#, extra: r#"<burst time="0.5" count="5"/>"#, ..Setup::default() };
    let frame = debris(&plain.evaluator(), 1.5);
    assert_eq!(frame.frame.emitted, 6);
    assert_eq!(frame.frame.particles.iter().filter(|p| p.mass == 0.0).count(), 5, "ordinary bursts carry no mass");
    assert_eq!(frame.frame.particles.iter().filter(|p| p.mass > 0.0).count(), 1);
}

#[test]
fn more_ejecta_than_maxparticles_is_an_error_not_a_truncation() {
    let ev = Setup { burst: r#"count="3000""#, ..Setup::default() }.evaluator();
    let frame = ev.evaluate(1.5);
    assert!(frame.failures.iter().any(|f| f.contains("maxParticles")), "{:?}", frame.failures);
}

#[test]
fn a_material_without_an_ejecta_row_is_an_error_that_says_so() {
    let ev = Setup { material: "regolith", ..Setup::default() }.evaluator();
    let frame = ev.evaluate(1.5);
    assert!(frame.failures.iter().any(|f| f.contains("regolith")), "{:?}", frame.failures);
}

#[test]
fn a_faster_body_throws_its_ejecta_faster() {
    let speed = |down: f64| {
        let frame = debris(&Setup { down, ..Setup::default() }.evaluator(), 1.0);
        frame.frame.particles.iter().map(|p| p.velocity.iter().map(|v| v * v).sum::<f64>().sqrt()).fold(0.0, f64::max)
    };
    let (slow, fast) = (speed(60.0), speed(150.0));
    assert!(slow < fast, "fastest ejecta: {slow} against {fast}");
}
