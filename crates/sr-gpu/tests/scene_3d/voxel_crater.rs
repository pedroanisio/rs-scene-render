//! A crater in a ground of voxels, end to end: the document's ball hits the ground, the world cuts it, and both renderers draw the ground and the part that came
//! away from what the frames say, with the surface that a remesh of the new cells makes.

use super::common::*;
use sr_3d::voxel::{
    srvol,
    surface::{mesh_quads, quads_hash, Classes},
};
use sr_3d::Occupancy;
use std::path::Path;

/// A slab of 240 by 40 by 240 cells of a quarter of a unit, its top at the key 60 (the scene's y points down), and a pillar of 2 by 2 cells that stands 15 over
/// it, 4 m from the middle of the slab, whose foot the crater reaches (the top comes away as a piece); the palette indices alternate so that the rim has colours to take.
fn ground() -> Occupancy {
    let mut cells = Vec::new();
    for k in 0..240 {
        for j in 60..100 {
            for i in 0..240 {
                cells.push(([i, j, k], 1 + ((i + j + k) % 3) as u8));
            }
        }
    }
    for k in 120..122 {
        for j in 0..60 {
            for i in 136..138 {
                cells.push(([i, j, k], 1 + ((i + j + k) % 3) as u8));
            }
        }
    }
    Occupancy::from_cells(cells).unwrap()
}

fn document(file: &Path, camera: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="96" height="96" fps="24" duration="2" background="#101010"/>
    <assets><voxelAsset id="model" src="{}"/></assets>
    <materials><material id="stone" baseColor="#b0b0b0" roughness="0.9"/></materials>
    <composition>
      <camera id="cam" x="30" y="-4" z="3" target="mid" fov="45" {camera}/>
      <object3D id="mid" primitive="box" width="0.1" height="0.1" depth="0.1" x="30" y="15" z="30" visible="false"/>
      <object3D id="ball" primitive="sphere" radius="2" x="30" y="2" z="30" visible="false">
        <rigidBody shape="sphere" mass="90478" velocityY="86.6" restitution="0" friction="0.5" linearDamping="0" angularDamping="0"/>
      </object3D>
      <object3D id="ground" primitive="voxels" voxels="model" material="stone">
        <rigidBody type="static" density="2700"/>
        <crater id="pit" source="ball" targetMaterial="softRock" gravity="9.80665"/>
      </object3D>
    </composition>
    <lights><light id="sun" type="directional" intensity="3" yaw="-30" pitch="-45"/><light id="fill" type="ambient" intensity="0.5"/></lights>
    <physics gravityY="0" pixelsPerMeter="1" fixedStep="0.004166666666666667" bounds="none"/></scene>"##,
        file.display()
    )
}

struct Frames {
    pixels: Vec<[f32; 4]>,
    surfaces: Vec<sr_gpu::VoxelSurfaceStat>,
    errors: Vec<String>,
}

fn draw(renderer: &mut sr_gpu::Renderer, ev: &sr_eval::Evaluator, t: f64) -> (Frames, sr_eval::FrameGraph) {
    let frame = ev.evaluate(t);
    assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
    let out = renderer.render(&frame, ev.program());
    (
        Frames {
            pixels: renderer.read(&out.texture),
            surfaces: out.stats.voxel_surfaces.clone(),
            errors: out.stats.errors.clone(),
        },
        frame,
    )
}

