//! A mesh that emits particles is a surface, closed or not: a flat one emits from where it is, and one that has no
//! surface to emit from is an error, never a quiet emitter that makes nothing.

use sr_eval::Evaluator;
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sr-flat-emitter-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The frame at `t` of a scene whose emitter takes its surface from `obj`, or why the evaluator refused.
fn frame_of(name: &str, obj: &str, t: f64) -> (usize, Vec<String>) {
    frame_with(name, obj, t, "", "")
}

/// The same, with more attributes on the emitter and more in the physics.
fn frame_with(name: &str, obj: &str, t: f64, emitter: &str, physics: &str) -> (usize, Vec<String>) {
    let dir = scratch(name);
    std::fs::write(dir.join("surface.obj"), obj).unwrap();
    let path = dir.join("scene.xml");
    std::fs::write(
        &path,
        format!(
            r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/>
          <assets><mesh id="surface" src="surface.obj"/></assets><materials><material id="gas" baseColor="#FFFFFF" unlit="true"/></materials>
          <composition><particles3D id="sparks" shape="sphere" segments="4" material="gas" emitterShape="mesh" emitterMesh="surface" rate="200" lifetime="2" maxParticles="1000" {emitter}/></composition>
          <physics pixelsPerMeter="1">{physics}</physics></scene>"##
        ),
    )
    .unwrap();
    let doc = sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e}"));
    let ev = Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"));
    let frame = ev.evaluate(t);
    let count = frame
        .nodes
        .iter()
        .find(|n| &*n.id == "sparks")
        .and_then(|n| n.particles3d.as_ref())
        .map_or(0, |p| p.frame.particles.len());
    (count, frame.problems.iter().chain(&frame.failures).cloned().collect())
}

const SQUARE: &str = "v 0 0 0\nv 4 0 0\nv 4 0 4\nv 0 0 4\nf 1 2 3\nf 1 3 4\n";

#[test]
fn a_flat_mesh_emits_from_its_surface() {
    let (count, said) = frame_of("flat", SQUARE, 1.0);
    assert!(said.is_empty(), "{said:?}");
    assert!(count > 100, "{count} particles from a square of 16 square units");
}

#[test]
fn a_mesh_with_no_surface_to_emit_from_is_an_error() {
    // all the points on a line: every triangle is degenerate
    let line = "v 0 0 0\nv 1 0 0\nv 2 0 0\nf 1 2 3\n";
    let (count, said) = frame_of("line", line, 1.0);
    assert!(said.iter().any(|m| m.contains("sparks")), "an error that names the emitter: {said:?} ({count} particles)");
}

/// An annulus of 96 segments in the xy plane, from radius 6 to 11, flat: two triangles a segment.
fn annulus() -> String {
    let (n, inner, outer) = (96usize, 6.0f64, 11.0f64);
    let mut obj = String::new();
    for i in 0..n {
        let a = std::f64::consts::TAU * i as f64 / n as f64;
        obj += &format!(
            "v {:.5} {:.5} 0\nv {:.5} {:.5} 0\n",
            outer * a.cos(),
            outer * a.sin(),
            inner * a.cos(),
            inner * a.sin()
        );
    }
    for i in 0..n {
        let j = (i + 1) % n;
        let (oi, ii, oj, ij) = (2 * i + 1, 2 * i + 2, 2 * j + 1, 2 * j + 2);
        obj += &format!("f {oi} {oj} {ij}\nf {oi} {ij} {ii}\n");
    }
    obj
}

/// The same ring closed, 0.3 thick: top, bottom and the two walls, as the scene's own script writes it.
fn closed_ring() -> String {
    let (n, inner, outer, h) = (96usize, 6.0f64, 11.0f64, 0.15f64);
    let mut obj = String::new();
    for i in 0..n {
        let a = std::f64::consts::TAU * i as f64 / n as f64;
        for (r, z) in [(outer, h), (outer, -h), (inner, h), (inner, -h)] {
            obj += &format!("v {:.5} {:.5} {z}\n", r * a.cos(), r * a.sin());
        }
    }
    for i in 0..n {
        let j = (i + 1) % n;
        let [oit, oib, iit, iib] = [0, 1, 2, 3].map(|k| 4 * i + k + 1);
        let [ojt, ojb, ijt, ijb] = [0, 1, 2, 3].map(|k| 4 * j + k + 1);
        for t in [
            (oit, ojt, ijt),
            (oit, ijt, iit),
            (oib, ijb, ojb),
            (oib, iib, ijb),
            (oit, oib, ojb),
            (oit, ojb, ojt),
            (iit, ijb, iib),
            (iit, ijt, ijb),
        ] {
            obj += &format!("f {} {} {}\n", t.0, t.1, t.2);
        }
    }
    obj
}

#[test]
fn a_flat_annulus_and_a_closed_ring_emit() {
    for (name, obj) in [("annulus", annulus()), ("ring", closed_ring())] {
        let (count, said) = frame_of(name, &obj, 1.0);
        assert!(said.is_empty(), "{name}: {said:?}");
        assert!(count > 100, "{name}: {count} particles");
    }
}

#[test]
fn the_ring_of_the_black_hole_scene_with_its_forces_emits() {
    let forces =
        r#"<forceField id="pull" type="radial" strength="8"/><forceField id="spin" type="vortex" strength="12"/>"#;
    for (name, obj) in [("annulus-forces", annulus()), ("ring-forces", closed_ring())] {
        let (count, said) =
            frame_with(name, &obj, 3.0, r#"drag="4" forceFields="pull spin" scaleZ="0.03" size="0.15""#, forces);
        assert!(said.is_empty(), "{name}: {said:?}");
        assert!(count > 100, "{name}: {count} particles");
    }
}
