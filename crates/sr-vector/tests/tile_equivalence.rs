//! The tile encoder's equivalence gate: `sr_vector::tile::encode` must
//! produce, bit for bit, what the original encoder produced (the same
//! tiles, the same per-tile command order, the same numeric values).
//! `reference` is a verbatim copy of that original; the tests feed both
//! with seeded synthetic scenes (random polylines, curves, strokes, masks,
//! mattes, nested groups, degenerate geometry) at several target sizes.
//!
//! The `#[ignore]` tests are micro-benchmarks on fixture-shaped scenes:
//! `cargo test --release -p sr-vector --test tile_equivalence -- --ignored --nocapture`

use sr_vector::geom::{p, Xf, P};
use sr_vector::measure::{self, TrimMode};
use sr_vector::path::Path;
use sr_vector::scene::{Cmd, FillRule, Gradient, GradientKind, MaskOp, MatteMode, Paint, Scene};
use sr_vector::stroke::{self, Cap, Join, Style};
use sr_vector::tile::{self, Encoded};
use sr_vector::{shapes, Poly};

/// The original encoder (as of commit 16bd88c), copied verbatim.
#[allow(dead_code, clippy::all)]
mod reference {
    use sr_vector::tile::{kind, Encoded, GpuCmd, MAX_DEPTH, TILE};
    use sr_vector::{Cmd, FillRule, MaskOp, MatteMode, Paint, Poly, Scene};

    enum Backdrop {
        Uniform(f32),
        Rows([f32; 16]),
    }

    struct PathTile {
        tile: u32,
        pieces: Vec<[f32; 4]>,
        backdrop: Backdrop,
    }

    /// Tiles a path: the tiles it touches with their pieces and backdrops.
    fn tile_path(polys: &[Poly], size: [u32; 2]) -> Vec<PathTile> {
        let t = TILE as f64;
        let (tx, ty) = (size[0].div_ceil(TILE) as i64, size[1].div_ceil(TILE) as i64);
        // row range of the path
        let (mut ymin, mut ymax) = (f64::INFINITY, f64::NEG_INFINITY);
        for q in polys {
            for pt in &q.pts {
                ymin = ymin.min(pt.y);
                ymax = ymax.max(pt.y);
            }
        }
        if ymin.partial_cmp(&ymax) != Some(std::cmp::Ordering::Less) || ymax <= 0.0 || ymin >= size[1] as f64 {
            return Vec::new();
        }
        let row0 = ((ymin / t).floor() as i64).max(0);
        let row1 = (((ymax / t).ceil() as i64) - 1).min(ty - 1);
        let mut rows: Vec<Vec<[f64; 4]>> = vec![Vec::new(); (row1 - row0 + 1).max(0) as usize];
        for q in polys {
            let n = q.pts.len();
            if n < 2 {
                continue;
            }
            // fills close every subpath
            for k in 0..n {
                let (a, b) = (q.pts[k], q.pts[(k + 1) % n]);
                if a.y == b.y || !(a.x.is_finite() && a.y.is_finite() && b.x.is_finite() && b.y.is_finite()) {
                    continue;
                }
                let (lo, hi) = (a.y.min(b.y), a.y.max(b.y));
                if hi <= 0.0 || lo >= size[1] as f64 {
                    continue;
                }
                let r0 = ((lo / t).floor() as i64).max(row0);
                let r1 = (((hi / t).ceil() as i64) - 1).min(row1);
                let inv = (b.x - a.x) / (b.y - a.y);
                for r in r0..=r1 {
                    let (top, bot) = (r as f64 * t, (r + 1) as f64 * t);
                    let ya = a.y.clamp(top, bot);
                    let yb = b.y.clamp(top, bot);
                    if ya == yb {
                        continue;
                    }
                    rows[(r - row0) as usize].push([
                        a.x + (ya - a.y) * inv,
                        ya - top,
                        a.x + (yb - a.y) * inv,
                        yb - top,
                    ]);
                }
            }
        }
        let xmax = |pc: &[f64; 4]| pc[0].max(pc[2]);
        let xmin = |pc: &[f64; 4]| pc[0].min(pc[2]);
        let mut out = Vec::new();
        for (ri, mut pieces) in rows.into_iter().enumerate() {
            if pieces.is_empty() {
                continue;
            }
            let r = row0 + ri as i64;
            pieces.sort_by(|a, b| xmin(a).total_cmp(&xmin(b)));
            let hi = pieces.iter().map(xmax).fold(f64::NEG_INFINITY, f64::max);
            let c0 = ((xmin(&pieces[0]) / t).floor() as i64).max(0);
            let c1 = ((hi / t).floor() as i64).min(tx - 1);
            let mut acc = [0.0f64; 16];
            // pieces entering the sweep in xmin order; active ones overlap the current column
            let mut next = 0;
            let mut active: Vec<usize> = Vec::new();
            let add = |acc: &mut [f64; 16], pc: &[f64; 4]| {
                let (y0, y1) = (pc[1].min(pc[3]), pc[1].max(pc[3]));
                let (p0, p1) = (y0.floor().max(0.0) as usize, (y1.ceil() as usize).min(16));
                for (py, a) in acc.iter_mut().enumerate().take(p1).skip(p0) {
                    let (fy0, fy1) = (py as f64, py as f64 + 1.0);
                    *a += pc[3].clamp(fy0, fy1) - pc[1].clamp(fy0, fy1);
                }
            };
            // pieces wholly left of the first column only feed the backdrop
            while next < pieces.len() && xmax(&pieces[next]) <= c0 as f64 * t && xmin(&pieces[next]) < c0 as f64 * t {
                add(&mut acc, &pieces[next]);
                next += 1;
            }
            for c in c0..=c1 {
                let x0 = c as f64 * t;
                while next < pieces.len() && xmin(&pieces[next]) < x0 + t {
                    active.push(next);
                    next += 1;
                }
                // pieces that ended left of this column move into the backdrop
                active.retain(|&k| {
                    if xmax(&pieces[k]) <= x0 {
                        add(&mut acc, &pieces[k]);
                        false
                    } else {
                        true
                    }
                });
                let nonzero = acc.iter().any(|v| v.abs() > 1e-9);
                if active.is_empty() && !nonzero {
                    continue;
                }
                let mine: Vec<[f32; 4]> = active
                    .iter()
                    .map(|&k| pieces[k])
                    .map(|pc| [(pc[0] - x0) as f32, pc[1] as f32, (pc[2] - x0) as f32, pc[3] as f32])
                    .collect();
                let uniform = acc.iter().all(|v| (v - acc[0]).abs() < 1e-9);
                let backdrop =
                    if uniform { Backdrop::Uniform(acc[0] as f32) } else { Backdrop::Rows(acc.map(|v| v as f32)) };
                out.push(PathTile { tile: (r * tx + c) as u32, pieces: mine, backdrop });
            }
        }
        out.sort_by_key(|p| p.tile);
        out
    }

