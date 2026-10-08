//! CPU fine-stage output and allocation regressions. Allocation accounting is local to the calling thread.
use sr_vector::tile::{self, coverage, kind, Encoded, TILE};
use sr_vector::{shapes, Cmd, FillRule, MaskOp, MatteMode, Paint, Scene};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! { static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) }; }
struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|n| {
            if let Some(v) = n.get() {
                n.set(Some(v + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|n| {
            if let Some(v) = n.get() {
                n.set(Some(v + 1));
            }
        });
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;
fn fill() -> Cmd {
    Cmd::Fill {
        polys: shapes::rect(0., 0., 65., 33., [0.; 4]).flatten(0.1),
        rule: FillRule::NonZero,
        paint: Paint::Solid { rgba: [0.3, 0.6, 0.8, 0.5], srgb: false },
        opacity: 0.8,
    }
}
#[test]
fn grouped_rasterization_allocations_do_not_grow_with_pixel_count() {
    let scene = Scene { cmds: vec![Cmd::Push { mask_init: 1. }, fill(), fill(), Cmd::Pop { opacity: 0.7 }] };
    let encoded = tile::encode(&scene, [65, 33]);
    assert!(encoded.cmds.iter().any(|c| c.kind == kind::PUSH));
    ALLOCATIONS.with(|n| n.set(Some(0)));
    let pixels = tile::render_cpu(&encoded, &|_, _, _| [0.3, 0.6, 0.8, 0.5]);
    let allocations = ALLOCATIONS.with(|n| n.replace(None).unwrap());
    assert_eq!(pixels.len(), 65 * 33);
    assert!(allocations <= 4, "fine rasterization allocated {allocations} times for 2145 pixels");
}
#[test]
fn nested_masks_mattes_and_partial_rows_match_original_fine_stage() {
    for mode in [MatteMode::Alpha, MatteMode::AlphaInverted, MatteMode::Luma, MatteMode::LumaInverted] {
        let scene = Scene {
            cmds: vec![
                Cmd::Push { mask_init: 1. },
                fill(),
                fill(),
                Cmd::Push { mask_init: 0. },
                fill(),
                Cmd::Mask {
                    polys: shapes::rect(5.5, 2.25, 27., 25., [0.; 4]).flatten(0.1),
                    rule: FillRule::EvenOdd,
                    op: MaskOp::Add,
                    opacity: 0.6,
                    invert: false,
                },
                Cmd::Pop { opacity: 0.7 },
                Cmd::PushMatte,
                fill(),
                Cmd::PopMatte { mode, opacity: 0.8 },
            ],
        };
        let e = tile::encode(&scene, [65, 33]);
        let paint = |i: u32, x: f32, y: f32| [0.2 + i as f32 * 0.03, x / 100., y / 100., 0.6];
        assert_eq!(tile::render_cpu(&e, &paint), reference(&e, 0..e.tiles[1], &paint));
        for row in 0..e.tiles[1] {
            assert_eq!(tile::render_cpu_rows(&e, row..row + 1, &paint), reference(&e, row..row + 1, &paint));
        }
    }
}
// Original fine-stage interpreter: retain its per-pixel stack as an independent equivalence oracle.
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

fn reference(e: &Encoded, rows: std::ops::Range<u32>, paint: &dyn Fn(u32, f32, f32) -> [f32; 4]) -> Vec<[f32; 4]> {
    let [w, h] = e.size;
    let y_first = rows.start * TILE;
    let y_end = (rows.end * TILE).min(h);
    let mut out = vec![[0.0f32; 4]; (w * y_end.saturating_sub(y_first)) as usize];
    for ty in rows {
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
                    out[((y - y_first) * w + x) as usize] = acc;
                }
            }
        }
    }
    out
}
