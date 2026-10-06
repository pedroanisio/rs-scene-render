//! The points a repeat places its copies on (SREP 26, `repeat/points`).
//!
//! A `points` child generates n points in the repeat's own space (pixels, +y down); copy i is placed on
//! point i, before the repeat's step offsets. The five types:
//!
//! * `grid`: columns × rows, centred on the origin;
//! * `along-path`: count points at equal drawn length along a path measured exactly as a motion path is
//!   (the same segments, arcs as cubics of at most 90°, and the same arc-length table of 32 samples per segment on
//!   the true curve), with jumps between subpaths taking no length;
//! * `scatter`: count points drawn with U(s, c, k) (the seeded lattice hash of noise and random draws, read as a unit
//!   number) inside a width × height rectangle, or by rejection inside a path region (at most 64 candidates per
//!   point), the region outlined by the same samples;
//! * `vertices`: the end point of every drawing command of a path;
//! * `list`: the pairs of `@at`.
//!
//! Every coordinate is computed in binary64 in the order the SREP writes it, so the numeric tests of the
//! SREP hold to the last digit. Points are a function of the document and the time alone.

use crate::path::{arc_table, commands, locate, segments_of, PathCmd, PathError, Seg, P, SAMPLES};

/// Candidates tried per requested point by a path scatter.
pub const CANDIDATES_PER_POINT: u64 = 64;

/// The lattice hash splitmix64(s ⊕ splitmix64(c ⊕ splitmix64(k))) that every seeded draw uses.
pub fn hash(seed: u64, channel: u64, k: u64) -> u64 {
    sr_vector::d24::d24_hash(seed, channel, k)
}

/// U(s, c, k) = ⌊hash / 2¹¹⌋ / 2⁵³: a number in [0, 1).
pub fn unit(seed: u64, channel: u64, k: u64) -> f64 {
    sr_vector::d24::d24_unit(seed, channel, k)
}

/// `pointU` of point i of n: i / (n − 1), and 0 when n = 1.
pub fn point_u(i: u64, n: u64) -> f64 {
    if n <= 1 {
        0.0
    } else {
        i as f64 / (n - 1) as f64
    }
}

/// One generated point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// Position in the repeat's space.
    pub x: f64,
    /// Position in the repeat's space.
    pub y: f64,
    /// θ: the rotation the copy is placed with, degrees clockwise (the direction under `orient="true"`).
    pub theta: f64,
    /// The direction of travel along the path (`pointAngle`): 0 for every type but `along-path`.
    pub direction: f64,
}

impl Point {
    fn at(p: P) -> Point {
        Point { x: p[0], y: p[1], theta: 0.0, direction: 0.0 }
    }
}

/// Path data parsed into its drawing commands and into the segments motion paths measure: lines as they are,
/// quadratics raised to cubics, arcs as cubics of at most 90°, a closepath back to a different point as a line. A
/// subpath without segments (a lone moveto) is not part of the path.
#[derive(Debug, Clone, PartialEq)]
pub struct FlatPath {
    cmds: Vec<PathCmd>,
    segs: Vec<Seg>,
    /// Subpaths with segments: (first segment, end segment, closed by a closepath).
    subs: Vec<(usize, usize, bool)>,
}

impl FlatPath {
    /// Parses SVG path data. Path data a path shape rejects is an error.
    pub fn parse(d: &str) -> Result<FlatPath, PathError> {
        let cmds = commands(d)?;
        let (segs, starts, from_cmd) = segments_of(&cmds);
        let mut bounds = starts.clone();
        bounds.push(segs.len());
        let subs = bounds
            .windows(2)
            .filter(|w| w[1] > w[0])
            .map(|w| {
                // closed when the commands after its last segment, up to the next moveto, hold a closepath
                let after = &cmds[from_cmd[w[1] - 1]..];
                let rest = after.iter().position(|c| matches!(c, PathCmd::Move(_))).unwrap_or(after.len());
                let closed = after[..rest].iter().any(|c| matches!(c, PathCmd::Close(..)));
                (w[0], w[1], closed)
            })
            .collect();
        Ok(FlatPath { cmds, segs, subs })
    }

    /// Whether the path has no segments (the empty string, or movetos only).
    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    /// The outline of each subpath as the arc-length table samples it: its first point, then 32 points per
    /// segment on the true curve.
    fn outlines(&self) -> Vec<Vec<P>> {
        self.subs
            .iter()
            .map(|&(a, b, _)| {
                let mut pts = vec![self.segs[a].at(0.0)];
                for s in &self.segs[a..b] {
                    pts.extend((1..=SAMPLES).map(|k| s.at(k as f64 / SAMPLES as f64)));
                }
                pts
            })
            .collect()
    }