    enum Node {
        Fill {
            tiles: Vec<PathTile>,
            rule: FillRule,
            paint: u32,
            opacity: f64,
        },
        Group {
            children: Vec<Node>,
            masks: Vec<(Vec<PathTile>, FillRule, MaskOp, f64, bool)>,
            mask_init: f64,
            opacity: f64,
            matte: Option<(Vec<Node>, MatteMode)>,
        },
    }

    fn tiles_of(nodes: &[Node]) -> Vec<u32> {
        let mut v = Vec::new();
        for n in nodes {
            match n {
                Node::Fill { tiles, .. } => v.extend(tiles.iter().map(|t| t.tile)),
                Node::Group { children, .. } => v.extend(tiles_of(children)),
            }
        }
        v.sort_unstable();
        v.dedup();
        v
    }

    struct Enc {
        size: [u32; 2],
        /// Tiles of every Fill and Mask command, computed up front in parallel.
        pre: Vec<Option<Vec<PathTile>>>,
        lists: Vec<Vec<GpuCmd>>,
        pieces: Vec<[f32; 4]>,
        backdrops: Vec<f32>,
        paints: Vec<Paint>,
        flattened: usize,
    }

    impl Enc {
        fn path_cmd(&mut self, kind: u32, pt: Option<&PathTile>, flags: u32, paint: u32, param: f64) -> GpuCmd {
            let mut c = GpuCmd { kind, paint, flags, param: param as f32, backdrop: u32::MAX, ..Default::default() };
            if let Some(pt) = pt {
                c.piece_off = self.pieces.len() as u32;
                c.piece_count = pt.pieces.len() as u32;
                self.pieces.extend_from_slice(&pt.pieces);
                match &pt.backdrop {
                    Backdrop::Uniform(v) => c.backdrop_val = *v,
                    Backdrop::Rows(r) => {
                        c.backdrop = self.backdrops.len() as u32;
                        self.backdrops.extend_from_slice(r);
                    }
                }
            }
            c
        }

        fn parse(&mut self, cmds: &[Cmd], i: &mut usize, until_matte: bool) -> Vec<Node> {
            let mut out = Vec::new();
            while *i < cmds.len() {
                match &cmds[*i] {
                    Cmd::Fill { polys, rule, paint, opacity } => {
                        let tiles =
                            self.pre.get_mut(*i).and_then(Option::take).unwrap_or_else(|| tile_path(polys, self.size));
                        *i += 1;
                        if tiles.is_empty() {
                            continue;
                        }
                        self.paints.push(paint.clone());
                        out.push(Node::Fill {
                            tiles,
                            rule: *rule,
                            paint: (self.paints.len() - 1) as u32,
                            opacity: *opacity,
                        });
                    }
                    Cmd::Push { mask_init } => {
                        *i += 1;
                        let children = self.parse(cmds, i, false);
                        let mut masks = Vec::new();
                        let mut matte = None;
                        let mut opacity = 1.0;
                        while *i < cmds.len() {
                            match &cmds[*i] {
                                Cmd::Mask { polys, rule, op, opacity, invert } => {
                                    let tiles = self
                                        .pre
                                        .get_mut(*i)
                                        .and_then(Option::take)
                                        .unwrap_or_else(|| tile_path(polys, self.size));
                                    masks.push((tiles, *rule, *op, *opacity, *invert));
                                    *i += 1;
                                }
                                Cmd::PushMatte => {
                                    *i += 1;
                                    let m = self.parse(cmds, i, true);
                                    if let Some(Cmd::PopMatte { mode, opacity: o }) = cmds.get(*i) {
                                        matte = Some((m, *mode));
                                        opacity = *o;
                                    }
                                    *i += 1;
                                    break;
                                }
                                Cmd::Pop { opacity: o } => {
                                    opacity = *o;
                                    *i += 1;
                                    break;
                                }
                                _ => break,
                            }
                        }
                        out.push(Node::Group { children, masks, mask_init: *mask_init, opacity, matte });
                    }
                    Cmd::Mask { .. } | Cmd::Pop { .. } | Cmd::PushMatte => return out,
                    Cmd::PopMatte { .. } => {
                        if until_matte {
                            return out;
                        }
                        *i += 1;
                    }
                }
            }
            out
        }

