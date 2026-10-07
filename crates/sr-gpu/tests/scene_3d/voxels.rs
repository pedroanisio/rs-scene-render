//! Voxel objects in a document: the surface of the cells of a `.vox` file drawn by the raster renderer and the path tracer.
//!
//! The files are written by the tests (a model of cells with a palette), so the number of quads of what is drawn is known by hand.

use super::common::*;
use std::path::{Path, PathBuf};

/// A `.vox` file (version 150) of one model: `cells` as `([x, y, z], palette index)` and the colours of the palette, entry `i` for the
/// index `i + 1`.
fn vox_file(name: &str, cells: &[([u8; 3], u8)], palette: &[[u8; 4]]) -> PathBuf {
    fn chunk(id: &[u8; 4], content: &[u8], children: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend((content.len() as u32).to_le_bytes());
        v.extend((children.len() as u32).to_le_bytes());
        v.extend(content);
        v.extend(children);
        v
    }
    let size = cells.iter().fold([1u32; 3], |s, (c, _)| std::array::from_fn(|a| s[a].max(u32::from(c[a]) + 1)));
    let mut children = chunk(b"SIZE", &size.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>(), &[]);
    let mut xyzi = (cells.len() as u32).to_le_bytes().to_vec();
    for (c, i) in cells {
        xyzi.extend([c[0], c[1], c[2], *i]);
    }
    children.extend(chunk(b"XYZI", &xyzi, &[]));
    let mut rgba = Vec::new();
    for i in 0..256 {
        rgba.extend(palette.get(i).copied().unwrap_or([255, 255, 255, 255]));
    }
    children.extend(chunk(b"RGBA", &rgba, &[]));
    let mut file = b"VOX ".to_vec();
    file.extend(150u32.to_le_bytes());
    file.extend(chunk(b"MAIN", &[], &children));
    let dir = std::env::temp_dir().join(format!("sr-voxels-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, file).unwrap();
    path
}

/// A box of `n` cells a side of the palette index `index`.
fn block(n: u8, index: u8) -> Vec<([u8; 3], u8)> {
    let mut cells = Vec::new();
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                cells.push(([x, y, z], index));
            }
        }
    }
    cells
}

const RED: [u8; 4] = [220, 30, 30, 255];
const BLUE: [u8; 4] = [30, 30, 220, 255];

/// A document with one voxels object (`object` its attributes) looked at from the front by a camera that fits a cube of 8 cells, under
/// a sun and an ambient light, with the materials `materials`.
fn document(file: &Path, object: &str, camera: &str, materials: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="2" background="#202020"/><assets><voxelAsset id="model" src="{}"/></assets>{materials}<composition><camera id="cam" x="4" y="-3" z="-24" target="obj" fov="30" {camera}/><object3D id="obj" primitive="voxels" voxels="model" x="-4" y="-4" z="-4" {object}/></composition><lights><light id="sun" type="directional" intensity="3" yaw="-30" pitch="-35"/><light id="fill" type="ambient" intensity="0.6"/></lights></scene>"##,
        file.display(),
        materials = if materials.is_empty() { String::new() } else { format!("<materials>{materials}</materials>") }
    )
}

struct Shot {
    pixels: Vec<[f32; 4]>,
    triangles: u64,
    draws: usize,
    groups: usize,
    errors: Vec<String>,
}

fn render(gpu: &sr_gpu::Gpu, xml: &str) -> Shot {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(0.5);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
    let out = renderer.render(&frame, ev.program());
    Shot {
        pixels: renderer.read(&out.texture),
        triangles: out.stats.triangles,
        draws: out.stats.objects3d,
        groups: out.stats.voxel_groups,
        errors: out.stats.errors.clone(),
    }
}

fn centre(shot: &Shot) -> [f32; 4] {
    let mut sum = [0.0; 4];
    for y in 28..36 {
        for x in 28..36 {
            for (s, v) in sum.iter_mut().zip(shot.pixels[y * 64 + x]) {
                *s += v / 64.0;
            }
        }
    }
    sum
}