    /// Every sampled outline point, subpath after subpath.
    pub fn vertices(&self) -> Vec<P> {
        self.outlines().into_iter().flatten().collect()
    }
}

/// Direction of the segment's tangent at `t`, in degrees clockwise from +x (+y down).
fn heading(s: &Seg, t: f64) -> f64 {
    let d = s.tangent(t);
    libm::atan2(d[1], d[0]).to_degrees()
}

/// Point i of a `columns` × `rows` grid (clause 2).
pub fn grid(columns: u64, rows: u64, spacing_x: f64, spacing_y: f64, i: u64) -> Point {
    let (c, r) = (columns.max(1), rows.max(1));
    let (col, row) = (i % c, i / c);
    Point::at([(col as f64 - (c as f64 - 1.0) / 2.0) * spacing_x, (row as f64 - (r as f64 - 1.0) / 2.0) * spacing_y])
}

/// `count` points at equal drawn length along `path` (clause 3), measured as a motion path is: on one subpath an
/// along-path point at fraction f of the length is where a motion path with constant speed is at progress f. A jump
/// between subpaths takes no length; at a vertex, including a jump, a point takes the outgoing segment, and
/// segments of zero length have no direction. θ is the direction when `orient`.
pub fn along_path(path: &FlatPath, count: u64, orient: bool) -> Vec<Point> {
    if path.is_empty() || count == 0 {
        return Vec::new();
    }
    let segs = &path.segs;
    let starts: Vec<usize> = path.subs.iter().map(|s| s.0).collect();
    let (table, total) = arc_table(segs, &starts);
    let first = segs[0].at(0.0);
    if total == 0.0 {
        return (0..count).map(|_| Point::at(first)).collect();
    }
    // the drawn length of each segment, from the table
    // (a segment's length includes no jump: the table restarts at each subpath)
    let mut lens = vec![0.0; segs.len()];
    for w in table.windows(2) {
        lens[w[1].1] += w[1].0 - w[0].0;
    }
    let next_drawn = |from: usize| (from..segs.len()).find(|&j| lens[j] > 0.0);
    let last_drawn = (0..segs.len()).rev().find(|&j| lens[j] > 0.0).expect("the path has length");
    let closed = path.subs.len() == 1 && path.subs[0].2;
    let n = count as f64;
    (0..count)
        .map(|i| {
            let l = if closed {
                i as f64 * total / n
            } else if count == 1 {
                0.0
            } else {
                i as f64 * total / (n - 1.0)
            };
            let (seg, s) = locate(&table, l);
            let s = s.clamp(0.0, 1.0);
            let mut pos = segs[seg].at(s);
            // which segment gives the direction: this one, or the outgoing one at a vertex or a zero-length segment
            let at_end = s >= 1.0 && l < total;
            let candidate = if at_end {
                next_drawn(seg + 1)
            } else if lens[seg] == 0.0 {
                next_drawn(seg)
            } else {
                Some(seg)
            };
            let dir = match candidate {
                Some(d) if d != seg => {
                    // crossing a jump: the point is where the next subpath starts
                    if starts.iter().any(|&b| b > seg && b <= d) {
                        pos = segs[d].at(0.0);
                    }
                    heading(&segs[d], 0.0)
                }
                Some(d) => heading(&segs[d], s),
                None => heading(&segs[last_drawn], 1.0),
            };
            Point { x: pos[0], y: pos[1], theta: if orient { dir } else { 0.0 }, direction: dir }
        })
        .collect()
}

/// Point i of a scatter in a `width` × `height` rectangle centred on the origin (clause 4.1).
pub fn scatter_rect(seed: u64, width: f64, height: f64, i: u64) -> Point {
    Point::at([(unit(seed, 0, i) - 0.5) * width, (unit(seed, 1, i) - 0.5) * height])
}