        fn emit(&mut self, nodes: &[Node], depth: usize, opacity_mul: f64) {
            for n in nodes {
                match n {
                    Node::Fill { tiles, rule, paint, opacity } => {
                        for pt in tiles {
                            let flags = (*rule == FillRule::EvenOdd) as u32;
                            let c = self.path_cmd(kind::FILL, Some(pt), flags, *paint, opacity * opacity_mul);
                            self.lists[pt.tile as usize].push(c);
                        }
                    }
                    Node::Group { children, masks, mask_init, opacity, matte } => {
                        let need = 1 + matte.is_some() as usize;
                        let plain = masks.is_empty() && matte.is_none() && *mask_init >= 1.0;
                        if depth + need > MAX_DEPTH || (plain && children.len() <= 1) {
                            // no isolation needed (or no room): opacity folds into the content
                            if !plain {
                                self.flattened += 1;
                            }
                            self.emit(children, depth, opacity_mul * opacity);
                            continue;
                        }
                        let tiles = tiles_of(children);
                        for &t in &tiles {
                            let c = GpuCmd { kind: kind::PUSH, param: *mask_init as f32, ..Default::default() };
                            self.lists[t as usize].push(c);
                        }
                        self.emit(children, depth + 1, 1.0);
                        for (mt, rule, op, mop, inv) in masks {
                            let flags =
                                (*rule == FillRule::EvenOdd) as u32 | ((*inv as u32) << 1) | ((*op as u32) << 4);
                            for &t in &tiles {
                                let pt = mt.binary_search_by_key(&t, |p| p.tile).ok().map(|k| &mt[k]);
                                let c = self.path_cmd(kind::MASK, pt, flags, 0, *mop);
                                self.lists[t as usize].push(c);
                            }
                        }
                        match matte {
                            Some((m, mode)) => {
                                for &t in &tiles {
                                    let c = GpuCmd { kind: kind::PUSH, param: 1.0, ..Default::default() };
                                    self.lists[t as usize].push(c);
                                }
                                self.emit(m, depth + 2, 1.0);
                                for &t in &tiles {
                                    let c = GpuCmd {
                                        kind: kind::POP_MATTE,
                                        flags: (*mode as u32) << 4,
                                        param: (*opacity * opacity_mul) as f32,
                                        ..Default::default()
                                    };
                                    self.lists[t as usize].push(c);
                                }
                            }
                            None => {
                                for &t in &tiles {
                                    let c = GpuCmd {
                                        kind: kind::POP,
                                        param: (*opacity * opacity_mul) as f32,
                                        ..Default::default()
                                    };
                                    self.lists[t as usize].push(c);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Encodes a scene (in target pixels) for a `size` target.
    pub fn encode(scene: &Scene, size: [u32; 2]) -> Encoded {
        let tiles = [size[0].div_ceil(TILE), size[1].div_ceil(TILE)];
        let pre = tile_all(scene, size);
        let mut e = Enc {
            size,
            pre,
            lists: vec![Vec::new(); (tiles[0] * tiles[1]) as usize],
            pieces: Vec::new(),
            backdrops: Vec::new(),
            paints: Vec::new(),
            flattened: 0,
        };
        let mut i = 0;
        let mut nodes = Vec::new();
        while i < scene.cmds.len() {
            let before = i;
            nodes.extend(e.parse(&scene.cmds, &mut i, false));
            if i == before {
                i += 1;
            }
        }
        // matte content in tiles the layer misses is removed by `balanced`
        e.emit(&nodes, 0, 1.0);
        let mut ranges = vec![[0u32; 2]; (tiles[0] * tiles[1]) as usize];
        let mut cmds = Vec::new();
        for (t, raw) in e.lists.iter().enumerate() {
            if raw.is_empty() {
                continue;
            }
            let list = balanced(raw);
            if !list.is_empty() {
                ranges[t] = [cmds.len() as u32, list.len() as u32];
                cmds.extend(list);
            }
        }
        Encoded {
            size,
            tiles,
            ranges,
            cmds,
            pieces: e.pieces,
            backdrops: e.backdrops,
            paints: e.paints,
            flattened_layers: e.flattened,
        }
    }

    /// Tiles every path of the scene, spread over the available cores.
    fn tile_all(scene: &Scene, size: [u32; 2]) -> Vec<Option<Vec<PathTile>>> {
        let polys: Vec<Option<&[Poly]>> = scene
            .cmds
            .iter()
            .map(|c| match c {
                Cmd::Fill { polys, .. } | Cmd::Mask { polys, .. } => Some(polys.as_slice()),
                _ => None,
            })
            .collect();
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(16);
        let work: usize = polys.iter().flatten().map(|p| p.iter().map(|q| q.pts.len()).sum::<usize>()).sum();
        if threads <= 1 || work < 20_000 {
            return polys.iter().map(|p| p.map(|p| tile_path(p, size))).collect();
        }
        let chunk = polys.len().div_ceil(threads);
        let mut out: Vec<Option<Vec<PathTile>>> = Vec::with_capacity(polys.len());
        std::thread::scope(|sc| {
            let handles: Vec<_> = polys
                .chunks(chunk)
                .map(|part| sc.spawn(move || part.iter().map(|p| p.map(|p| tile_path(p, size))).collect::<Vec<_>>()))
                .collect();
            for h in handles {
                out.extend(h.join().unwrap_or_default());
            }
        });
        out
    }

    /// Drops pushes and pops that cannot pair within one tile (matte content in tiles its layer misses).
    fn balanced(list: &[GpuCmd]) -> Vec<GpuCmd> {
        let mut depth = 0i32;
        let mut out = Vec::with_capacity(list.len());
        for c in list {
            match c.kind {
                kind::PUSH => depth += 1,
                kind::POP => depth -= 1,
                kind::POP_MATTE => depth -= 2,
                _ => {}
            }
            if depth < 0 {
                depth = 0;
                continue;
            }
            out.push(*c);
        }
        // unmatched pushes: close them
        while depth > 0 {
            out.push(GpuCmd { kind: kind::POP, param: 1.0, ..Default::default() });
            depth -= 1;
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Deterministic scene generation
// ---------------------------------------------------------------------------

/// splitmix64.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
    /// A point in and around the target.
    fn pt(&mut self, size: [u32; 2]) -> P {
        let (w, h) = (size[0] as f64, size[1] as f64);
        let mut q = p(self.range(-64.0, w + 64.0), self.range(-64.0, h + 64.0));
        match self.below(40) {
            0 => q.x = (q.x / 16.0).round() * 16.0,
            1 => q.y = (q.y / 16.0).round() * 16.0,
            2 => q = p(q.x.round(), q.y.round()),
            3 => q = p(q.x.round() + 0.5, q.y.round() + 0.5),
            _ => {}
        }
        q
    }
    fn paint(&mut self) -> Paint {
        match self.below(4) {
            0 | 1 => {
                Paint::Solid { rgba: [self.unit(), self.unit(), self.unit(), self.unit()], srgb: self.chance(0.5) }
            }
            2 => Paint::External { index: self.below(8) as u32, to_local: self.xf() },
            _ => {
                let kind = match self.below(3) {
                    0 => GradientKind::Linear { a: p(self.unit(), self.unit()), b: p(self.unit(), self.unit()) },
                    1 => GradientKind::Radial { c: p(0.5, 0.5), r: self.unit(), f: p(0.4, 0.6), fr: 0.0 },
                    _ => GradientKind::Conic { c: p(0.5, 0.5), start: self.range(0.0, 360.0) },
                };
                let n = 2 + self.below(3);
                let stops =
                    (0..n).map(|k| (k as f64 / (n - 1) as f64, [self.unit(), self.unit(), self.unit(), 1.0])).collect();
                Paint::Gradient(Box::new(Gradient {
                    kind,
                    stops,
                    spread: self.below(3) as u32,
                    to_gradient: self.xf(),
                }))
            }
        }
    }
    fn xf(&mut self) -> Xf {
        Xf::trs(
            p(self.range(-50.0, 50.0), self.range(-50.0, 50.0)),
            p(self.range(-10.0, 10.0), self.range(-10.0, 10.0)),
            self.range(-180.0, 180.0),
            self.range(0.2, 3.0),
            self.range(0.2, 3.0),
        )
    }
    fn rule(&mut self) -> FillRule {
        if self.chance(0.5) {
            FillRule::NonZero
        } else {
            FillRule::EvenOdd
        }
    }
}

fn poly(pts: Vec<P>, closed: bool) -> Poly {
    Poly { pts, closed }
}

/// Random polyline: 0 to 40 points, sometimes with special values.
fn polyline(rng: &mut Rng, size: [u32; 2]) -> Poly {
    let n = match rng.below(10) {
        0 => rng.below(3),
        1..=6 => 3 + rng.below(12),
        _ => 3 + rng.below(40),
    };
    let mut pts: Vec<P> = (0..n).map(|_| rng.pt(size)).collect();
    if n > 0 && rng.chance(0.08) {
        let k = rng.below(n);
        pts[k] = match rng.below(6) {
            0 => p(f64::NAN, pts[k].y),
            1 => p(pts[k].x, f64::NAN),
            2 => p(f64::INFINITY, pts[k].y),
            3 => p(pts[k].x, f64::NEG_INFINITY),
            4 => p(rng.range(-1e300, 1e300), pts[k].y),
            _ => p(pts[k].x, rng.range(-1e12, 1e12)),
        };
    }
    if n > 1 && rng.chance(0.05) {
        // a vertical or horizontal twin of a point: zero-length and axis-aligned edges
        let k = rng.below(n - 1);
        pts[k + 1] = if rng.chance(0.5) { pts[k] } else { p(pts[k].x + rng.range(-40.0, 40.0), pts[k].y) };
    }
    poly(pts, rng.chance(0.7))
}

/// A polygon well under a pixel across.
fn tiny(rng: &mut Rng, size: [u32; 2]) -> Poly {
    let c = rng.pt(size);
    let r = rng.range(1e-4, 1.0);
    let n = 3 + rng.below(4);
    let pts = (0..n)
        .map(|k| {
            let a = k as f64 / n as f64 * std::f64::consts::TAU;
            p(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    poly(pts, true)
}

/// An axis-aligned rectangle, often on tile boundaries, sometimes empty.
fn aligned_rect(rng: &mut Rng, size: [u32; 2]) -> Poly {
    let (w, h) = (size[0] as f64, size[1] as f64);
    let snap = |rng: &mut Rng, lo: f64, hi: f64| {
        let v = rng.range(lo, hi);
        match rng.below(5) {
            0 => (v / 16.0).round() * 16.0,
            1 => (v / 16.0).round() * 16.0 + 1e-9,
            2 => (v / 16.0).round() * 16.0 - 1e-9,
            3 => v.round(),
            _ => v,
        }
    };
    let x0 = snap(rng, -40.0, w + 40.0);
    let y0 = snap(rng, -40.0, h + 40.0);
    let x1 = if rng.chance(0.1) { x0 } else { snap(rng, -40.0, w + 40.0) };
    let y1 = if rng.chance(0.1) { y0 } else { snap(rng, -40.0, h + 40.0) };
    poly(vec![p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1)], true)
}

/// Whole-target and off-target rectangles.
fn extreme_rect(rng: &mut Rng, size: [u32; 2]) -> Poly {
    let (w, h) = (size[0] as f64, size[1] as f64);
    let (x0, y0, x1, y1) = match rng.below(8) {
        0 => (0.0, 0.0, w, h),
        1 => (-1e6, -1e6, 1e6, 1e6),
        2 => (-100.0, -100.0, -1.0, h + 100.0),
        3 => (w + 1.0, -100.0, w + 100.0, h + 100.0),
        4 => (-100.0, -100.0, w + 100.0, -0.5),
        5 => (-100.0, h, w + 100.0, h + 100.0),
        6 => (-1e300, 0.0, 1e300, 16.0),
        _ => (0.0, -1e300, 1.0, 1e300),
    };
    let mut pts = vec![p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1)];
    if rng.chance(0.5) {
        pts.reverse();
    }
    poly(pts, true)
}

/// A closed run of random cubics, flattened.
fn blob(rng: &mut Rng, size: [u32; 2], tol: f64) -> Vec<Poly> {
    let c = rng.pt(size);
    let r = rng.range(2.0, 120.0);
    let n = 3 + rng.below(8);
    let mut path = Path::default();
    let at = |rng: &mut Rng, k: usize| {
        let a = k as f64 / n as f64 * std::f64::consts::TAU;
        let rr = r * rng.range(0.5, 1.5);
        p(c.x + rr * a.cos(), c.y + rr * a.sin())
    };
    path.move_to(at(rng, 0));
    for k in 1..=n {
        let q = at(rng, k % n);
        let c1 = p(q.x + rng.range(-r, r), q.y + rng.range(-r, r));
        let c2 = p(q.x + rng.range(-r, r), q.y + rng.range(-r, r));
        path.cubic_to(c1, c2, q);
    }
    if rng.chance(0.8) {
        path.close();
    }
    path.flatten(tol)
}

fn shape_path(rng: &mut Rng, w: f64, h: f64) -> Path {
    match rng.below(4) {
        0 => shapes::ellipse(w * 0.5, h * 0.5, w * 0.5, h * 0.5),
        1 => shapes::star(p(w * 0.5, h * 0.5), 5, w * 0.5, w * 0.25, 0.0, 0.0, 0.0),
        2 => shapes::rect(0.0, 0.0, w, h, [rng.range(0.0, 4.0); 4]),
        _ => shapes::polygon(p(w * 0.5, h * 0.5), 3 + rng.below(6) as u32, w * 0.5, 0.0, 0.0),
    }
}

/// A stroked (and maybe trimmed) primitive somewhere on the target.
fn stroked_shape(rng: &mut Rng, size: [u32; 2], tol: f64) -> Vec<Poly> {
    let (w, h) = (rng.range(4.0, 80.0), rng.range(4.0, 80.0));
    let path = shape_path(rng, w, h);
    let pos = rng.pt(size);
    let xf = Xf::trs(pos, p(w * 0.5, h * 0.5), rng.range(0.0, 360.0), rng.range(0.5, 2.0), rng.range(0.5, 2.0));
    let mut polys = path.transform(&xf).flatten(tol);
    if rng.chance(0.5) {
        let end = rng.range(0.0, 1.0);
        let mode = if rng.chance(0.5) { TrimMode::Simultaneous } else { TrimMode::Sequential };
        polys = measure::trim(&polys, 0.0, end, rng.range(0.0, 1.0), mode);
    }
    let style = Style {
        width: rng.range(0.5, 12.0),
        cap: [Cap::Butt, Cap::Round, Cap::Square][rng.below(3)],
        join: [Join::Miter, Join::Round, Join::Bevel][rng.below(3)],
        miter_limit: rng.range(1.0, 10.0),
    };
    stroke::stroke(&polys, &style, tol)
}

/// Polygons for a fill or mask: a mix of every generator, sometimes empty.
fn fill_polys(rng: &mut Rng, size: [u32; 2]) -> Vec<Poly> {
    let tol = [0.05, 0.1, 0.25, 1.0][rng.below(4)];
    match rng.below(20) {
        0 => Vec::new(),
        1 | 2 => vec![tiny(rng, size)],
        3 | 4 => vec![aligned_rect(rng, size)],
        5 => vec![extreme_rect(rng, size)],
        6..=8 => blob(rng, size, tol),
        9..=12 => stroked_shape(rng, size, tol),
        13 => (0..1 + rng.below(30)).map(|_| tiny(rng, size)).collect(),
        14 => {
            let mut v = blob(rng, size, tol);
            v.extend(stroked_shape(rng, size, tol));
            v.push(polyline(rng, size));
            v
        }
        _ => (0..1 + rng.below(4)).map(|_| polyline(rng, size)).collect(),
    }
}

fn gen_mask(rng: &mut Rng, size: [u32; 2]) -> Cmd {
    Cmd::Mask {
        polys: fill_polys(rng, size),
        rule: rng.rule(),
        op: [MaskOp::Add, MaskOp::Subtract, MaskOp::Intersect, MaskOp::Difference, MaskOp::Lighten, MaskOp::Darken]
            [rng.below(6)],
        opacity: rng.range(0.0, 1.0),
        invert: rng.chance(0.3),
    }
}

/// Appends up to `budget` commands: fills, layers (masks, mattes, nesting past
/// the stack depth) and, now and then, a stray command the parser must skip.
fn gen_cmds(rng: &mut Rng, out: &mut Vec<Cmd>, size: [u32; 2], depth: usize, budget: &mut usize) {
    let items = 1 + rng.below(6);
    for _ in 0..items {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        match rng.below(20) {
            0..=11 => out.push(Cmd::Fill {
                polys: fill_polys(rng, size),
                rule: rng.rule(),
                paint: rng.paint(),
                opacity: if rng.chance(0.1) { 1.0 } else { rng.range(0.0, 1.0) },
            }),
            12..=17 if depth < 11 => {
                out.push(Cmd::Push { mask_init: [0.0, 1.0, 0.5][rng.below(3)] });
                gen_cmds(rng, out, size, depth + 1, budget);
                for _ in 0..rng.below(3) {
                    out.push(gen_mask(rng, size));
                }
                match rng.below(10) {
                    0..=5 => out.push(Cmd::Pop { opacity: rng.range(0.0, 1.0) }),
                    6..=8 => {
                        out.push(Cmd::PushMatte);
                        gen_cmds(rng, out, size, depth + 2, budget);
                        if rng.chance(0.9) {
                            let mode =
                                [MatteMode::Alpha, MatteMode::AlphaInverted, MatteMode::Luma, MatteMode::LumaInverted]
                                    [rng.below(4)];
                            out.push(Cmd::PopMatte { mode, opacity: rng.range(0.0, 1.0) });
                        }
                    }
                    _ => {} // a layer left open
                }
            }
            18 => out.push(match rng.below(4) {
                0 => Cmd::Pop { opacity: 0.5 },
                1 => Cmd::PopMatte { mode: MatteMode::Alpha, opacity: 0.5 },
                2 => Cmd::PushMatte,
                _ => gen_mask(rng, size),
            }),
            _ => {}
        }
    }
}

fn random_scene(seed: u64, size: [u32; 2], budget: usize) -> Scene {
    let mut rng = Rng::new(seed);
    let mut cmds = Vec::new();
    let mut budget = budget;
    while budget > 0 {
        gen_cmds(&mut rng, &mut cmds, size, 0, &mut budget);
    }
    Scene { cmds }
}

// ---------------------------------------------------------------------------
// Fixture-shaped scenes
// ---------------------------------------------------------------------------

fn points_of(s: &Scene) -> usize {
    s.cmds
        .iter()
        .map(|c| match c {
            Cmd::Fill { polys, .. } | Cmd::Mask { polys, .. } => polys.iter().map(|q| q.pts.len()).sum(),
            _ => 0,
        })
        .sum()
}

/// perf_vector: 2,000 small stroked primitives (22 px, 3 px round stroke,
/// rotated, trimmed) spread over 1080p, one fill each.
fn perf_vector_like(seed: u64, count: usize) -> Scene {
    let mut rng = Rng::new(seed);
    let tol = 0.05;
    let style = Style { width: 3.0, cap: Cap::Round, join: Join::Round, miter_limit: 4.0 };
    let mut s = Scene::default();
    let cols = 50;
    for k in 0..count {
        let (cx, cy) = (20.0 + (k % cols) as f64 * 38.0, 20.0 + (k / cols) as f64 * 26.0);
        let path = shape_path(&mut rng, 22.0, 22.0);
        let xf = Xf::trs(p(cx, cy), p(11.0, 11.0), rng.range(0.0, 360.0), 1.0, 1.0);
        let polys = path.transform(&xf).flatten(tol);
        let polys = measure::trim(&polys, 0.0, rng.range(0.05, 1.0), 0.0, TrimMode::Simultaneous);
        let polys = stroke::stroke(&polys, &style, tol);
        let paint = Paint::Solid { rgba: [rng.unit(), rng.unit(), rng.unit(), 1.0], srgb: true };
        s.cmds.push(Cmd::Fill { polys, rule: FillRule::NonZero, paint, opacity: 1.0 });
    }
    s
}

/// A glyph-like outline about `size` px tall at the origin.
fn glyph_path(rng: &mut Rng, size: f64) -> Path {
    let h = size;
    match rng.below(6) {
        0 => {
            // an O: outer ellipse with a hole
            let mut o = shapes::ellipse(h * 0.35, h * 0.5, h * 0.35, h * 0.5);
            o.extend(&shapes::ellipse(h * 0.35, h * 0.5, h * 0.2, h * 0.36));
            o
        }
        1 => shapes::rect(0.0, 0.0, h * 0.18, h, [0.0; 4]),
        2 => {
            // a T
            let mut t = shapes::rect(0.0, 0.0, h * 0.7, h * 0.16, [0.0; 4]);
            t.extend(&shapes::rect(h * 0.27, h * 0.16, h * 0.16, h * 0.84, [0.0; 4]));
            t
        }
        3 => {
            // a rounded box with a rounded hole (an o or a d bowl)
            let mut o = shapes::rect(0.0, h * 0.3, h * 0.6, h * 0.7, [h * 0.3; 4]);
            o.extend(&shapes::rect(h * 0.15, h * 0.45, h * 0.3, h * 0.4, [h * 0.15; 4]));
            o
        }
        _ => {
            // an s-like run of cubics
            let mut pth = Path::default();
            let n = 6 + rng.below(4);
            let at = |rng: &mut Rng, k: usize| {
                let a = k as f64 / n as f64 * std::f64::consts::TAU;
                p(h * 0.3 + h * 0.3 * rng.range(0.6, 1.0) * a.cos(), h * 0.5 + h * 0.5 * rng.range(0.6, 1.0) * a.sin())
            };
            pth.move_to(at(rng, 0));
            for k in 1..=n {
                let q = at(rng, k % n);
                pth.cubic_to(p(q.x + rng.range(-3.0, 3.0), q.y + rng.range(-3.0, 3.0)), p(q.x, q.y - 2.0), q);
            }
            pth.close();
            pth
        }
    }
}

/// perf_text: `layers` text lines of `per_layer` glyphs each (30 px bold),
/// every glyph its own fill with a per-character offset, rotation and
/// scale; every third line is mask-revealed like the text node does.
fn perf_text_like(seed: u64, layers: usize, per_layer: usize) -> Scene {
    let mut rng = Rng::new(seed);
    let tol = 0.05;
    let mut s = Scene::default();
    for l in 0..layers {
        let y = 20.0 + l as f64 * 21.0;
        let clip = l % 3 == 2;
        if clip {
            s.cmds.push(Cmd::Push { mask_init: 0.0 });
        }
        for g in 0..per_layer {
            let x = 40.0 + g as f64 * 18.0;
            let path = glyph_path(&mut rng, 22.0);
            let xf = Xf::trs(
                p(x, y + rng.range(-3.0, 3.0)),
                p(6.0, 11.0),
                rng.range(-10.0, 10.0),
                rng.range(0.9, 1.1),
                rng.range(0.9, 1.1),
            );
            let polys = path.transform(&xf).flatten(tol);
            let paint = Paint::Solid { rgba: [rng.unit(), rng.unit(), 1.0, 1.0], srgb: true };
            s.cmds.push(Cmd::Fill { polys, rule: FillRule::NonZero, paint, opacity: rng.range(0.2, 1.0) });
        }
        if clip {
            let polys = shapes::rect(36.0, y - 4.0, per_layer as f64 * 18.0 + 8.0, 30.0, [0.0; 4]).flatten(tol);
            s.cmds.push(Cmd::Mask { polys, rule: FillRule::NonZero, op: MaskOp::Add, opacity: 1.0, invert: false });
            s.cmds.push(Cmd::Pop { opacity: 1.0 });
        }
    }
    s
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

fn first_diff<T: PartialEq>(a: &[T], b: &[T]) -> Option<usize> {
    if a.len() != b.len() {
        return Some(a.len().min(b.len()));
    }
    a.iter().zip(b).position(|(x, y)| x != y)
}

/// Asserts every field of two encodings is identical, floats compared by bits.
fn assert_same(got: &Encoded, want: &Encoded, what: &str) {
    assert_eq!(got.size, want.size, "{what}: size");
    assert_eq!(got.tiles, want.tiles, "{what}: tiles");
    assert_eq!(got.flattened_layers, want.flattened_layers, "{what}: flattened_layers");
    assert_eq!(format!("{:?}", got.paints), format!("{:?}", want.paints), "{what}: paints");
    if let Some(k) = first_diff(&got.ranges, &want.ranges) {
        panic!(
            "{what}: ranges differ at tile {k} (lens {} vs {}): {:?} vs {:?}",
            got.ranges.len(),
            want.ranges.len(),
            got.ranges.get(k),
            want.ranges.get(k)
        );
    }
    let bits = |c: &tile::GpuCmd| {
        (c.kind, c.piece_off, c.piece_count, c.backdrop, c.backdrop_val.to_bits(), c.paint, c.flags, c.param.to_bits())
    };
    let (gc, wc): (Vec<_>, Vec<_>) = (got.cmds.iter().map(bits).collect(), want.cmds.iter().map(bits).collect());
    if let Some(k) = first_diff(&gc, &wc) {
        panic!(
            "{what}: cmds differ at {k} (lens {} vs {}): {:?} vs {:?}",
            gc.len(),
            wc.len(),
            got.cmds.get(k),
            want.cmds.get(k)
        );
    }
    let pb = |v: &[f32; 4]| v.map(f32::to_bits);
    let (gp, wp): (Vec<_>, Vec<_>) = (got.pieces.iter().map(pb).collect(), want.pieces.iter().map(pb).collect());
    if let Some(k) = first_diff(&gp, &wp) {
        panic!(
            "{what}: pieces differ at {k} (lens {} vs {}): {:?} vs {:?}",
            gp.len(),
            wp.len(),
            got.pieces.get(k),
            want.pieces.get(k)
        );
    }
    let (gb, wb): (Vec<_>, Vec<_>) =
        (got.backdrops.iter().map(|v| v.to_bits()).collect(), want.backdrops.iter().map(|v| v.to_bits()).collect());
    if let Some(k) = first_diff(&gb, &wb) {
        panic!(
            "{what}: backdrops differ at {k} (lens {} vs {}): {:?} vs {:?}",
            gb.len(),
            wb.len(),
            got.backdrops.get(k),
            want.backdrops.get(k)
        );
    }
}

fn check(scene: &Scene, size: [u32; 2], what: &str) {
    let want = reference::encode(scene, size);
    let got = tile::encode(scene, size);
    assert_same(&got, &want, &format!("{what} at {}x{}", size[0], size[1]));
}

const SIZES: [[u32; 2]; 7] = [[1920, 1080], [640, 360], [16, 16], [17, 33], [1, 1], [100, 3], [0, 0]];

#[test]
fn encode_matches_reference_on_random_scenes() {
    let mut tiles = 0usize;
    let mut cmds = 0usize;
    for seed in 0..48u64 {
        for (k, &size) in SIZES.iter().enumerate() {
            let scene = random_scene(seed * 1000 + k as u64, size, 40 + (seed as usize % 5) * 30);
            let want = reference::encode(&scene, size);
            let got = tile::encode(&scene, size);
            assert_same(&got, &want, &format!("seed {seed} at {}x{}", size[0], size[1]));
            tiles += want.ranges.iter().filter(|r| r[1] > 0).count();
            cmds += want.cmds.len();
        }
    }
    assert!(tiles > 10_000 && cmds > 50_000, "the generator must exercise the encoder ({tiles} tiles, {cmds} cmds)");
}

#[test]
fn encode_matches_reference_on_degenerate_geometry() {
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| poly(vec![p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1)], true);
    let fill = |polys: Vec<Poly>| Cmd::Fill {
        polys,
        rule: FillRule::NonZero,
        paint: Paint::Solid { rgba: [1.0; 4], srgb: false },
        opacity: 1.0,
    };
    let cases: Vec<(&str, Vec<Cmd>)> = vec![
        ("empty scene", vec![]),
        ("empty polys", vec![fill(vec![])]),
        ("no points", vec![fill(vec![poly(vec![], true)])]),
        ("one point", vec![fill(vec![poly(vec![p(5.0, 5.0)], true)])]),
        ("two points", vec![fill(vec![poly(vec![p(5.0, 5.0), p(40.0, 30.0)], false)])]),
        ("horizontal line", vec![fill(vec![poly(vec![p(5.0, 5.0), p(40.0, 5.0)], false)])]),
        ("vertical line", vec![fill(vec![poly(vec![p(5.0, 5.0), p(5.0, 40.0)], false)])]),
        ("zero area", vec![fill(vec![rect(3.0, 7.0, 30.0, 7.0)])]),
        ("tile aligned", vec![fill(vec![rect(16.0, 16.0, 48.0, 64.0)])]),
        ("pixel aligned", vec![fill(vec![rect(1.0, 2.0, 17.0, 33.0)])]),
        ("whole target", vec![fill(vec![rect(0.0, 0.0, 1920.0, 1080.0)])]),
        ("around target", vec![fill(vec![rect(-10.0, -10.0, 5000.0, 5000.0)])]),
        ("above", vec![fill(vec![rect(0.0, -100.0, 100.0, -1.0)])]),
        ("touching top", vec![fill(vec![rect(0.0, -100.0, 100.0, 0.0)])]),
        ("below", vec![fill(vec![rect(0.0, 1080.0, 100.0, 2000.0)])]),
        ("left", vec![fill(vec![rect(-100.0, 0.0, -1.0, 100.0)])]),
        ("touching left", vec![fill(vec![rect(-100.0, 0.0, 0.0, 100.0)])]),
        ("right", vec![fill(vec![rect(1920.0, 0.0, 3000.0, 100.0)])]),
        ("huge", vec![fill(vec![rect(-1e300, -1e300, 1e300, 1e300)])]),
        ("huge x", vec![fill(vec![poly(vec![p(-1e300, 3.0), p(1e300, 5.0), p(1e300, 30.0)], true)])]),
        ("huge y", vec![fill(vec![poly(vec![p(3.0, -1e300), p(5.0, 1e300), p(30.0, 1e300)], true)])]),
        ("steep", vec![fill(vec![poly(vec![p(3.0, 3.0), p(3.0 + 1e-300, 30.0), p(1e300, 30.0)], true)])]),
        ("nan x", vec![fill(vec![poly(vec![p(f64::NAN, 3.0), p(50.0, 5.0), p(20.0, 30.0)], true)])]),
        ("nan y", vec![fill(vec![poly(vec![p(3.0, f64::NAN), p(50.0, 5.0), p(20.0, 30.0)], true)])]),
        ("all nan", vec![fill(vec![poly(vec![p(f64::NAN, f64::NAN); 3], true)])]),
        ("inf", vec![fill(vec![poly(vec![p(f64::INFINITY, 3.0), p(50.0, f64::NEG_INFINITY), p(20.0, 30.0)], true)])]),
        ("degenerate second edge", vec![fill(vec![poly(vec![p(3.0, 3.0), p(3.0, 3.0), p(30.0, 40.0)], true)])]),
        ("stray pop", vec![Cmd::Pop { opacity: 1.0 }, fill(vec![rect(1.0, 1.0, 40.0, 40.0)])]),
        (
            "stray pop matte",
            vec![Cmd::PopMatte { mode: MatteMode::Luma, opacity: 1.0 }, fill(vec![rect(1.0, 1.0, 40.0, 40.0)])],
        ),
        ("stray mask", vec![gen_mask(&mut Rng::new(1), [64, 64])]),
        ("open layer", vec![Cmd::Push { mask_init: 0.0 }, fill(vec![rect(1.0, 1.0, 40.0, 40.0)])]),
        ("open matte", vec![Cmd::Push { mask_init: 1.0 }, fill(vec![rect(1.0, 1.0, 40.0, 40.0)]), Cmd::PushMatte]),
        ("empty layer", vec![Cmd::Push { mask_init: 1.0 }, Cmd::Pop { opacity: 0.5 }]),
        (
            "matte off layer",
            vec![
                Cmd::Push { mask_init: 1.0 },
                fill(vec![rect(1.0, 1.0, 40.0, 40.0)]),
                fill(vec![rect(50.0, 1.0, 90.0, 40.0)]),
                Cmd::PushMatte,
                fill(vec![rect(200.0, 200.0, 300.0, 300.0)]),
                fill(vec![rect(20.0, 20.0, 60.0, 60.0)]),
                Cmd::PopMatte { mode: MatteMode::AlphaInverted, opacity: 0.7 },
            ],
        ),
        (
            "mask off layer",
            vec![
                Cmd::Push { mask_init: 0.0 },
                fill(vec![rect(1.0, 1.0, 40.0, 40.0)]),
                fill(vec![rect(50.0, 1.0, 90.0, 40.0)]),
                Cmd::Mask {
                    polys: vec![rect(200.0, 200.0, 300.0, 300.0), rect(20.0, 20.0, 60.0, 60.0)],
                    rule: FillRule::EvenOdd,
                    op: MaskOp::Intersect,
                    opacity: 0.5,
                    invert: true,
                },
                Cmd::Pop { opacity: 0.9 },
            ],
        ),
    ];
    for (name, cmds) in cases {
        let scene = Scene { cmds };
        for &size in &SIZES {
            check(&scene, size, name);
        }
    }
    // deeper than the stack: flattened layers
    let mut cmds = Vec::new();
    for d in 0..12 {
        cmds.push(Cmd::Push { mask_init: if d % 2 == 0 { 0.0 } else { 1.0 } });
        cmds.push(fill(vec![rect(d as f64 * 5.0, d as f64 * 5.0, 100.0, 100.0)]));
    }
    for d in 0..12 {
        if d % 3 == 0 {
            cmds.push(Cmd::Mask {
                polys: vec![rect(2.0, 2.0, 60.0, 60.0)],
                rule: FillRule::NonZero,
                op: MaskOp::Add,
                opacity: 1.0,
                invert: false,
            });
        }
        cmds.push(Cmd::Pop { opacity: 0.8 });
    }
    check(&Scene { cmds }, [640, 360], "deep nesting");
}

#[test]
fn encode_matches_reference_on_threaded_scenes() {
    for seed in 0..3u64 {
        let scene = random_scene(7_000 + seed, [1920, 1080], 400);
        assert!(points_of(&scene) >= 20_000, "must take the threaded path ({} points)", points_of(&scene));
        check(&scene, [1920, 1080], &format!("threaded seed {seed}"));
        check(&scene, [640, 360], &format!("threaded seed {seed}"));
    }
}

#[test]
fn encode_matches_reference_on_fixture_shaped_scenes() {
    let v = perf_vector_like(3, 2000);
    check(&v, [1920, 1080], "perf_vector-like");
    check(&v, [640, 360], "perf_vector-like");
    let t = perf_text_like(4, 50, 40);
    check(&t, [1920, 1080], "perf_text-like");
    check(&t, [640, 360], "perf_text-like");
}

// ---------------------------------------------------------------------------
// Micro-benchmarks
// ---------------------------------------------------------------------------

/// (min, median) milliseconds of `f` over `iters` runs, and its last output.
fn time_ms(mut f: impl FnMut() -> Encoded, iters: usize) -> (f64, f64, Encoded) {
    let mut ts = Vec::with_capacity(iters);
    let mut last = None;
    for _ in 0..iters {
        let clock = std::time::Instant::now();
        last = Some(f());
        ts.push(clock.elapsed().as_secs_f64() * 1e3);
    }
    ts.sort_by(f64::total_cmp);
    (ts[0], ts[ts.len() / 2], last.unwrap())
}

fn bench(name: &str, scene: &Scene, size: [u32; 2]) {
    let iters = 40;
    let mut r = (f64::INFINITY, f64::INFINITY);
    let mut n = (f64::INFINITY, f64::INFINITY);
    let (mut re, mut ne) = (None, None);
    // alternate the two so clock and cache state are shared
    for _ in 0..4 {
        let (a, b, e) = time_ms(|| reference::encode(scene, size), iters / 4);
        r = (r.0.min(a), r.1.min(b));
        re = Some(e);
        let (a, b, e) = time_ms(|| tile::encode(scene, size), iters / 4);
        n = (n.0.min(a), n.1.min(b));
        ne = Some(e);
    }
    let (re, ne) = (re.unwrap(), ne.unwrap());
    assert_same(&ne, &re, name);
    println!(
        "{name}: {} cmds, {} pieces, {} points, {} fills | reference min {:.3} ms median {:.3} ms | encode min {:.3} ms median {:.3} ms | speedup x{:.2}",
        re.cmds.len(),
        re.pieces.len(),
        points_of(scene),
        scene.cmds.iter().filter(|c| matches!(c, Cmd::Fill { .. })).count(),
        r.0,
        r.1,
        n.0,
        n.1,
        r.1 / n.1
    );
}

#[test]
#[ignore]
fn bench_perf_vector_like() {
    bench("perf_vector-like 1080p", &perf_vector_like(3, 2000), [1920, 1080]);
}

#[test]
#[ignore]
fn bench_perf_text_like() {
    bench("perf_text-like 1080p", &perf_text_like(4, 50, 40), [1920, 1080]);
}

#[test]
#[ignore]
fn bench_random_scene() {
    bench("random 1080p", &random_scene(11, [1920, 1080], 600), [1920, 1080]);
}

/// Under the encoder's 20,000-point threshold: the single-threaded path,
/// which measures the per-path work without thread start-up.
#[test]
#[ignore]
fn bench_sequential() {
    let v = perf_vector_like(3, 300);
    assert!(points_of(&v) < 20_000, "{} points", points_of(&v));
    bench("perf_vector-like 300 shapes, one thread", &v, [1920, 1080]);
    let t = perf_text_like(4, 8, 40);
    assert!(points_of(&t) < 20_000, "{} points", points_of(&t));
    bench("perf_text-like 8 lines, one thread", &t, [1920, 1080]);
}
