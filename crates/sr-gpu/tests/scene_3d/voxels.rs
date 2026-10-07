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