/// The scatter inside a path region (clause 4.2): the accepted points and their candidate indices j.
/// Every subpath is closed for this purpose; `evenodd` selects the even-odd rule over nonzero.
pub fn scatter_path(path: &FlatPath, seed: u64, count: u64, evenodd: bool) -> (Vec<Point>, Vec<u64>) {
    if count == 0 || path.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut edges: Vec<(P, P)> = Vec::new();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for pts in path.outlines() {
        for p in &pts {
            x0 = x0.min(p[0]);
            y0 = y0.min(p[1]);
            x1 = x1.max(p[0]);
            y1 = y1.max(p[1]);
        }
        edges.extend(pts.windows(2).map(|w| (w[0], w[1])));
        // every subpath is closed for this purpose
        let (first, last) = (pts[0], pts[pts.len() - 1]);
        if first != last {
            edges.push((last, first));
        }
    }
    let (bw, bh) = (x1 - x0, y1 - y0);
    if bw == 0.0 || bh == 0.0 {
        return (Vec::new(), Vec::new());
    }
    let winding = |px: f64, py: f64| -> i64 {
        let mut w = 0;
        for &(a, b) in &edges {
            let crosses = |lo: P, hi: P| lo[1] <= py && py < hi[1];
            let right = || a[0] + (py - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) > px;
            if crosses(a, b) && right() {
                w += 1;
            } else if crosses(b, a) && right() {
                w -= 1;
            }
        }
        w
    };
    let mut pts = Vec::new();
    let mut js = Vec::new();
    let tries = count.saturating_mul(CANDIDATES_PER_POINT);
    for j in 0..tries {
        let (px, py) = (x0 + unit(seed, 0, j) * bw, y0 + unit(seed, 1, j) * bh);
        let w = winding(px, py);
        let inside = if evenodd { w % 2 != 0 } else { w != 0 };
        if inside {
            pts.push(Point::at([px, py]));
            js.push(j);
            if pts.len() as u64 == count {
                break;
            }
        }
    }
    (pts, js)
}

/// One point per drawing command of the path, at its end point (clause 5). A closepath adds none, and the
/// last point of a closed subpath is dropped when it coincides with the subpath's first point.
pub fn vertices(path: &FlatPath) -> Vec<Point> {
    let mut out: Vec<P> = Vec::new();
    // drawing commands since the subpath began
    let mut drawn = 0usize;
    for c in &path.cmds {
        match *c {
            PathCmd::Move(p) => {
                out.push(p);
                drawn = 0;
            }
            PathCmd::Close(_, start) => {
                if drawn > 0 && out.last() == Some(&start) {
                    out.pop();
                }
                drawn = 0;
            }
            _ => {
                out.push(c.end());
                drawn += 1;
            }
        }
    }
    out.into_iter().map(Point::at).collect()
}

/// The pairs of `@at` (clause 6).
pub fn list(at: &[P]) -> Vec<Point> {
    at.iter().copied().map(Point::at).collect()
}

/// How a repeat's points are laid out for one frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Layout {
    /// `grid`: point i from the live `spacingX` and `spacingY`.
    Grid {
        /// Columns.
        columns: u64,
        /// Rows.
        rows: u64,
    },
    /// `scatter` in a rectangle: point i from the live `width` and `height`.
    Rect,
    /// `along-path`, `scatter` in a path, `vertices` and `list`: points that do not animate.
    Fixed(Vec<Point>),
}

/// Index of `spacingX`, `spacingY`, `width` and `height` in [`Generator::base`] and [`Generator::slots`].
pub const ANIMATED: [&str; 4] = ["spacingX", "spacingY", "width", "height"];

/// The points of one repeat: n is fixed, positions may follow the four animated sizes.
#[derive(Debug, Clone, PartialEq)]
pub struct Generator {
    /// The repeat node whose copies are placed.
    pub node: u32,
    /// Layout.
    pub layout: Layout,
    /// s: `points/@seed`, else `project/@seed`.
    pub seed: u64,
    /// n, the number of points and copies.
    pub n: u32,
    /// Static `spacingX`, `spacingY`, `width` and `height` (see [`ANIMATED`]).
    pub base: [f64; 4],
    /// Their slots, when animated.
    pub slots: [Option<u32>; 4],
}

impl Generator {
    /// Point i, given the current `spacingX`, `spacingY`, `width` and `height`.
    pub fn point(&self, i: u32, live: [f64; 4]) -> Point {
        match &self.layout {
            Layout::Grid { columns, rows } => grid(*columns, *rows, live[0], live[1], i as u64),
            Layout::Rect => scatter_rect(self.seed, live[2], live[3], i as u64),
            Layout::Fixed(p) => p.get(i as usize).copied().unwrap_or(Point::at([0.0, 0.0])),
        }
    }

    /// `pointU` of point i.
    pub fn u(&self, i: u32) -> f64 {
        point_u(i as u64, self.n as u64)
    }

    /// `pointRandom` of point i: U(s, 2, i).
    pub fn random(&self, i: u32) -> f64 {
        unit(self.seed, 2, i as u64)
    }
}
