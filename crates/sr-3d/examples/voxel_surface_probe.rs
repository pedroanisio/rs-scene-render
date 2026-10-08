//! The cost of the surface of a grid of voxels: cells, faces, quads and CPU seconds of a full mesh, of an incremental update after a cut
//! and of a frame with nothing changed, for a solid cube, a sphere, a hollow shell and a checkerboard of about a million cells.
//!
//! usage: voxel_surface_probe

use sr_3d::occupancy::Occupancy;
use sr_3d::voxel::surface::{expand, exposed_faces, mesh_quads, Classes, SurfaceCache};
use std::time::Instant;

fn grid(cells: impl Iterator<Item = [i32; 3]>) -> Occupancy {
    Occupancy::from_cells(cells.map(|c| (c, 1))).expect("a grid")
}

fn main() {
    let classes = Classes::identity();
    let cube = |n: i32| (0..n).flat_map(move |z| (0..n).flat_map(move |y| (0..n).map(move |x| [x, y, z])));
    let ball = |r: f64, inner: f64| {
        let n = (2.0 * r).ceil() as i32 + 1;
        cube(n).filter(move |c| {
            let d = ((f64::from(c[0]) + 0.5 - r).powi(2)
                + (f64::from(c[1]) + 0.5 - r).powi(2)
                + (f64::from(c[2]) + 0.5 - r).powi(2))
            .sqrt();
            d <= r && d > inner
        })
    };
    let cases: Vec<(&str, Occupancy)> = vec![
        ("cube 100", grid(cube(100))),
        ("sphere r 62", grid(ball(62.0, -1.0))),
        ("shell r 100 of 2", grid(ball(100.0, 98.0))),
        ("checkerboard 126", grid(cube(126).filter(|c| (c[0] + c[1] + c[2]) % 2 == 0))),
    ];
    println!(
        "{:<18} {:>9} {:>9} {:>9} {:>10} {:>10} {:>10} {:>10}",
        "case", "cells", "faces", "quads", "faces s", "mesh s", "expand s", "vertex MiB"
    );
    for (name, g) in &cases {
        let started = Instant::now();
        let faces = exposed_faces(g, &classes).len();
        let faces_s = started.elapsed().as_secs_f64();
        let started = Instant::now();
        let quads = mesh_quads(g, &classes);
        let mesh_s = started.elapsed().as_secs_f64();
        let started = Instant::now();
        let p = expand(&quads, |_| [1.0; 4]);
        let expand_s = started.elapsed().as_secs_f64();
        println!(
            "{name:<18} {:>9} {faces:>9} {:>9} {faces_s:>10.3} {mesh_s:>10.3} {expand_s:>10.3} {:>10.1}",
            g.count(),
            quads.len(),
            (p.vertices.len() * std::mem::size_of::<sr_3d::Vertex>()) as f64 / (1 << 20) as f64
        );
    }
    // a cut of a sphere of radius 12 out of the side of the shell, and a frame with nothing changed
    let mut g = cases[2].1.clone();
    let mut cache = SurfaceCache::new();
    let started = Instant::now();
    cache.update(&g, &classes, usize::MAX).expect("fits");
    let first = started.elapsed().as_secs_f64();
    let before = g.revision();
    let mut removed = 0;
    for c in ball(12.0, -1.0).map(|c| [c[0] + 187, c[1] + 88, c[2] + 88]) {
        if g.set(c, 0).unwrap() {
            removed += 1;
        }
    }
    let bricks = g.changed_bricks_since(before).len();
    let started = Instant::now();
    let cut = cache.update(&g, &classes, usize::MAX).expect("fits");
    let cut_s = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let still = cache.update(&g, &classes, usize::MAX).expect("fits");
    let still_s = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let fresh = mesh_quads(&g, &classes);
    let fresh_s = started.elapsed().as_secs_f64();
    assert_eq!(cache.quads(), fresh, "the cache is the mesh of the cut grid");
    println!(
        "shell: first update {first:.3} s; a cut of {removed} cells in {bricks} bricks: {} planes remeshed, {cut_s:.3} s (a full mesh {fresh_s:.3} s); nothing changed: {} planes, {still_s:.6} s",
        cut.remeshed, still.remeshed
    );
}
