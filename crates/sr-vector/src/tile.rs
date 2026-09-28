//! The tile encoder: a scene in target pixels becomes per-tile command
//! lists for the fine rasteriser, the architecture of Vello's coarse
//! stage run on the CPU.
//!
//! Every polygon edge is cut at the 16-pixel tile rows. Within a row, a
//! tile receives the pieces that overlap it horizontally, and a backdrop:
//! for each of its 16 pixel rows, the signed height of the pieces lying
//! entirely to its left. The fine stage then computes exact area coverage
//! per pixel: backdrop plus, for each piece, the piece's height within the
//! pixel row times the fraction of the pixel to the piece's right. No edge
//! is clipped horizontally, so there are no seams between tiles.

use crate::path::Poly;
use crate::scene::{Cmd, FillRule, MaskOp, MatteMode, Paint, Scene};

/// Tile edge in pixels.
pub const TILE: u32 = 16;
/// Layer stack depth of the fine stage.
pub const MAX_DEPTH: usize = 8;

/// Command kinds.
pub mod kind {
    pub const FILL: u32 = 0;
    pub const PUSH: u32 = 1;
    pub const MASK: u32 = 2;
    pub const POP: u32 = 3;
    pub const POP_MATTE: u32 = 4;
}

/// A fine-stage command (8 words, matching `raster.wgsl`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GpuCmd {
    pub kind: u32,
    pub piece_off: u32,
    pub piece_count: u32,
    /// Offset of 16 per-row backdrop values, or `u32::MAX` for the uniform `backdrop_val`.
    pub backdrop: u32,
    pub backdrop_val: f32,
    pub paint: u32,
    /// Bit 0 even-odd, bit 1 invert, bits 4–7 mask op or matte mode.
    pub flags: u32,
    /// Opacity, or the initial mask value of a push.
    pub param: f32,
}

/// Encoder output.
#[derive(Debug, Clone, Default)]
pub struct Encoded {
    pub size: [u32; 2],
    pub tiles: [u32; 2],
    /// Per tile (row-major): first command and count.
    pub ranges: Vec<[u32; 2]>,
    pub cmds: Vec<GpuCmd>,
    /// Pieces (x0, y0, x1, y1) in tile-local pixels.
    pub pieces: Vec<[f32; 4]>,
    pub backdrops: Vec<f32>,
    /// Paints referenced by `GpuCmd::paint`.
    pub paints: Vec<Paint>,
    /// Layers flattened because they exceeded the stack depth.
    pub flattened_layers: usize,
}

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
                rows[(r - row0) as usize].push([a.x + (ya - a.y) * inv, ya - top, a.x + (yb - a.y) * inv, yb - top]);
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
                        let flags = (*rule == FillRule::EvenOdd) as u32 | ((*inv as u32) << 1) | ((*op as u32) << 4);
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

/// Exact coverage contribution of a piece to pixel (px, py), tile-local.
pub fn piece_coverage(px: f32, py: f32, pc: [f32; 4]) -> f32 {
    let [x0, y0, x1, y1] = pc;
    let ya = (y0 - py).clamp(0.0, 1.0);
    let yb = (y1 - py).clamp(0.0, 1.0);
    let dy = yb - ya;
    if dy == 0.0 {
        return 0.0;
    }
    let inv = 1.0 / (y1 - y0);
    let xa = x0 + (x1 - x0) * ((ya + py - y0) * inv) - px;
    let xb = x0 + (x1 - x0) * ((yb + py - y0) * inv) - px;
    let g = |x: f32| {
        if x <= 0.0 {
            0.0
        } else if x < 1.0 {
            x * x * 0.5
        } else {
            x - 0.5
        }
    };
    let m = if (xb - xa).abs() < 1e-4 { ((xa + xb) * 0.5).clamp(0.0, 1.0) } else { (g(xb) - g(xa)) / (xb - xa) };
    dy * (1.0 - m)
}