#[test]
fn a_block_of_voxels_is_six_quads_in_the_colour_of_its_palette_whichever_renderer_draws_it() {
    let Some(gpu) = gpu() else { return };
    let file = vox_file("block.vox", &block(8, 1), &[RED]);
    for camera in ["", r#"renderer="pathtrace" pathSamples="8" maxBounces="2""#] {
        let shot = render(&gpu, &document(&file, "", camera, ""));
        assert!(shot.errors.is_empty(), "{:?}", shot.errors);
        assert_eq!((shot.triangles, shot.draws, shot.groups), (12, 1, 1), "six quads, one draw: {camera}");
        let c = centre(&shot);
        assert!(
            c[0] > 2.0 * c[1] && c[0] > 2.0 * c[2],
            "the block is red in the middle of the picture: {c:?} ({camera})"
        );
    }
}

#[test]
fn colours_do_not_split_the_draw_and_a_hole_through_the_block_is_sixteen_quads() {
    let Some(gpu) = gpu() else { return };
    // half red and half blue: the faces that cross the two halves are cut in two (10 quads), and the colour is in the vertices, so
    // there is still one draw
    let halves: Vec<_> = block(8, 1).into_iter().map(|(c, _)| (c, if c[0] < 4 { 1 } else { 2 })).collect();
    let shot = render(&gpu, &document(&vox_file("halves.vox", &halves, &[RED, BLUE]), "", "", ""));
    assert!(shot.errors.is_empty(), "{:?}", shot.errors);
    assert_eq!((shot.triangles, shot.draws, shot.groups), (20, 1, 1));
    // a square hole of 2 through the middle of a block of 8, along z: 16 quads
    let holed: Vec<_> =
        block(8, 1).into_iter().filter(|(c, _)| !((3..5).contains(&c[0]) && (3..5).contains(&c[1]))).collect();
    let shot = render(&gpu, &document(&vox_file("holed.vox", &holed, &[RED]), "", "", ""));
    assert!(shot.errors.is_empty(), "{:?}", shot.errors);
    assert_eq!(shot.triangles, 32);
}

