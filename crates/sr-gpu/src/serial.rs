//! SREP 67, Semantics 5 to 7: the serial image operations `error-diffusion` and `segmented-sort`, in integer
//! arithmetic on 16-bit display-encoded values. Each output pixel depends on pixels processed before it, which a pixel
//! shader cannot express, so they run on the CPU over the node's content rectangle.
//!
//! The operations are exact functions of their integer input (the SREP's determinism claim). The conversion between the
//! renderer's stored working colours and those integers is in floating point and is not covered by the claim.

/// An error-diffusion kernel: entries (dx, dy, weight) and the denominator (SREP 67, Semantics 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kernel {
    pub entries: &'static [(i32, i32, i64)],
    pub denominator: i64,
}

/// The kernels by their `@kernel` names. The weights are those of the SREP's table, which matches the values as they are
/// commonly republished (see the SREP's references); the original publications were not available to check against.
pub fn kernel(name: &str) -> Option<Kernel> {
    const FS: &[(i32, i32, i64)] = &[(1, 0, 7), (-1, 1, 3), (0, 1, 5), (1, 1, 1)];
    const ATKINSON: &[(i32, i32, i64)] = &[(1, 0, 1), (2, 0, 1), (-1, 1, 1), (0, 1, 1), (1, 1, 1), (0, 2, 1)];
    const JJN: &[(i32, i32, i64)] = &[
        (1, 0, 7),
        (2, 0, 5),
        (-2, 1, 3),
        (-1, 1, 5),
        (0, 1, 7),
        (1, 1, 5),
        (2, 1, 3),
        (-2, 2, 1),
        (-1, 2, 3),
        (0, 2, 5),
        (1, 2, 3),
        (2, 2, 1),
    ];
    const STUCKI: &[(i32, i32, i64)] = &[
        (1, 0, 8),
        (2, 0, 4),
        (-2, 1, 2),
        (-1, 1, 4),
        (0, 1, 8),
        (1, 1, 4),
        (2, 1, 2),
        (-2, 2, 1),
        (-1, 2, 2),
        (0, 2, 4),
        (1, 2, 2),
        (2, 2, 1),
    ];
    const BURKES: &[(i32, i32, i64)] = &[(1, 0, 8), (2, 0, 4), (-2, 1, 2), (-1, 1, 4), (0, 1, 8), (1, 1, 4), (2, 1, 2)];
    const SIERRA: &[(i32, i32, i64)] = &[
        (1, 0, 5),
        (2, 0, 3),
        (-2, 1, 2),
        (-1, 1, 4),
        (0, 1, 5),
        (1, 1, 4),
        (2, 1, 2),
        (-1, 2, 2),
        (0, 2, 3),
        (1, 2, 2),
    ];
    let (entries, denominator) = match name {
        "floyd-steinberg" => (FS, 16),
        "atkinson" => (ATKINSON, 8),
        "jarvis-judice-ninke" => (JJN, 48),
        "stucki" => (STUCKI, 42),
        "burkes" => (BURKES, 32),
        "sierra" => (SIERRA, 32),
        _ => return None,
    };
    Some(Kernel { entries, denominator })
}

/// Error diffusion of `px` (row-major, `w` × `h`, 16-bit R, G, B) to the colours of `palette` (SREP 67, Semantics 6):
/// returns the chosen palette colour of every pixel.
pub fn error_diffusion(
    px: &[[u16; 3]],
    w: usize,
    h: usize,
    k: Kernel,
    palette: &[[u16; 3]],
    serpentine: bool,
) -> Vec<[u16; 3]> {
    assert_eq!(px.len(), w * h, "pixels for {w} x {h}");
    assert!(!palette.is_empty(), "a palette has a colour");
    // working values: unclamped integers that gather the diffused error
    let mut work: Vec<[i64; 3]> = px.iter().map(|p| p.map(i64::from)).collect();
    let mut out = vec![[0u16; 3]; w * h];
    for y in 0..h {
        let reverse = serpentine && y % 2 == 1;
        for i in 0..w {
            let x = if reverse { w - 1 - i } else { i };
            let v = work[y * w + x];
            // the nearest palette colour; a tie goes to the lowest index
            let mut best = 0;
            let mut best_d = i128::MAX;
            for (k, p) in palette.iter().enumerate() {
                let d: i128 = (0..3).map(|c| (v[c] - p[c] as i64) as i128).map(|e| e * e).sum();
                if d < best_d {
                    (best, best_d) = (k, d);
                }
            }
            let q = palette[best];
            out[y * w + x] = q;
            let e = [v[0] - q[0] as i64, v[1] - q[1] as i64, v[2] - q[2] as i64];
            for &(dx, dy, wgt) in k.entries {
                // a reversed row mirrors the kernel in x
                let dx = if reverse { -dx } else { dx };
                let (xx, yy) = (x as i64 + dx as i64, y as i64 + dy as i64);
                if xx < 0 || yy < 0 || xx >= w as i64 || yy >= h as i64 {
                    continue;
                }
                let t = &mut work[yy as usize * w + xx as usize];
                for c in 0..3 {
                    t[c] += (e[c] * wgt).div_euclid(k.denominator);
                }
            }
        }
    }
    out
}

