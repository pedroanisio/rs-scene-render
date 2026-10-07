//! Where the smoke of one pyro volume is, frame by frame: the extent of the cells whose density is above a share
//! of the peak, along y (scene y points down), the smoke held in the cells at the top and the bottom face of the
//! domain, and the total, so that one can tell whether an open domain has cut a plume.
//!
//! usage: pyro_extent SCENE NODE-ID TIME... [-- ATTRIBUTES]   (ATTRIBUTES are added to the `<pyro>` element, for
//! example `follow="true" followMargin="6"`)
use sr_eval::Evaluator;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("scene path");
    let id = args.next().expect("node id");
    let mut times = Vec::new();
    let mut extra = String::new();
    while let Some(a) = args.next() {
        if a == "--" {
            extra = args.collect::<Vec<_>>().join(" ");
            break;
        }
        times.push(a.parse::<f64>().expect("a time"));
    }
    let mut xml = std::fs::read_to_string(&path).unwrap();
    if !extra.is_empty() {
        xml = xml.replacen("<pyro ", &format!("<pyro {extra} "), 1);
    }
    let base = std::path::Path::new(&path).parent().map(|p| p.to_path_buf());
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: false, base_dir: base }).unwrap();
    let ev = Evaluator::new(&doc, &Default::default()).unwrap();
    for time in times {
        let started = std::time::Instant::now();
        let frame = ev.evaluate(time);
        assert!(frame.failures.is_empty() && frame.problems.is_empty(), "{:?} {:?}", frame.failures, frame.problems);
        let node = frame.nodes.iter().find(|n| *n.id == *id).expect("node");
        let volume = node.sim_volume.as_ref().expect("a native pyro volume");
        let grid = volume.data.grid("density").unwrap();
        let t = grid.transform();
        let h = t.index_to_world([1.0, 0.0, 0.0])[0] - t.index_to_world([0.0, 0.0, 0.0])[0];
        // every stored voxel: (y in the object's axes, density)
        let mut voxels: Vec<(f64, f64)> = Vec::new();
        for (key, values) in grid.bricks() {
            for (k, v) in values.iter().enumerate() {
                if *v > 0.0 {
                    let (i, j, l) = ((k % 8) as i32, ((k / 8) % 8) as i32, (k / 64) as i32);
                    let p =
                        t.index_to_world([(key[0] * 8 + i) as f64, (key[1] * 8 + j) as f64, (key[2] * 8 + l) as f64]);
                    voxels.push((p[1], *v as f64));
                }
            }
        }
        let peak = voxels.iter().map(|v| v.1).fold(0.0, f64::max);
        let total: f64 = voxels.iter().map(|v| v.1).sum();
        let (ylo, yhi) = (
            voxels.iter().map(|v| v.0).fold(f64::INFINITY, f64::min),
            voxels.iter().map(|v| v.0).fold(f64::NEG_INFINITY, f64::max),
        );
        let extent = |share: f64| {
            let mut ys = voxels.iter().filter(|v| v.1 > share * peak).map(|v| v.0);
            let first = ys.next();
            first.map(|f| ys.fold((f, f), |(lo, hi), y| (lo.min(y), hi.max(y))))
        };
        // the smoke within 3 cells of the top face and of the bottom face of the stored range of the domain
        let face = |top: bool| -> f64 {
            let edge = if top { ylo } else { yhi };
            voxels.iter().filter(|v| (v.0 - edge).abs() < 3.0 * h).map(|v| v.1).sum()
        };
        // where the window is: the y of the first cell of the grid in the object's axes, and the key of the whole volume
        let window_y = t.index_to_world([0.0, 0.0, 0.0])[1] - 0.5 * h;
        println!(
            "WINDOW t={time}: the first cell of the grid is at y = {window_y}, the key of the volume {:016x}",
            volume.key
        );
        println!(
            "EXTENT t={time}: peak {peak:.4} total {total:.2} stored y {ylo:.1}..{yhi:.1}; above 1e-1 of the peak {:?}, 1e-2 {:?}, 1e-3 {:?}, 1e-4 {:?}; in the 3 cells at the top face {:.3e}, bottom face {:.3e} ({:.2} s)",
            extent(1e-1).map(|e| (e.0.round(), e.1.round())),
            extent(1e-2).map(|e| (e.0.round(), e.1.round())),
            extent(1e-3).map(|e| (e.0.round(), e.1.round())),
            extent(1e-4).map(|e| (e.0.round(), e.1.round())),
            face(true),
            face(false),
            started.elapsed().as_secs_f64()
        );
    }
}