/// Area coverage of a command's path at a pixel.
pub fn coverage(e: &Encoded, c: &GpuCmd, px: u32, py: u32) -> f32 {
    let mut area = if c.backdrop == u32::MAX { c.backdrop_val } else { e.backdrops[(c.backdrop + py) as usize] };
    for k in 0..c.piece_count {
        area += piece_coverage(px as f32, py as f32, e.pieces[(c.piece_off + k) as usize]);
    }
    let a = area.abs();
    let cov = if c.flags & 1 == 1 { (a - 2.0 * (a * 0.5).round()).abs() } else { a.min(1.0) };
    if c.flags & 2 == 2 {
        1.0 - cov
    } else {
        cov
    }
}

fn mask_combine(m: f32, v: f32, op: u32) -> f32 {
    match op {
        1 => m * (1.0 - v),
        2 => m * v,
        3 => m + v - 2.0 * m * v,
        4 => m.max(v),
        5 => m.min(v),
        _ => m + v - m * v,
    }
}

/// CPU reference of the fine stage (premultiplied RGBA, row-major), with
/// `paint(index, x, y)` returning a straight colour.
pub fn render_cpu(e: &Encoded, paint: &dyn Fn(u32, f32, f32) -> [f32; 4]) -> Vec<[f32; 4]> {
    let [w, h] = e.size;
    let mut out = vec![[0.0f32; 4]; (w * h) as usize];
    for ty in 0..e.tiles[1] {
        for tx in 0..e.tiles[0] {
            let [off, n] = e.ranges[(ty * e.tiles[0] + tx) as usize];
            if n == 0 {
                continue;
            }
            for py in 0..TILE {
                for px in 0..TILE {
                    let (x, y) = (tx * TILE + px, ty * TILE + py);
                    if x >= w || y >= h {
                        continue;
                    }
                    let mut acc = [0.0f32; 4];
                    let mut m = 1.0f32;
                    let mut stack: Vec<([f32; 4], f32)> = Vec::new();
                    for c in &e.cmds[off as usize..(off + n) as usize] {
                        match c.kind {
                            kind::FILL => {
                                let cov = coverage(e, c, px, py) * c.param;
                                if cov > 0.0 {
                                    let s = paint(c.paint, x as f32 + 0.5, y as f32 + 0.5);
                                    let a = s[3] * cov;
                                    let src = [s[0] * a, s[1] * a, s[2] * a, a];
                                    for k in 0..4 {
                                        acc[k] = src[k] + acc[k] * (1.0 - a);
                                    }
                                }
                            }
                            kind::PUSH => {
                                stack.push((acc, m));
                                acc = [0.0; 4];
                                m = c.param;
                            }
                            kind::MASK => {
                                let v = coverage(e, c, px, py) * c.param;
                                m = mask_combine(m, v, (c.flags >> 4) & 15);
                            }
                            kind::POP => {
                                let layer = acc.map(|v| v * m * c.param);
                                let (pa, pm) = stack.pop().unwrap_or(([0.0; 4], 1.0));
                                acc = [0, 1, 2, 3].map(|k| layer[k] + pa[k] * (1.0 - layer[3]));
                                m = pm;
                            }
                            kind::POP_MATTE => {
                                let mt = acc;
                                let luma = 0.2126 * mt[0] + 0.7152 * mt[1] + 0.0722 * mt[2];
                                let mv = match (c.flags >> 4) & 15 {
                                    1 => 1.0 - mt[3],
                                    2 => luma,
                                    3 => 1.0 - luma,
                                    _ => mt[3],
                                };
                                let (content, cm) = stack.pop().unwrap_or(([0.0; 4], 1.0));
                                let layer = content.map(|v| v * cm * mv * c.param);
                                let (pa, pm) = stack.pop().unwrap_or(([0.0; 4], 1.0));
                                acc = [0, 1, 2, 3].map(|k| layer[k] + pa[k] * (1.0 - layer[3]));
                                m = pm;
                            }
                            _ => {}
                        }
                    }
                    out[(y * w + x) as usize] = acc;
                }
            }
        }
    }
    out
}
