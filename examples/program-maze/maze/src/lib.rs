//! A scene-render `<program>` (SREP 66): a maze by recursive backtracking, drawn on as two animated paths, the
//! walls and then the solution.
//!
//! Parameters (`<param name=… value=…/>`, read with `sr.param_f64`):
//! `cols`, `rows` (cells, default 32 × 18), `cell` (pixels, default 50), `x`, `y` (top-left corner, default 160, 90),
//! `wall` (stroke width, default 6), `draw` (seconds the walls take to draw on, default 6), `solve` (seconds for
//! the solution, default 3), `pause` (seconds between them, default 0.5).
//!
//! Randomness comes only from `sr.rand_u64`, the seeded generator of SREP 66, so the same seed gives the same maze
//! on every host. Build: `./build.sh` (needs the wasm32-unknown-unknown target).

use std::fmt::Write;

#[link(wasm_import_module = "sr")]
extern "C" {
    fn rand_u64() -> u64;
    fn param_f64(name: *const u8, len: usize, default: f64) -> f64;
}

fn param(name: &str, default: f64) -> f64 {
    // SAFETY: the host reads `len` bytes at `name` from this module's memory, which the slice holds
    unsafe { param_f64(name.as_ptr(), name.len(), default) }
}

fn random_below(n: usize) -> usize {
    // SAFETY: a host import without arguments
    let r = unsafe { rand_u64() };
    // Lemire's multiply-shift: unbiased enough for n far below 2^32, and the same on every host
    ((r >> 32) * n as u64 >> 32) as usize
}

const N: u8 = 1;
const E: u8 = 2;
const S: u8 = 4;
const W: u8 = 8;

/// Carves passages: `open[c]` holds the directions in which cell `c` has no wall.
fn carve(cols: usize, rows: usize) -> Vec<u8> {
    let mut open = vec![0u8; cols * rows];
    let mut seen = vec![false; cols * rows];
    let mut stack = vec![0usize];
    seen[0] = true;
    while let Some(&c) = stack.last() {
        let (x, y) = (c % cols, c / cols);
        let mut next = [(0usize, 0u8, 0u8); 4];
        let mut n = 0;
        if y > 0 && !seen[c - cols] {
            next[n] = (c - cols, N, S);
            n += 1;
        }
        if x + 1 < cols && !seen[c + 1] {
            next[n] = (c + 1, E, W);
            n += 1;
        }
        if y + 1 < rows && !seen[c + cols] {
            next[n] = (c + cols, S, N);
            n += 1;
        }
        if x > 0 && !seen[c - 1] {
            next[n] = (c - 1, W, E);
            n += 1;
        }
        if n == 0 {
            stack.pop();
            continue;
        }
        let (to, d, back) = next[random_below(n)];
        open[c] |= d;
        open[to] |= back;
        seen[to] = true;
        stack.push(to);
    }
    open
}

/// The cells from the top-left to the bottom-right corner, by breadth-first search.
fn solve(open: &[u8], cols: usize) -> Vec<usize> {
    let goal = open.len() - 1;
    let mut from = vec![usize::MAX; open.len()];
    let mut queue = std::collections::VecDeque::from([0usize]);
    from[0] = 0;
    while let Some(c) = queue.pop_front() {
        if c == goal {
            break;
        }
        for (d, step) in [(N, -(cols as isize)), (E, 1), (S, cols as isize), (W, -1)] {
            if open[c] & d != 0 {
                let to = (c as isize + step) as usize;
                if from[to] == usize::MAX {
                    from[to] = c;
                    queue.push_back(to);
                }
            }
        }
    }
    let mut path = vec![goal];
    while *path.last().unwrap() != 0 {
        path.push(from[*path.last().unwrap()]);
    }
    path.reverse();
    path
}

#[no_mangle]
pub extern "C" fn generate() -> u64 {
    let cols = param("cols", 32.0).clamp(2.0, 400.0) as usize;
    let rows = param("rows", 18.0).clamp(2.0, 400.0) as usize;
    let cell = param("cell", 50.0).max(1.0);
    let (x0, y0) = (param("x", 160.0), param("y", 90.0));
    let wall = param("wall", 6.0).max(0.5);
    let (draw, solve_t, pause) = (param("draw", 6.0).max(0.1), param("solve", 3.0).max(0.1), param("pause", 0.5).max(0.0));
    let open = carve(cols, rows);

    // walls: every north and west wall of each cell, then the south and east borders, as one path of segments
    // (the entrance at the top-left and the exit at the bottom-right stay open)
    let (w, h) = (cols as f64 * cell, rows as f64 * cell);
    let mut walls = String::new();
    let p = |x: usize, y: usize| (x0 + x as f64 * cell, y0 + y as f64 * cell);
    for y in 0..rows {
        for x in 0..cols {
            let c = y * cols + x;
            if open[c] & N == 0 && !(x == 0 && y == 0) {
                let ((ax, ay), (bx, by)) = (p(x, y), p(x + 1, y));
                let _ = write!(walls, "M{ax} {ay}L{bx} {by}");
            }
            if open[c] & W == 0 {
                let ((ax, ay), (bx, by)) = (p(x, y), p(x, y + 1));
                let _ = write!(walls, "M{ax} {ay}L{bx} {by}");
            }
        }
    }
    let _ = write!(walls, "M{} {}L{} {}", x0, y0 + h, x0 + w - cell, y0 + h);
    let _ = write!(walls, "M{} {}L{} {}", x0 + w, y0, x0 + w, y0 + h);

    let route = solve(&open, cols);
    let mut sol = String::new();
    for (i, c) in route.iter().enumerate() {
        let (cx, cy) = (x0 + ((c % cols) as f64 + 0.5) * cell, y0 + ((c / cols) as f64 + 0.5) * cell);
        let _ = write!(sol, "{}{cx} {cy}", if i == 0 { "M" } else { "L" });
    }

    let (t1, t2) = (draw + pause, draw + pause + solve_t);
    let mut out = String::new();
    let _ = write!(
        out,
        r##"<shape id="maze-walls" shape="path" width="{w}" height="{h}" path="{walls}" fill="#00000000" stroke="#E8E4D8FF" strokeWidth="{wall}" strokeCap="round" trimEnd="0"><animate property="trimEnd"><key time="0" value="0" interpolation="ease-in-out"/><key time="{draw}" value="1"/></animate></shape>"##
    );
    let _ = write!(
        out,
        r##"<shape id="maze-route" shape="path" width="{w}" height="{h}" path="{sol}" fill="#00000000" stroke="#FF5A36FF" strokeWidth="{}" strokeCap="round" strokeJoin="round" trimEnd="0"><animate property="trimEnd"><key time="{t1}" value="0" interpolation="ease-in-out"/><key time="{t2}" value="1"/></animate></shape>"##,
        cell * 0.28
    );
    let bytes = out.into_bytes().into_boxed_slice();
    let (ptr, len) = (bytes.as_ptr() as u64, bytes.len() as u64);
    std::mem::forget(bytes);
    (ptr << 32) | len
}
