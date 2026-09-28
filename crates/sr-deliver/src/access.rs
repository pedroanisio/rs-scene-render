//! Accessibility checks on rendered frames (Batch 9).
//!
//! **Flash analysis** follows ITU-R BT.1702 and WCAG 2.3.1. Each frame
//! arrives as a 48 × 27 grid of display-referred linear sRGB. In every cell:
//!
//! * a *general transition* is a change in relative luminance of at least
//!   0.1 where the darker state is below 0.8;
//! * a *red transition* involves a saturated red state (R / (R + G + B) ≥ 0.8)
//!   and changes (R − G − B) × 320 (negative values clipped to 0) by more
//!   than 20.
//!
//! Transitions are counted with hysteresis: a new transition needs a swing
//! of the threshold against the previous extreme. A cell *flashes* when it
//! makes more than three flashes (eight or more opposing transitions) within
//! one second. A frame fails when the flashing cells cover more than 25 % of
//! any window one third of the frame wide and tall, the 10° visual field at
//! the reference 1024 × 768 viewing geometry.

use std::collections::VecDeque;

pub const COLS: usize = 48;
pub const ROWS: usize = 27;
const WIN_W: usize = COLS / 3;
const WIN_H: usize = ROWS / 3;

#[derive(Clone, Default)]
struct Track {
    ext: Option<f64>,
    dir: i8,
    times: VecDeque<f64>,
}

impl Track {
    /// Feeds a value; `transition(a, b)` decides whether a swing from `a` to `b` counts.
    fn push(&mut self, t: f64, v: f64, threshold: f64, transition: impl Fn(f64, f64) -> bool) {
        let Some(e) = self.ext else {
            self.ext = Some(v);
            return;
        };
        let swing = v - e;
        let reverses = (self.dir >= 0 && swing <= -threshold) || (self.dir <= 0 && swing >= threshold);
        if reverses && transition(e, v) {
            self.times.push_back(t);
            self.dir = if swing > 0.0 { 1 } else { -1 };
            self.ext = Some(v);
        } else if (self.dir >= 0 && v > e) || (self.dir <= 0 && v < e) {
            // extend the current excursion
            self.ext = Some(v);
        }
        while self.times.front().is_some_and(|&f| f <= t - 1.0) {
            self.times.pop_front();
        }
    }

    fn flashing(&self) -> bool {
        self.times.len() >= 8
    }
}

/// Luminance of display-linear sRGB.
fn luma(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn red_ratio(c: [f64; 3]) -> f64 {
    let s = c[0] + c[1] + c[2];
    if s > 0.0 {
        c[0] / s
    } else {
        0.0
    }
}

/// Streaming flash detector.
pub struct FlashDetector {
    general: Vec<Track>,
    red: Vec<Track>,
    last: Vec<[f64; 3]>,
    /// Frames that fail the general-flash test, and the first failing time.
    pub general_frames: usize,
    pub red_frames: usize,
    pub first: Option<f64>,
    pub frames: usize,
}

impl Default for FlashDetector {
    fn default() -> Self {
        FlashDetector {
            general: vec![Track::default(); COLS * ROWS],
            red: vec![Track::default(); COLS * ROWS],
            last: Vec::new(),
            general_frames: 0,
            red_frames: 0,
            first: None,
            frames: 0,
        }
    }
}

/// Largest count of set cells in any window of the grid.
fn window_max(mask: &[bool]) -> usize {
    let mut best = 0;
    for y0 in 0..=ROWS - WIN_H {
        for x0 in 0..=COLS - WIN_W {
            let mut n = 0;
            for y in y0..y0 + WIN_H {
                for x in x0..x0 + WIN_W {
                    n += mask[y * COLS + x] as usize;
                }
            }
            best = best.max(n);
        }
    }
    best
}

impl FlashDetector {
    /// Feeds the grid of one frame at time `t`.
    pub fn push(&mut self, t: f64, cells: &[[f64; 3]]) {
        if cells.len() != COLS * ROWS {
            return;
        }
        self.frames += 1;
        for (k, c) in cells.iter().enumerate() {
            self.general[k].push(t, luma(*c), 0.1, |a, b| a.min(b) < 0.8);
            let prev = self.last.get(k).copied().unwrap_or(*c);
            let saturated = red_ratio(*c) >= 0.8 || red_ratio(prev) >= 0.8;
            let redness = ((c[0] - c[1] - c[2]) * 320.0).max(0.0);
            self.red[k].push(t, redness, 20.0 + 1e-9, |_, _| saturated);
        }
        self.last = cells.to_vec();
        let limit = WIN_W * WIN_H / 4;
        let g: Vec<bool> = self.general.iter().map(Track::flashing).collect();
        let r: Vec<bool> = self.red.iter().map(Track::flashing).collect();
        let (gf, rf) = (window_max(&g) > limit, window_max(&r) > limit);
        self.general_frames += gf as usize;
        self.red_frames += rf as usize;
        if (gf || rf) && self.first.is_none() {
            self.first = Some(t);
        }
    }

    /// A description of the failure, if any frame failed.
    pub fn verdict(&self) -> Option<String> {
        if self.general_frames == 0 && self.red_frames == 0 {
            return None;
        }
        Some(format!(
            "flashCheck: {} frames exceed the general-flash limit and {} the red-flash limit (more than 3 flashes per second over 25% of a 10° field), first at {:.3} s",
            self.general_frames,
            self.red_frames,
            self.first.unwrap_or(0.0)
        ))
    }
}
