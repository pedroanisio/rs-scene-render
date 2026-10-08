//! What the content-keyed cache of derived products costs against what it saves, on a body of about a million cells: the fingerprint, the comparison
//! of the content, and each product worked out and found.
use sr_3d::occupancy::Occupancy;
use sr_eval::voxel_cache::DerivedCache;
use std::time::Instant;

fn time<T>(what: &str, runs: u32, mut f: impl FnMut() -> T) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..runs {
        let start = Instant::now();
        std::hint::black_box(f());
        best = best.min(start.elapsed().as_secs_f64() * 1e3);
    }
    println!("  {what:<44} {best:>9.2} ms");
    best
}

fn main() {
    // a ground slab 200 by 25 by 200 cells with a hole (about a million cells), a bar and a loose lump
    let mut cells = Vec::new();
    for k in 0..200 {
        for j in 0..25 {
            for i in 0..200 {
                if (i - 100) * (i - 100) + (k - 100) * (k - 100) > 400 {
                    cells.push(([i, j, k], 1u8 + ((i + j + k) % 3) as u8));
                }
            }
        }
    }
    let o = Occupancy::from_cells(cells).unwrap();
    println!("{} cells in {} bricks (best of 5 on a shared machine)", o.count(), o.bricks().count());
    let size = [0.25; 3];
    time("fingerprint", 5, || o.fingerprint());
    time("moments + properties", 5, || o.moments().properties(size, 2400.0).unwrap());
    time("components", 3, || o.components());
    time("body (cells in scan order)", 5, || sr_eval::voxels::body(&o, size, 2400.0, 1.0).unwrap());
    let mut cache = DerivedCache::new(1 << 30);
    let first =
        time("cache: properties, first (key, work, snapshot)", 1, || cache.properties(&o, size, 2400.0).unwrap());
    let hit = time("cache: properties, hit (key + compare)", 5, || cache.properties(&o, size, 2400.0).unwrap());
    cache.body(&o, size, 2400.0, 1.0).unwrap();
    time("cache: body, hit (key + compare)", 5, || cache.body(&o, size, 2400.0, 1.0).unwrap());
    cache.components(&o);
    time("cache: components, hit (key + compare)", 5, || cache.components(&o));
    println!(
        "  kept {} bytes for three products ({:.1} bytes a cell); a hit costs {:.1} percent of the first",
        cache.bytes(),
        cache.bytes() as f64 / o.count() as f64,
        100.0 * hit / first
    );
}
