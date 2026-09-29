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
//!
//! The encoder is allocation-light: every path's tiles, pieces and
//! backdrops land in one arena per thread (`TileArena`), the per-row
//! scratch is reused from path to path, and the per-tile command lists are
//! one flat list in emission order that a counting pass buckets by tile.
//! `tests/tile_equivalence.rs` holds the original encoder and checks that
//! this one reproduces its output bit for bit.

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

/// One tile of a path within a `TileArena`.
#[derive(Clone, Copy)]
struct TileRef {
    tile: u32,
    piece_off: u32,
    piece_count: u32,
    /// Index of the per-row backdrop in `TileArena::rows`, or `u32::MAX` for the uniform `backdrop_val`.
    backdrop: u32,
    backdrop_val: f32,
}

/// The tiles of every path of a scene (or of one thread's share of it).
#[derive(Default)]
struct TileArena {
    tiles: Vec<TileRef>,
    /// Tile-local pieces, referenced by `TileRef::piece_off`.
    pieces: Vec<[f32; 4]>,
    /// Per-row backdrops, 16 values each, referenced by `TileRef::backdrop`.
    rows: Vec<f32>,
}

/// A path's tiles: `arena.tiles[start..end]`, in tile order.
#[derive(Clone, Copy)]
struct PathRange {
    start: u32,
    end: u32,
}

/// Consecutive arena tiles `first..=last` used by consecutive commands, and
/// the backdrop offsets of the first and last of them that have rows.
#[derive(Clone, Copy)]
struct Run {
    first: u32,
    last: u32,
    rows: Option<(u32, u32)>,
}

impl TileArena {
    /// Appends another arena, relocating its references; `pre` gets `part` relocated the same way.
    fn append(&mut self, pre: &mut Vec<Option<PathRange>>, part: Vec<Option<PathRange>>, mut a: TileArena) {
        if self.tiles.is_empty() && self.pieces.is_empty() && self.rows.is_empty() {
            pre.extend(part);
            *self = a;
            return;
        }
        let (dt, dp, dr) = (self.tiles.len() as u32, self.pieces.len() as u32, self.rows.len() as u32);
        pre.extend(part.into_iter().map(|r| r.map(|r| PathRange { start: r.start + dt, end: r.end + dt })));
        self.tiles.extend(a.tiles.iter().map(|t| TileRef {
            piece_off: t.piece_off + dp,
            backdrop: if t.backdrop == u32::MAX { u32::MAX } else { t.backdrop + dr },
            ..*t
        }));
        self.pieces.append(&mut a.pieces);
        self.rows.append(&mut a.rows);
    }
}

/// A piece of a row in the sweep: its x extent and the piece itself.
#[derive(Clone, Copy)]
struct Cut {
    lo: f64,
    hi: f64,
    pc: [f64; 4],
}

/// Per-thread scratch of `tile_path`, reused from path to path.
#[derive(Default)]
struct Scratch {
    /// The path's pieces per tile row, in edge order.
    rows: Vec<Vec<[f64; 4]>>,
    /// (sort key of the left x, index) of the current row's pieces.
    order: Vec<(u64, u32)>,
    /// The current row's pieces in left-x order.
    sorted: Vec<Cut>,
    /// Indices into `sorted` of the pieces overlapping the current column.
    active: Vec<u32>,
}

/// `f64::total_cmp` as an unsigned key: `key(a) < key(b)` iff `a.total_cmp(&b)` is `Less`.
fn sort_key(x: f64) -> u64 {
    let b = x.to_bits() as i64;
    let k = b ^ (((b >> 63) as u64) >> 1) as i64;
    (k as u64) ^ (1 << 63)
}

/// Sorts (key, index) pairs, which never tie, by moving each into place:
/// a row's pieces come in a few monotone runs, so few moves are needed.
#[inline]
fn insertion_sort(v: &mut [(u64, u32)]) {
    for i in 1..v.len() {
        let x = v[i];
        let mut j = i;
        while j > 0 && v[j - 1] > x {
            v[j] = v[j - 1];
            j -= 1;
        }
        v[j] = x;
    }
}