#[test]
fn palette_materials_of_the_document_are_one_draw_each_and_the_object_material_fills_in_the_rest() {
    let Some(gpu) = gpu() else { return };
    let halves: Vec<_> = block(8, 1).into_iter().map(|(c, _)| (c, if c[0] < 4 { 1 } else { 2 })).collect();
    let file = vox_file("materials.vox", &halves, &[RED, BLUE]);
    let materials = r##"<material id="stone" baseColor="#808080"/><material id="moss" baseColor="#208020"/><material id="fallback" baseColor="#F0F0F0"/>"##;
    let both = render(&gpu, &document(&file, r#"palette="stone moss""#, "", materials));
    assert!(both.errors.is_empty(), "{:?}", both.errors);
    assert_eq!((both.draws, both.groups), (2, 2), "one draw for each document material");
    // an index the palette does not name takes the object's own material: the one list entry and the fallback are two draws too
    let rest = render(&gpu, &document(&file, r#"palette="stone" material="fallback""#, "", materials));
    assert_eq!((rest.draws, rest.groups), (2, 2));
    // the same material for both indices is one class and one draw: 6 quads, not 10
    let same = render(&gpu, &document(&file, r#"palette="stone stone""#, "", materials));
    assert_eq!((same.draws, same.triangles), (1, 12));
}

#[test]
fn a_surface_over_its_memory_budget_is_an_error_with_its_cause_and_draws_nothing() {
    let Some(gpu) = gpu() else { return };
    // a checkerboard of 12 (864 cells, 5184 quads, 6.4 MB at the peak) under a budget of 1 MiB
    let board: Vec<_> = block(12, 1).into_iter().filter(|(c, _)| (c[0] + c[1] + c[2]) % 2 == 0).collect();
    let file = vox_file("board.vox", &board, &[RED]);
    let small = render(&gpu, &document(&file, r#"surfaceMemoryMiB="1""#, "", ""));
    assert!(small.errors.iter().any(|e| e.contains("voxel surface exceeds memory budget")), "{:?}", small.errors);
    assert_eq!(small.triangles, 0);
    let roomy = render(&gpu, &document(&file, r#"surfaceMemoryMiB="16""#, "", ""));
    assert!(roomy.errors.is_empty(), "{:?}", roomy.errors);
    assert_eq!(roomy.triangles, 2 * 6 * 864);
}

#[test]
fn a_frame_that_changes_nothing_of_the_cells_gives_the_same_picture_to_the_bit() {
    let Some(gpu) = gpu() else { return };
    let file = vox_file("again.vox", &block(8, 1), &[RED]);
    let xml = document(&file, "", "", "");
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::default()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
    let mut frames = Vec::new();
    for t in [0.2, 0.2, 0.9] {
        let out = renderer.render(&ev.evaluate(t), ev.program());
        assert!(out.stats.errors.is_empty(), "{:?}", out.stats.errors);
        frames.push(renderer.read(&out.texture));
    }
    assert!(frames[0] == frames[1] && frames[1] == frames[2], "a still object is the same picture at every time");
}

// ---------------------------------------------------------------------------------------------------------- T-junctions

/// The cells of a ball of radius `r` cells (index 1): a closed body whose merged quads meet at vertices that lie on the edges of larger ones.
fn ball(r: f32) -> Vec<([u8; 3], u8)> {
    let n = (2.0 * r).ceil() as u8 + 1;
    block(n, 1)
        .into_iter()
        .filter(|(c, _)| {
            let d = |a: usize| f32::from(c[a]) + 0.5 - r;
            (d(0) * d(0) + d(1) * d(1) + d(2) * d(2)).sqrt() <= r
        })
        .collect()
}

/// Pixels that are background but enclosed by the body: the background that cannot be reached from the corners of the picture without
/// crossing a pixel of the body (lighter than a half). A crack between two quads of a closed body shows as such a pixel.
fn holes(shot: &Shot, size: usize) -> usize {
    let solid = |i: usize| shot.pixels[i][0] > 0.5;
    let mut reached = vec![false; size * size];
    let mut stack = Vec::new();
    for i in 0..size {
        for p in [i, (size - 1) * size + i, i * size, i * size + size - 1] {
            if !solid(p) && !reached[p] {
                reached[p] = true;
                stack.push(p);
            }
        }
    }
    while let Some(p) = stack.pop() {
        let (x, y) = (p % size, p / size);
        for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
            if nx < size && ny < size {
                let q = ny * size + nx;
                if !solid(q) && !reached[q] {
                    reached[q] = true;
                    stack.push(q);
                }
            }
        }
    }
    (0..size * size).filter(|i| !solid(*i) && !reached[*i]).count()
}

#[test]
fn a_closed_voxel_body_has_no_cracks_between_its_merged_quads_at_any_pose_in_either_renderer() {
    let Some(gpu) = gpu() else { return };
    // an unlit white ball of radius 10 on black: the quads that the merge makes meet at vertices that lie on the edges of larger quads (a
    // T-junction), and a crack between them would show the black through the ball. Poses of the object in both axes, both renderers
    let file = vox_file("ball.vox", &ball(6.0), &[[255, 255, 255, 255]]);
    let materials = r##"<material id="white" baseColor="#FFFFFF" unlit="true"/>"##;
    let mut worst = 0;
    for (renderer, camera) in
        [("raster", ""), ("path tracer", r#"renderer="pathtrace" pathSamples="4" maxBounces="1""#)]
    {
        // the camera goes round the ball (its centre is at the origin) at every pose; the ball does not move
        for (ry, rx) in [
            (0.0f32, 0.0f32),
            (17.0, 5.0),
            (41.0, 23.0),
            (63.0, 77.0),
            (97.0, 12.0),
            (133.0, 61.0),
            (171.0, 33.0),
            (229.0, 49.0),
            (283.0, 7.0),
            (337.0, 85.0),
        ] {
            let (a, b) = (ry.to_radians(), rx.to_radians());
            let (x, y, z) = (40.0 * a.sin() * b.cos(), -40.0 * b.sin(), -40.0 * a.cos() * b.cos());
            let xml = document(&file, r#"palette="white""#, camera, materials)
                .replace(r##"background="#202020""##, r##"background="#000000""##)
                .replace(r#"x="4" y="-3" z="-24" target="obj""#, &format!(r#"x="{x}" y="{y}" z="{z}" target="mid""#))
                .replace(r#"x="-4" y="-4" z="-4""#, r#"x="-6" y="-6" z="-6""#)
                .replace("</composition>", r#"<object3D id="mid" primitive="box" width="0.1" height="0.1" depth="0.1" visible="false"/></composition>"#);
            let shot = render(&gpu, &xml);
            assert!(shot.errors.is_empty(), "{:?}", shot.errors);
            let body = shot.pixels.iter().filter(|p| p[0] > 0.5).count();
            assert!((600..1800).contains(&body), "the ball is seen, with room round it ({body} pixels of it)");
            let count = holes(&shot, 64);
            println!("{renderer} rotationY {ry} rotationX {rx}: {body} pixels of the body, {count} pixels of the background inside the silhouette");
            worst = worst.max(count);
        }
    }
    assert_eq!(worst, 0, "a crack shows the background through the closed body");
}

// -------------------------------------------------------------------------------------------------------------- cellSize

/// The picture of a block of 4 cells lit from above by a sun that casts shadows, with cells of `size` scene units and the camera
/// moved in proportion: the same picture if the shading did not depend on the scale.
fn scaled_block(gpu: &sr_gpu::Gpu, file: &Path, size: f32, camera: &str, shadows: bool) -> Shot {
    let d = 40.0 * size;
    let xml = format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="2" background="#202020"/><assets><voxelAsset id="model" src="{}"/></assets><composition><camera id="cam" x="0" y="{}" z="{}" target="mid" fov="30" {camera}/><object3D id="obj" primitive="voxels" voxels="model" cellSize="{size}" x="{}" y="{}" z="{}"/><object3D id="ground" primitive="plane" width="{ground}" height="{ground}" y="{floor}" rotationX="-90" castShadow="false"/><object3D id="mid" primitive="box" width="0.1" height="0.1" depth="0.1" visible="false"/></composition><lights><light id="sun" type="directional" intensity="3" castShadow="{shadows}" yaw="90" pitch="-50"/><light id="fill" type="ambient" intensity="0.3"/></lights></scene>"##,
        file.display(),
        -0.5 * d,
        -0.87 * d,
        -2.0 * size,
        -2.0 * size,
        -2.0 * size,
        ground = 40.0 * size,
        floor = 2.0 * size,
    );
    render(gpu, &xml)
}

#[test]
fn the_shadow_of_a_block_is_the_same_whatever_the_size_of_its_cells_down_to_the_sizes_the_srep_states() {
    let Some(gpu) = gpu() else { return };
    let file = vox_file("scaled.vox", &block(4, 1), &[RED]);
    for (renderer, camera) in
        [("raster", ""), ("path tracer", r#"renderer="pathtrace" pathSamples="16" maxBounces="2""#)]
    {
        // a block of 4 cells on a floor under a sun that casts shadows, the camera and the floor moved in proportion: the same picture
        // whatever the size of a cell, if the shadows (their offsets and biases) scale with the scene
        let reference = scaled_block(&gpu, &file, 1.0, camera, true);
        let unshadowed = scaled_block(&gpu, &file, 1.0, camera, false);
        let differ =
            |a: &Shot, b: &Shot| a.pixels.iter().zip(&b.pixels).filter(|(x, y)| (x[0] - y[0]).abs() > 0.02).count();
        let shadow = differ(&reference, &unshadowed);
        assert!(shadow > 40, "{renderer}: the picture has a shadow in it ({shadow} pixels differ without it)");
        let mut by_size = Vec::new();
        for size in [4.0f32, 2.0, 0.5, 0.25, 0.1, 0.05, 0.02] {
            let shot = scaled_block(&gpu, &file, size, camera, true);
            assert!(shot.errors.is_empty(), "{:?}", shot.errors);
            by_size.push((size, differ(&shot, &reference)));
        }
        println!("{renderer}: pixels that differ from the picture of cells of 1, by cell size, of {shadow} that are the shadow's: {by_size:?}");
        // above a unit nothing changes; below it the raster renderer's shadow loses its offsets (the SREP says from which size, and why
        // the path tracer's holds longer); these are the measured limits: a fix that scales them with the scene fails the last two
        for (size, diff) in &by_size {
            match (renderer, *size) {
                // the shadow map's texels follow the size of the scene, so the raster edge moves by a few pixels (6 measured at 4)
                ("raster", s) if s >= 2.0 => assert!(*diff <= 8, "{renderer}: {diff} pixels differ at {s}"),
                ("raster", s) if s >= 0.5 => assert!(*diff <= 12, "{renderer}: {diff} pixels differ at {s}"),
                (_, s) if s >= 2.0 => assert!(*diff <= 2, "{renderer}: {diff} pixels differ at {s}"),
                ("raster", s) if s <= 0.1 => {
                    assert!(*diff >= shadow * 9 / 10, "{renderer}: the shadow is gone at {s}: {diff} of {shadow}")
                }
                ("path tracer", s) if s >= 0.25 => assert!(*diff <= 4, "{renderer}: {diff} pixels differ at {s}"),
                _ => {}
            }
        }
    }
}
