//! Foam as a coverage of the ocean surface (`whitewater@foamMode="albedo"`): the tracers are the same, the surface carries their
//! coverage in the alpha of its vertex colour, and no foam triangles are made.

use sr_sim::ocean::whitewater::Kind;

/// A basin of 8 x 8 with a wave and an impulse, whose whitewater (many tracers, a low threshold) is `whitewater`.
fn ocean(whitewater: &str) -> sr_eval::Evaluator {
    let xml = format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="4"/><composition>
          <ocean id="sea" width="8" depth="8" cellSize="0.5" bottomDepth="2" dt="0.05">
            <waterImpulse time="0.1" radius="2" amplitude="0.3"/><wave wavelength="4" amplitude="0.2" phase="0"/>
            <whitewater emissionRate="6" threshold="0.2" lifetime="3" seed="5" {whitewater}/>
          </ocean></composition></scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
}

fn sea(ev: &sr_eval::Evaluator, t: f64) -> std::sync::Arc<sr_eval::ocean::SimOcean> {
    let f = ev.evaluate(t);
    assert!(f.problems.is_empty(), "{:?}", f.problems);
    f.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap().clone()
}

#[test]
fn in_particles_mode_foam_is_drawn_as_triangles_and_the_surface_has_no_coverage() {
    let particles = sea(&ocean(""), 1.5);
    let explicit = sea(&ocean(r#"foamMode="particles""#), 1.5);
    let foam = particles.whitewater.as_ref().unwrap().particles.iter().filter(|p| p.kind == Kind::Foam).count();
    assert!(foam > 20, "the case makes foam: {foam}");
    assert!(!particles.whitewater_mesh[0].indices.is_empty());
    assert!(particles.mesh.vertices.iter().all(|v| v.color == [1.0; 4]), "the surface is as it always was");
    // the default is the explicit mode: the same bits
    assert_eq!(particles.mesh.vertices, explicit.mesh.vertices);
    assert_eq!(particles.whitewater_mesh[0].vertices, explicit.whitewater_mesh[0].vertices);
    assert_eq!(particles.key, explicit.key);
}

#[test]
fn in_albedo_mode_the_tracers_become_the_coverage_of_the_surface_and_no_foam_is_drawn() {
    let (radius, t) = (1.0, 1.5);
    let particles = sea(&ocean(""), t);
    let albedo = sea(&ocean(&format!(r#"foamMode="albedo" foamRadius="{radius}""#)), t);
    // the simulation does not know the mode: the same tracers, the same spray, the same surface but for its colour
    assert_eq!(albedo.whitewater, particles.whitewater);
    assert!(albedo.whitewater_mesh[0].indices.is_empty(), "no foam triangles");
    assert_eq!(albedo.whitewater_mesh[1].vertices, particles.whitewater_mesh[1].vertices, "spray is drawn as before");
    assert_eq!(albedo.mesh.indices, particles.mesh.indices);
    for (a, p) in albedo.mesh.vertices.iter().zip(&particles.mesh.vertices) {
        assert_eq!((a.pos, a.normal, a.uv, a.tangent), (p.pos, p.normal, p.uv, p.tangent));
        assert_eq!(a.color[..3], [1.0; 3], "only the alpha carries the coverage");
        assert!((0.0..=1.0).contains(&a.color[3]), "{}", a.color[3]);
    }
    // a vertex is covered exactly where a living foam tracer is within the radius
    let frame = albedo.whitewater.as_ref().unwrap();
    let near = |x: f32, z: f32| {
        frame.particles.iter().any(|p| {
            let age = (frame.time - p.birth) / p.lifetime;
            p.kind == Kind::Foam
                && (0.0..1.0).contains(&age)
                && (f64::from(x) - p.position[0]).hypot(f64::from(z) - p.position[2]) < radius
        })
    };
    let (mut covered, mut bare) = (0, 0);
    for v in &albedo.mesh.vertices {
        let c = v.color[3];
        assert_eq!(c > 0.0, near(v.pos[0], v.pos[2]), "vertex at ({}, {}) has coverage {c}", v.pos[0], v.pos[2]);
        covered += usize::from(c > 0.0);
        bare += usize::from(c == 0.0);
    }
    assert!(covered > 10 && bare > 10, "the case covers part of the surface: {covered} covered, {bare} bare");
    // the coverage is a function of the frame: evaluating it again, or after another time, gives the same
    sea(&ocean(&format!(r#"foamMode="albedo" foamRadius="{radius}""#)), 0.5);
    let again = sea(&ocean(&format!(r#"foamMode="albedo" foamRadius="{radius}""#)), t);
    assert_eq!(again.mesh.vertices, albedo.mesh.vertices);
}

#[test]
fn the_default_coverage_radius_is_one_cell() {
    let one_cell = sea(&ocean(r#"foamMode="albedo""#), 1.5);
    let explicit = sea(&ocean(r#"foamMode="albedo" foamRadius="0.5""#), 1.5);
    assert_eq!(one_cell.mesh.vertices, explicit.mesh.vertices);
    let wider = sea(&ocean(r#"foamMode="albedo" foamRadius="1.5""#), 1.5);
    let sum = |s: &sr_eval::ocean::SimOcean| s.mesh.vertices.iter().map(|v| f64::from(v.color[3])).sum::<f64>();
    assert!(
        sum(&wider) > 2.0 * sum(&one_cell),
        "a wider radius covers more: {} against {}",
        sum(&wider),
        sum(&one_cell)
    );
}

/// A basin of `size` by `size` cells of side 1 whose whitewater is `whitewater`, with the ocean's own attributes `ocean_attrs`.
fn basin(size: u32, ocean_attrs: &str, whitewater: &str) -> sr_eval::Evaluator {
    let xml = format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="4"/><composition>
          <ocean id="sea" width="{size}" depth="{size}" cellSize="1" bottomDepth="2" dt="0.1" {ocean_attrs}>
            <waterImpulse time="0.1" radius="3" amplitude="0.3"/>
            <whitewater emissionRate="60" threshold="0.05" lifetime="3" seed="5" {whitewater}/>
          </ocean></composition></scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
}

#[test]
fn the_memory_of_the_coverage_counts_in_the_surface_budget() {
    // 301 x 301 vertices: the surface alone takes 27.5 MB, the coverage's bins and shares another 7.2 MB, under a budget of 30 MiB
    let made = |attrs: &str, whitewater: &str| {
        let f = basin(600, attrs, whitewater).evaluate(1.0);
        (f.failures.clone(), f.nodes.iter().find(|n| &*n.id == "sea").and_then(|n| n.sim_ocean.clone()).is_some())
    };
    let (failures, made_particles) = made(r#"surfaceMemoryMiB="108""#, "");
    assert!(made_particles, "the particles mode fits: {failures:?}");
    let (failures, made_albedo) = made(r#"surfaceMemoryMiB="108""#, r#"foamMode="albedo""#);
    assert!(!made_albedo, "the coverage does not fit");
    assert!(
        failures.iter().any(|m| m.contains("foam coverage") && m.contains("memory")),
        "the failure says what and which budget: {failures:?}"
    );
    let (failures, made_albedo) = made(r#"surfaceMemoryMiB="128""#, r#"foamMode="albedo""#);
    assert!(made_albedo, "with room for it: {failures:?}");
}