fn scene(name: &str, camera: &str) -> (std::path::PathBuf, sr_eval::Evaluator) {
    let dir = std::env::temp_dir().join(format!("sr-voxel-crater-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("ground.srvol");
    std::fs::write(&file, srvol::write(&ground(), 0.25).unwrap()).unwrap();
    let doc = sr_model::load_str(&document(&file, camera), &sr_model::LoadOptions::default())
        .unwrap_or_else(|e| panic!("{e:?}"));
    (dir, sr_eval::Evaluator::new(&doc, &Default::default()).unwrap())
}

/// What a surface of these cells is when it is made from nothing: the classes of the document material (every index is the stone's).
fn full_remesh_hash(grid: &Occupancy) -> (usize, u64) {
    let classes = Classes::new(|_| 1, &[]);
    let quads = mesh_quads(grid, &classes);
    (quads.len(), quads_hash(&quads))
}

#[test]
fn the_surface_of_the_ground_after_the_cut_is_the_surface_of_a_full_remesh_of_its_new_cells_in_both_renderers() {
    let Some(gpu) = gpu() else { return };
    for camera in ["", r#"renderer="pathtrace" pathSamples="4" maxBounces="2""#] {
        let (_dir, ev) = scene("surface", camera);
        let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let (before, frame) = draw(&mut renderer, &ev, 0.05);
        assert!(before.errors.is_empty(), "{:?}", before.errors);
        let ground = frame.nodes.iter().find(|n| &*n.id == "ground").unwrap().voxels.clone().unwrap();
        assert_eq!(before.surfaces.len(), 1, "the ground, whole");
        assert_eq!((before.surfaces[0].revision, before.surfaces[0].full), (0, true));
        assert_eq!((before.surfaces[0].quads, before.surfaces[0].hash), full_remesh_hash(&ground.grid));
        // the next frame is after the impact: one cut, and the surface follows the bricks it touched and no more
        let (after, frame) = draw(&mut renderer, &ev, 0.6);
        assert!(after.errors.is_empty(), "{:?}", after.errors);
        let cut = frame.nodes.iter().find(|n| &*n.id == "ground").unwrap().voxels.clone().unwrap();
        assert_eq!(cut.revision, 1);
        println!(
            "CUT: {} bricks, {} pieces, {} cells left",
            cut.changed_bricks.len(),
            cut.pieces.len(),
            cut.grid.count()
        );
        let owner = after.surfaces.iter().find(|s| s.body.is_none()).expect("the ground");
        assert_eq!(owner.revision, 1);
        assert!(!owner.full, "{camera}: a cut is an incremental remesh: {owner:?}");
        let (quads, hash) = full_remesh_hash(&cut.grid);
        assert_eq!(
            (owner.quads, owner.hash),
            (quads, hash),
            "{camera}: the surface after the cut is that of the cells made again"
        );
        assert!(
            owner.remeshed > 0 && owner.remeshed < 3 * 130,
            "{camera}: only the planes of the bricks of the cut: {}",
            owner.remeshed
        );
        // the part of the pillar that came away is drawn from its own cells
        assert!(!cut.pieces.is_empty());
        for piece in cut.pieces.iter().filter(|p| p.enabled) {
            let drawn = after.surfaces.iter().find(|s| s.body == Some(piece.body)).expect("the piece is drawn");
            assert_eq!((drawn.quads, drawn.hash), full_remesh_hash(&piece.grid), "{camera}: piece {}", piece.body);
        }
        // a renderer that never saw the ground whole draws the same picture, bit for bit
        let mut fresh = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let (again, _) = draw(&mut fresh, &ev, 0.6);
        assert!(again.surfaces.iter().find(|s| s.body.is_none()).unwrap().full);
        assert_eq!(after.pixels, again.pixels, "{camera}: an incremental remesh draws what a full one does");
    }
}

#[test]
fn the_raster_renderer_and_the_path_tracer_cover_the_same_pixels_but_for_the_edges_of_the_ground_after_the_cut() {
    let Some(gpu) = gpu() else { return };
    let mut masks = Vec::new();
    for camera in ["", r#"renderer="pathtrace" pathSamples="8" maxBounces="2""#] {
        let (_dir, ev) = scene("coverage", camera);
        let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let (shot, _) = draw(&mut renderer, &ev, 0.6);
        assert!(shot.errors.is_empty(), "{:?}", shot.errors);
        // background #101010 is 0.0052 in linear light
        masks.push(shot.pixels.iter().map(|p| p[0] > 0.02 || p[1] > 0.02 || p[2] > 0.02).collect::<Vec<_>>());
    }
    let (raster, traced) = (&masks[0], &masks[1]);
    let covered = raster.iter().filter(|c| **c).count();
    let differ: Vec<usize> = (0..raster.len()).filter(|i| raster[*i] != traced[*i]).collect();
    println!("COVERAGE: {covered} pixels by the raster renderer, {} differ from the path tracer's", differ.len());
    assert!(covered > 96 * 96 / 4, "the ground fills a good part of the picture: {covered}");
    assert!(
        differ.len() * 100 <= covered,
        "at most one pixel in a hundred of what is covered: {} of {covered}",
        differ.len()
    );
    let edge = |mask: &[bool], i: usize| {
        let (x, y) = ((i % 96) as i32, (i / 96) as i32);
        (-1..=1).any(|dy| {
            (-1..=1).any(|dx| {
                let (nx, ny) = (x + dx, y + dy);
                (0..96).contains(&nx) && (0..96).contains(&ny) && mask[(ny * 96 + nx) as usize] != mask[i]
            })
        })
    };
    for i in differ {
        assert!(edge(raster, i) || edge(traced, i), "pixel ({}, {}) differs and is not at an edge", i % 96, i / 96);
    }
}