/// `x.floor() as i64` without the libm call (the baseline x86-64 target has
/// no `roundsd`): `as` truncates towards zero, saturates and maps NaN to 0
/// exactly as the cast of the floored value does, so only a truncation that
/// moved up needs one step down. `tests::floor_ceil_match_libm` proves it.
#[inline]
fn floor_i64(x: f64) -> i64 {
    let i = x as i64;
    if (i as f64) > x {
        i.saturating_sub(1)
    } else {
        i
    }
}

/// `x.ceil() as i64`, see `floor_i64`.
#[inline]
fn ceil_i64(x: f64) -> i64 {
    let i = x as i64;
    if (i as f64) < x {
        i.saturating_add(1)
    } else {
        i
    }
}

/// Tiles a path into `out`: the tiles it touches with their pieces and backdrops.
fn tile_path(polys: &[Poly], size: [u32; 2], s: &mut Scratch, out: &mut TileArena) -> PathRange {
    let start = out.tiles.len() as u32;
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
        return PathRange { start, end: start };
    }
    let row0 = floor_i64(ymin / t).max(0);
    let row1 = (ceil_i64(ymax / t) - 1).min(ty - 1);
    let nrows = (row1 - row0 + 1).max(0) as usize;
    let Scratch { rows, order, sorted, active } = s;
    if rows.len() < nrows {
        rows.resize_with(nrows, Vec::new);
    }
    let rows = &mut rows[..nrows];
    for r in rows.iter_mut() {
        r.clear();
    }
    let height = size[1] as f64;
    for q in polys {
        let n = q.pts.len();
        if n < 2 {
            continue;
        }
        // every edge, the closing one last: fills close every subpath
        let pts = &q.pts[..n];
        for k in 0..n {
            let a = pts[k];
            let b = pts[if k + 1 < n { k + 1 } else { 0 }];
            if a.y == b.y || !(a.x.is_finite() && a.y.is_finite() && b.x.is_finite() && b.y.is_finite()) {
                continue;
            }
            let (lo, hi) = (a.y.min(b.y), a.y.max(b.y));
            if hi <= 0.0 || lo >= height {
                continue;
            }
            let r0 = floor_i64(lo / t).max(row0);
            let r1 = (ceil_i64(hi / t) - 1).min(row1);
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
    for (ri, pieces) in rows.iter().enumerate() {
        if pieces.is_empty() {
            continue;
        }
        let r = row0 + ri as i64;
        // stable sort by xmin: sorting (key, index) pairs, which never tie, gives the same order
        order.clear();
        order.extend(pieces.iter().enumerate().map(|(i, pc)| (sort_key(xmin(pc)), i as u32)));
        if order.len() <= 32 {
            insertion_sort(order);
        } else {
            order.sort_unstable();
        }
        sorted.clear();
        sorted.extend(order.iter().map(|&(_, i)| {
            let pc = pieces[i as usize];
            Cut { lo: xmin(&pc), hi: xmax(&pc), pc }
        }));
        let hi = sorted.iter().map(|c| c.hi).fold(f64::NEG_INFINITY, f64::max);
        let c0 = floor_i64(sorted[0].lo / t).max(0);
        let c1 = floor_i64(hi / t).min(tx - 1);
        let mut acc = [0.0f64; 16];
        // pieces entering the sweep in xmin order; active ones overlap the current column
        let mut next = 0;
        active.clear();
        let add = |acc: &mut [f64; 16], pc: &[f64; 4]| {
            let (y0, y1) = (pc[1].min(pc[3]), pc[1].max(pc[3]));
            let (p0, p1) = (floor_i64(y0).max(0) as usize, (ceil_i64(y1).max(0) as usize).min(16));
            for (py, a) in acc.iter_mut().enumerate().take(p1).skip(p0) {
                let (fy0, fy1) = (py as f64, py as f64 + 1.0);
                *a += pc[3].clamp(fy0, fy1) - pc[1].clamp(fy0, fy1);
            }
        };
        // pieces wholly left of the first column only feed the backdrop
        while next < sorted.len() && sorted[next].hi <= c0 as f64 * t && sorted[next].lo < c0 as f64 * t {
            add(&mut acc, &sorted[next].pc);
            next += 1;
        }
        for c in c0..=c1 {
            let x0 = c as f64 * t;
            while next < sorted.len() && sorted[next].lo < x0 + t {
                active.push(next as u32);
                next += 1;
            }
            // pieces that ended left of this column move into the backdrop
            let mut kept = 0;
            for i in 0..active.len() {
                let k = active[i];
                if sorted[k as usize].hi <= x0 {
                    add(&mut acc, &sorted[k as usize].pc);
                } else {
                    active[kept] = k;
                    kept += 1;
                }
            }
            active.truncate(kept);
            if active.is_empty() && !acc.iter().any(|v| v.abs() > 1e-9) {
                continue;
            }
            let piece_off = out.pieces.len() as u32;
            out.pieces.extend(active.iter().map(|&k| {
                let pc = sorted[k as usize].pc;
                [(pc[0] - x0) as f32, pc[1] as f32, (pc[2] - x0) as f32, pc[3] as f32]
            }));
            let uniform = acc.iter().all(|v| (v - acc[0]).abs() < 1e-9);
            let (backdrop, backdrop_val) = if uniform {
                (u32::MAX, acc[0] as f32)
            } else {
                let off = out.rows.len() as u32;
                out.rows.extend_from_slice(&acc.map(|v| v as f32));
                (off, 0.0)
            };
            out.tiles.push(TileRef {
                tile: (r * tx + c) as u32,
                piece_off,
                piece_count: active.len() as u32,
                backdrop,
                backdrop_val,
            });
        }
    }
    // rows ascend and columns ascend within a row, so the tiles are already in tile order
    PathRange { start, end: out.tiles.len() as u32 }
}

enum Node {
    Fill {
        tiles: PathRange,
        rule: FillRule,
        paint: u32,
        opacity: f64,
    },
    Group {
        children: Vec<Node>,
        masks: Vec<(PathRange, FillRule, MaskOp, f64, bool)>,
        mask_init: f64,
        opacity: f64,
        matte: Option<(Vec<Node>, MatteMode)>,
    },
}

struct Enc {
    size: [u32; 2],
    /// Tiles of every Fill and Mask command, computed up front in parallel.
    pre: Vec<Option<PathRange>>,
    arena: TileArena,
    scratch: Scratch,
    /// Commands in emission order, with the tile of each and the arena tile
    /// whose pieces it uses (`u32::MAX` for none).
    flat: Vec<GpuCmd>,
    flat_tile: Vec<u32>,
    flat_src: Vec<u32>,
    /// Pieces and backdrop values claimed so far; the output offsets.
    piece_len: u32,
    backdrop_len: u32,
    /// Whether the arena tiles used so far are exactly `0..next_src`, in that
    /// order: then the output pieces and backdrops are the arena's own.
    in_order: bool,
    next_src: u32,
    paints: Vec<Paint>,
    flattened: usize,
}

impl Enc {
    fn take_tiles(&mut self, i: usize, polys: &[Poly]) -> PathRange {
        match self.pre.get_mut(i).and_then(Option::take) {
            Some(r) => r,
            None => tile_path(polys, self.size, &mut self.scratch, &mut self.arena),
        }
    }

    /// A path command over arena tile `src` (if any), claiming its pieces and backdrop.
    fn path_cmd(&mut self, kind: u32, src: Option<u32>, flags: u32, paint: u32, param: f64) -> GpuCmd {
        let mut c = GpuCmd { kind, paint, flags, param: param as f32, backdrop: u32::MAX, ..Default::default() };
        if let Some(k) = src {
            let pt = self.arena.tiles[k as usize];
            c.piece_off = self.piece_len;
            c.piece_count = pt.piece_count;
            self.piece_len += pt.piece_count;
            if pt.backdrop == u32::MAX {
                c.backdrop_val = pt.backdrop_val;
            } else {
                c.backdrop = self.backdrop_len;
                self.backdrop_len += 16;
            }
            if k == self.next_src {
                self.next_src += 1;
            } else {
                self.in_order = false;
            }
        }
        c
    }

    fn push(&mut self, tile: u32, src: Option<u32>, c: GpuCmd) {
        self.flat.push(c);
        self.flat_tile.push(tile);
        self.flat_src.push(src.unwrap_or(u32::MAX));
    }

    /// The output pieces and backdrops: the arena's own buffers when the
    /// commands used its tiles in order, otherwise gathered in command order.
    fn materialise(&mut self) -> (Vec<[f32; 4]>, Vec<f32>) {
        if self.in_order {
            let mut pieces = std::mem::take(&mut self.arena.pieces);
            let mut backdrops = std::mem::take(&mut self.arena.rows);
            pieces.truncate(self.piece_len as usize);
            backdrops.truncate(self.backdrop_len as usize);
            return (pieces, backdrops);
        }
        // consecutive arena tiles are contiguous in the arena: copy them as one run
        let mut pieces = Vec::with_capacity(self.piece_len as usize);
        let mut backdrops = Vec::with_capacity(self.backdrop_len as usize);
        let arena = &self.arena;
        let mut run: Option<Run> = None;
        let flush = |run: &Option<Run>, pieces: &mut Vec<[f32; 4]>, backdrops: &mut Vec<f32>| {
            if let Some(r) = run {
                let (a, b) = (arena.tiles[r.first as usize], arena.tiles[r.last as usize]);
                pieces.extend_from_slice(&arena.pieces[a.piece_off as usize..(b.piece_off + b.piece_count) as usize]);
                if let Some((b0, b1)) = r.rows {
                    backdrops.extend_from_slice(&arena.rows[b0 as usize..b1 as usize + 16]);
                }
            }
        };
        for &k in &self.flat_src {
            if k == u32::MAX {
                continue;
            }
            let bd = arena.tiles[k as usize].backdrop;
            match &mut run {
                Some(r) if r.last + 1 == k => {
                    r.last = k;
                    if bd != u32::MAX {
                        r.rows = Some((r.rows.map_or(bd, |(b0, _)| b0), bd));
                    }
                }
                _ => {
                    flush(&run, &mut pieces, &mut backdrops);
                    run = Some(Run { first: k, last: k, rows: (bd != u32::MAX).then_some((bd, bd)) });
                }
            }
        }
        flush(&run, &mut pieces, &mut backdrops);
        (pieces, backdrops)
    }

    fn parse(&mut self, cmds: &[Cmd], i: &mut usize, until_matte: bool) -> Vec<Node> {
        let mut out = Vec::new();
        while *i < cmds.len() {
            match &cmds[*i] {
                Cmd::Fill { polys, rule, paint, opacity } => {
                    let tiles = self.take_tiles(*i, polys);
                    *i += 1;
                    if tiles.start == tiles.end {
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
                                let tiles = self.take_tiles(*i, polys);
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

    /// The tiles of every fill under `nodes`, ascending and unique.
    fn tiles_of(&self, nodes: &[Node]) -> Vec<u32> {
        fn collect(arena: &TileArena, nodes: &[Node], v: &mut Vec<u32>) {
            for n in nodes {
                match n {
                    Node::Fill { tiles, .. } => {
                        v.extend(arena.tiles[tiles.start as usize..tiles.end as usize].iter().map(|t| t.tile))
                    }
                    Node::Group { children, .. } => collect(arena, children, v),
                }
            }
        }
        let mut v = Vec::new();
        collect(&self.arena, nodes, &mut v);
        v.sort_unstable();
        v.dedup();
        v
    }

    fn emit(&mut self, nodes: &[Node], depth: usize, opacity_mul: f64) {
        for n in nodes {
            match n {
                Node::Fill { tiles, rule, paint, opacity } => {
                    let flags = (*rule == FillRule::EvenOdd) as u32;
                    for k in tiles.start..tiles.end {
                        let tile = self.arena.tiles[k as usize].tile;
                        let c = self.path_cmd(kind::FILL, Some(k), flags, *paint, opacity * opacity_mul);
                        self.push(tile, Some(k), c);
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
                    let tiles = self.tiles_of(children);
                    for &t in &tiles {
                        let c = GpuCmd { kind: kind::PUSH, param: *mask_init as f32, ..Default::default() };
                        self.push(t, None, c);
                    }
                    self.emit(children, depth + 1, 1.0);
                    for (mt, rule, op, mop, inv) in masks {
                        let flags = (*rule == FillRule::EvenOdd) as u32 | ((*inv as u32) << 1) | ((*op as u32) << 4);
                        // both lists ascend: walk the mask's tiles alongside the layer's
                        let (ms, me) = (mt.start as usize, mt.end as usize);
                        let mut j = ms;
                        for &t in &tiles {
                            while j < me && self.arena.tiles[j].tile < t {
                                j += 1;
                            }
                            let src = if j < me && self.arena.tiles[j].tile == t { Some(j as u32) } else { None };
                            let c = self.path_cmd(kind::MASK, src, flags, 0, *mop);
                            self.push(t, src, c);
                        }
                    }
                    match matte {
                        Some((m, mode)) => {
                            for &t in &tiles {
                                let c = GpuCmd { kind: kind::PUSH, param: 1.0, ..Default::default() };
                                self.push(t, None, c);
                            }
                            self.emit(m, depth + 2, 1.0);
                            for &t in &tiles {
                                let c = GpuCmd {
                                    kind: kind::POP_MATTE,
                                    flags: (*mode as u32) << 4,
                                    param: (*opacity * opacity_mul) as f32,
                                    ..Default::default()
                                };
                                self.push(t, None, c);
                            }
                        }
                        None => {
                            for &t in &tiles {
                                let c = GpuCmd {
                                    kind: kind::POP,
                                    param: (*opacity * opacity_mul) as f32,
                                    ..Default::default()
                                };
                                self.push(t, None, c);
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
    let ntiles = (tiles[0] * tiles[1]) as usize;
    let (pre, arena) = tile_all(scene, size);
    let paths = arena.tiles.len();
    let mut e = Enc {
        size,
        pre,
        arena,
        scratch: Scratch::default(),
        flat: Vec::with_capacity(paths),
        flat_tile: Vec::with_capacity(paths),
        flat_src: Vec::with_capacity(paths),
        piece_len: 0,
        backdrop_len: 0,
        in_order: true,
        next_src: 0,
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
    // bucket the commands by tile, keeping emission order within a tile
    let mut ranges = vec![[0u32; 2]; ntiles];
    let mut starts = vec![0u32; ntiles + 1];
    for &t in &e.flat_tile {
        starts[t as usize + 1] += 1;
    }
    for k in 0..ntiles {
        starts[k + 1] += starts[k];
    }
    let mut fill = starts.clone();
    let mut by_tile = vec![GpuCmd::default(); e.flat.len()];
    for (&t, c) in e.flat_tile.iter().zip(&e.flat) {
        let k = &mut fill[t as usize];
        by_tile[*k as usize] = *c;
        *k += 1;
    }
    let cmds = if e.flat.iter().all(|c| c.kind == kind::FILL || c.kind == kind::MASK) {
        // no layers anywhere: `balanced` would pass every list through
        for t in 0..ntiles {
            let (s0, s1) = (starts[t], starts[t + 1]);
            if s0 != s1 {
                ranges[t] = [s0, s1 - s0];
            }
        }
        by_tile
    } else {
        let mut cmds = Vec::with_capacity(e.flat.len());
        for t in 0..ntiles {
            let (s0, s1) = (starts[t] as usize, starts[t + 1] as usize);
            if s0 == s1 {
                continue;
            }
            let off = cmds.len();
            balanced(&by_tile[s0..s1], &mut cmds);
            if cmds.len() > off {
                ranges[t] = [off as u32, (cmds.len() - off) as u32];
            }
        }
        cmds
    };
    let (pieces, backdrops) = e.materialise();
    Encoded { size, tiles, ranges, cmds, pieces, backdrops, paints: e.paints, flattened_layers: e.flattened }
}

/// Tiles every path of the scene, spread over the available cores.
fn tile_all(scene: &Scene, size: [u32; 2]) -> (Vec<Option<PathRange>>, TileArena) {
    let polys: Vec<Option<&[Poly]>> = scene
        .cmds
        .iter()
        .map(|c| match c {
            Cmd::Fill { polys, .. } | Cmd::Mask { polys, .. } => Some(polys.as_slice()),
            _ => None,
        })
        .collect();
    let tile_part = |part: &[Option<&[Poly]>]| {
        let mut scratch = Scratch::default();
        // sized for flattened outlines: about one piece per point, a tile per dozen
        let points: usize = part.iter().flatten().map(|p| p.iter().map(|q| q.pts.len()).sum::<usize>()).sum();
        let mut arena = TileArena {
            tiles: Vec::with_capacity(points / 8),
            pieces: Vec::with_capacity(points + points / 4),
            rows: Vec::with_capacity(points),
        };
        let pre: Vec<Option<PathRange>> =
            part.iter().map(|p| p.map(|p| tile_path(p, size, &mut scratch, &mut arena))).collect();
        (pre, arena)
    };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(16);
    let work: usize = polys.iter().flatten().map(|p| p.iter().map(|q| q.pts.len()).sum::<usize>()).sum();
    if threads <= 1 || work < 20_000 {
        return tile_part(&polys);
    }
    let chunk = polys.len().div_ceil(threads);
    let mut pre: Vec<Option<PathRange>> = Vec::with_capacity(polys.len());
    let mut arena = TileArena::default();
    std::thread::scope(|sc| {
        let handles: Vec<_> = polys.chunks(chunk).map(|part| sc.spawn(move || tile_part(part))).collect();
        for h in handles {
            let (part, a) = h.join().unwrap_or_default();
            arena.append(&mut pre, part, a);
        }
    });
    (pre, arena)
}

/// Appends `list` to `out` without the pushes and pops that cannot pair within one tile
/// (matte content in tiles its layer misses).
fn balanced(list: &[GpuCmd], out: &mut Vec<GpuCmd>) {
    let mut depth = 0i32;
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

#[cfg(test)]
mod tests {
    use super::{ceil_i64, floor_i64};

    /// `floor_i64` and `ceil_i64` agree with `floor() as i64` and `ceil() as i64`
    /// at every awkward value: integers, halves, ±0, subnormals, the 2^52, 2^53
    /// and 2^63 neighbourhoods, ±inf, NaN, and a million random bit patterns.
    #[test]
    fn floor_ceil_match_libm() {
        let mut xs = vec![0.0, -0.0, 0.5, -0.5, 1.5, -1.5, 1e-300, -1e-300, f64::MIN_POSITIVE, -f64::MIN_POSITIVE];
        xs.extend([f64::INFINITY, f64::NEG_INFINITY, f64::NAN, -f64::NAN, f64::MAX, f64::MIN, f64::EPSILON]);
        for e in [51, 52, 53, 62, 63, 64] {
            let b = 2f64.powi(e);
            for d in [-2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0] {
                xs.push(b + d);
                xs.push(-(b + d));
                xs.push((b + d).next_up());
                xs.push((b + d).next_down());
                xs.push(-(b + d).next_up());
                xs.push(-(b + d).next_down());
            }
        }
        for k in -70..70 {
            xs.push(k as f64 / 16.0);
            xs.push(k as f64 / 16.0 + 1e-9);
            xs.push(k as f64 / 16.0 - 1e-9);
        }
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..1_000_000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            xs.push(f64::from_bits(s));
            xs.push(f64::from_bits(s) / 16.0);
            xs.push((s >> 40) as f64 / 16.0 - 2.0e6);
        }
        for &x in &xs {
            assert_eq!(floor_i64(x), x.floor() as i64, "floor {x:e}");
            assert_eq!(ceil_i64(x), x.ceil() as i64, "ceil {x:e}");
        }
    }
}