/// The sort key of a 16-bit colour: `luma = (13933 R + 46871 G + 4732 B) >> 16`, or one channel (SREP 67, Semantics 7).
pub fn sort_key(p: [u16; 3], key: &str) -> u32 {
    match key {
        "red" => p[0] as u32,
        "green" => p[1] as u32,
        "blue" => p[2] as u32,
        _ => ((13933u64 * p[0] as u64 + 46871u64 * p[1] as u64 + 4732u64 * p[2] as u64) >> 16) as u32,
    }
}

/// Sorts the segments of one line in place (SREP 67, Semantics 7): the maximal runs whose key lies in `[lo, hi]`, each
/// reordered stably by key, ascending or descending. `T` carries the whole pixel (RGBA together); `key` reads its key.
pub fn sort_line<T: Copy>(line: &mut [T], key: impl Fn(&T) -> u32, lo: u32, hi: u32, descending: bool) {
    let inside = |k: u32| lo <= k && k <= hi;
    let mut i = 0;
    while i < line.len() {
        if !inside(key(&line[i])) {
            i += 1;
            continue;
        }
        let start = i;
        while i < line.len() && inside(key(&line[i])) {
            i += 1;
        }
        let seg = &mut line[start..i];
        // a stable sort: equal keys keep their order (descending reverses the comparison, not the run)
        if descending {
            seg.sort_by_key(|b| std::cmp::Reverse(key(b)));
        } else {
            seg.sort_by_key(|p| key(p));
        }
    }
}

/// The threshold of `low` or `high` (0..1) as a 16-bit key: `round(v · 65535)`.
pub fn threshold(v: f64) -> u32 {
    (v.clamp(0.0, 1.0) * 65535.0).round() as u32
}

/// A display-encoded value in [0, 1] as 16 bits: `round(clamp(e, 0, 1) · 65535)`.
pub fn to16(e: f64) -> u16 {
    (e.clamp(0.0, 1.0) * 65535.0).round() as u16
}

/// A `#RRGGBB[AA]` palette colour as 16-bit values `257 · code8`.
pub fn palette16(rgba8: [u8; 4]) -> [u16; 3] {
    [257 * rgba8[0] as u16, 257 * rgba8[1] as u16, 257 * rgba8[2] as u16]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kit's reference (sr-core conformance/tools/srep67_cases.py, `diffuse`): mid grey (#808080 = 32896) on 64 × 64
    /// with black and white. Floyd–Steinberg: 127.7 mean and white, black, white, black in the first row; Atkinson:
    /// 127.5 and white, black, black, white.
    #[test]
    fn mid_grey_matches_the_kit() {
        let grey = vec![[32896u16; 3]; 64 * 64];
        let bw = [[0u16; 3], [65535; 3]];
        for (name, mean, first) in
            [("floyd-steinberg", 127.7, [255.0, 0.0, 255.0, 0.0]), ("atkinson", 127.5, [255.0, 0.0, 0.0, 255.0])]
        {
            let out = error_diffusion(&grey, 64, 64, kernel(name).unwrap(), &bw, false);
            let white = out.iter().filter(|p| p[0] == 65535).count();
            let m = (255.0 * white as f64 / 4096.0 * 10.0).round() / 10.0;
            assert_eq!(m, mean, "{name}");
            let row: Vec<f64> = out[..4].iter().map(|p| if p[0] == 65535 { 255.0 } else { 0.0 }).collect();
            assert_eq!(row, first, "{name}");
        }
    }

    #[test]
    fn every_kernel_keeps_the_mean_of_a_flat_field_and_diffuses_at_most_the_error() {
        let bw = [[0u16; 3], [65535; 3]];
        for name in ["floyd-steinberg", "atkinson", "jarvis-judice-ninke", "stucki", "burkes", "sierra"] {
            let k = kernel(name).unwrap();
            let total: i64 = k.entries.iter().map(|e| e.2).sum();
            // all of the error, except Atkinson, which diffuses six eighths
            assert_eq!(total, if name == "atkinson" { 6 } else { k.denominator }, "{name}");
            assert!(k.entries.iter().all(|&(dx, dy, _)| dy > 0 || dx > 0), "{name}: an entry goes back in the scan");
            for (level, tolerance) in [(16448u16, 0.02), (32896, 0.02), (49344, 0.02)] {
                let out = error_diffusion(&vec![[level; 3]; 128 * 128], 128, 128, k, &bw, false);
                let mean = out.iter().map(|p| p[0] as f64).sum::<f64>() / (128.0 * 128.0 * 65535.0);
                let want = level as f64 / 65535.0;
                // Atkinson loses a quarter of each error, so it drifts in the shadows and highlights
                let tol = if name == "atkinson" { 0.08 } else { tolerance };
                assert!((mean - want).abs() < tol, "{name} at {want}: {mean}");
            }
        }
    }

    #[test]
    fn the_error_floors_toward_minus_infinity_and_a_tie_takes_the_lowest_index() {
        // one row of 2: 1 above black, then black; Floyd–Steinberg sends floor(1 * 7 / 16) = 0 right
        let out = error_diffusion(
            &[[1, 1, 1], [0, 0, 0]],
            2,
            1,
            kernel("floyd-steinberg").unwrap(),
            &[[0; 3], [2; 3]],
            false,
        );
        // 1 is equally far from 0 and 2: the lowest index (0) wins
        assert_eq!(out, vec![[0; 3], [0; 3]]);
        // -1 error: floor(-1 * 7 / 16) = -1, not 0
        let out = error_diffusion(
            &[[1, 1, 1], [2, 2, 2]],
            2,
            1,
            kernel("floyd-steinberg").unwrap(),
            &[[0; 3], [2; 3], [4; 3]],
            false,
        );
        assert_eq!(out[0], [0; 3]);
        let out2 = error_diffusion(
            &[[3, 3, 3], [2, 2, 2]],
            2,
            1,
            kernel("floyd-steinberg").unwrap(),
            &[[0; 3], [4; 3]],
            false,
        );
        // 3 -> 4 (error -1); the next value gains floor(-7 / 16) = -1: 1 -> 0
        assert_eq!(out2, vec![[4; 3], [0; 3]]);
    }

    #[test]
    fn serpentine_scans_odd_rows_right_to_left_with_the_kernel_mirrored() {
        // row 0 is black (no error), so row 1 depends on itself alone: scanned right to left it is the mirror of its
        // mirror image scanned left to right
        let w = 16;
        let row: Vec<[u16; 3]> = (0..w).map(|i| [(i as u16) * 4000; 3]).collect();
        let px = [vec![[0u16; 3]; w], row.clone()].concat();
        let bw = [[0u16; 3], [65535; 3]];
        let k = kernel("floyd-steinberg").unwrap();
        let snake = error_diffusion(&px, w, 2, k, &bw, true);
        let mirrored: Vec<[u16; 3]> = row.iter().rev().copied().collect();
        let alone = error_diffusion(&mirrored, w, 1, k, &bw, false);
        let back: Vec<[u16; 3]> = alone.into_iter().rev().collect();
        assert_eq!(snake[w..], back[..]);
        // and differs from the left-to-right scan of that row
        let plain = error_diffusion(&px, w, 2, k, &bw, false);
        assert_ne!(plain[w..], snake[w..]);
        assert_eq!(plain[..w], snake[..w]);
    }

    #[test]
    fn segments_sort_stably_inside_the_thresholds() {
        // kit: white then black sorts to black then white ascending, white then black descending
        let key = |p: &(u32, usize)| p.0;
        let mut line: Vec<(u32, usize)> = (0..8).map(|i| (if i < 4 { 65535 } else { 0 }, i)).collect();
        sort_line(&mut line, key, 0, 65535, false);
        assert_eq!(line.iter().map(|p| p.0).collect::<Vec<_>>(), [0, 0, 0, 0, 65535, 65535, 65535, 65535]);
        // stable: equal keys keep their order
        assert_eq!(line.iter().map(|p| p.1).collect::<Vec<_>>(), [4, 5, 6, 7, 0, 1, 2, 3]);
        sort_line(&mut line, key, 0, 65535, true);
        assert_eq!(line.iter().map(|p| p.1).collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5, 6, 7]);
        // kit threshold: high = 0.5 leaves the white pixels outside every segment: nothing moves
        let mut line: Vec<(u32, usize)> = (0..8).map(|i| (if i < 4 { 65535 } else { 0 }, i)).collect();
        sort_line(&mut line, key, threshold(0.0), threshold(0.5), false);
        assert_eq!(line.iter().map(|p| p.1).collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5, 6, 7]);
        // two segments separated by an outside pixel sort apart
        let mut line = vec![(5, 0), (3, 1), (65535, 2), (2, 3), (1, 4)];
        sort_line(&mut line, key, 0, 100, false);
        assert_eq!(line.iter().map(|p| p.1).collect::<Vec<_>>(), [1, 0, 2, 4, 3]);
    }

    #[test]
    fn luma_weights_are_rec709_and_sum_to_one() {
        assert_eq!(13933 + 46871 + 4732, 65536);
        assert_eq!(sort_key([65535; 3], "luma"), 65535);
        assert_eq!(sort_key([65535, 0, 0], "luma"), ((13933u64 * 65535) >> 16) as u32);
        assert_eq!(sort_key([1, 2, 3], "green"), 2);
        // 0.2126, 0.7152, 0.0722 (ITU-R BT.709-6) times 65536, rounded
        for (w, c) in [(13933, 0.2126), (46871, 0.7152), (4732, 0.0722)] {
            assert_eq!(w, (c * 65536.0_f64).round() as i64);
        }
    }
}
