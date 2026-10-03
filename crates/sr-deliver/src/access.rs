//! Accessibility checks on rendered frames.
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

use std::collections::{BTreeMap, VecDeque};

use sr_gpu::ContrastTarget;

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

/// What the checks take from each rendered frame. Both checks compare frames over the whole output (a
/// flash counts transitions within a second, a text is judged at the frames where it is most visible),
/// so everything observed reaches one [`Judge`], in frame order.
pub(crate) trait Observe {
    /// The flash grid of the frame at output time `t`.
    fn flash(&mut self, t: f64, cells: Vec<[f64; 3]>);
    /// The contrast `ratio` of a text at accumulated opacity `opacity`, measured at time `t`.
    fn contrast(&mut self, id: &ContrastTarget, opacity: f64, ratio: f64, t: f64);
    /// A text the inline probe cannot measure, seen at `opacity` in frame `frame` of the frames observed.
    fn unprobed(&mut self, id: &ContrastTarget, frame: usize, opacity: f64);
    /// Flush one complete frame, applying backpressure when observations are streamed.
    fn finish_frame(&mut self) -> Result<(), String> {
        Ok(())
    }
}

/// The state of the checks over an output.
pub(crate) struct Judge {
    /// The flash analysis, when the flash check is on.
    pub flash: Option<FlashDetector>,
    /// Each text is judged at its most visible: the lowest ratio among the frames where its accumulated
    /// opacity is at its maximum over the encode. A label fading in passes through every ratio down to
    /// 1:1 on its way to rest, which is not what a reader faces; text held dim is judged dim.
    /// (opacity, ratio, time) for each independent text target.
    pub lowest: BTreeMap<ContrastTarget, (f64, f64, f64)>,
    /// Text requiring final-frame measurement: (frame index, opacity) where each appears.
    pub unprobed: BTreeMap<ContrastTarget, Vec<(usize, f64)>>,
}

impl Judge {
    pub fn new(flash: bool) -> Judge {
        Judge { flash: flash.then(FlashDetector::default), lowest: BTreeMap::new(), unprobed: BTreeMap::new() }
    }

    /// Takes what a stretch of frames showed; `first` is the index of its first frame in the output.
    /// Stretches must arrive in frame order.
    pub fn replay(&mut self, seen: Seen, first: usize) {
        for (t, cells) in seen.flash {
            self.flash(t, cells);
        }
        for (id, opacity, ratio, t) in &seen.contrast {
            self.contrast(id, *opacity, *ratio, *t);
        }
        for (id, frame, opacity) in &seen.unprobed {
            self.unprobed(id, first + frame, *opacity);
        }
    }
}

impl Observe for Judge {
    fn flash(&mut self, t: f64, cells: Vec<[f64; 3]>) {
        if let Some(d) = self.flash.as_mut() {
            d.push(t, &cells);
        }
    }

    fn contrast(&mut self, id: &ContrastTarget, opacity: f64, ratio: f64, t: f64) {
        let e = self.lowest.entry(id.clone()).or_insert((f64::MIN, f64::MAX, t));
        if opacity > e.0 + 1e-3 {
            *e = (opacity, ratio, t);
        } else if (opacity - e.0).abs() <= 1e-3 && ratio < e.1 {
            *e = (e.0, ratio, t);
        }
    }

    fn unprobed(&mut self, id: &ContrastTarget, frame: usize, opacity: f64) {
        self.unprobed.entry(id.clone()).or_default().push((frame, opacity));
    }
}

/// What a stretch of frames showed, kept until the frames before it have been judged: time segments
/// rendered at once finish in any order.
#[derive(Default)]
pub(crate) struct Seen {
    flash: Vec<(f64, Vec<[f64; 3]>)>,
    contrast: Vec<(ContrastTarget, f64, f64, f64)>,
    unprobed: Vec<(ContrastTarget, usize, f64)>,
}

impl Observe for Seen {
    fn flash(&mut self, t: f64, cells: Vec<[f64; 3]>) {
        self.flash.push((t, cells));
    }

    fn contrast(&mut self, id: &ContrastTarget, opacity: f64, ratio: f64, t: f64) {
        self.contrast.push((id.clone(), opacity, ratio, t));
    }

    fn unprobed(&mut self, id: &ContrastTarget, frame: usize, opacity: f64) {
        self.unprobed.push((id.clone(), frame, opacity));
    }
}

/// One bounded batch in flight plus a bounded channel per rendering worker. The coordinator drops
/// receivers on failure before joining producers, so a blocked send always has an escape.
pub(crate) const OBSERVATION_FRAMES: usize = 128;

pub(crate) struct Streaming {
    seen: Seen,
    frames: usize,
    limit: usize,
    tx: std::sync::mpsc::SyncSender<Seen>,
}

impl Streaming {
    pub fn new(tx: std::sync::mpsc::SyncSender<Seen>, checks: bool) -> Self {
        Self { seen: Seen::default(), frames: 0, limit: if checks { OBSERVATION_FRAMES } else { usize::MAX }, tx }
    }
    pub fn flush(&mut self) -> Result<(), String> {
        if self.frames == 0 {
            return Ok(());
        }
        self.tx.send(std::mem::take(&mut self.seen)).map_err(|_| "accessibility coordinator stopped".to_string())?;
        self.frames = 0;
        Ok(())
    }
}

impl Observe for Streaming {
    fn flash(&mut self, t: f64, cells: Vec<[f64; 3]>) {
        self.seen.flash(t, cells);
    }
    fn contrast(&mut self, id: &ContrastTarget, opacity: f64, ratio: f64, t: f64) {
        self.seen.contrast(id, opacity, ratio, t);
    }
    fn unprobed(&mut self, id: &ContrastTarget, frame: usize, opacity: f64) {
        self.seen.unprobed(id, frame, opacity);
    }
    fn finish_frame(&mut self) -> Result<(), String> {
        self.frames += 1;
        if self.frames == self.limit {
            self.flush()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod streaming_tests {
    use super::*;

    #[test]
    fn accessibility_stream_is_bounded_and_disconnect_unblocks_producer() {
        let (tx, rx) = std::sync::mpsc::sync_channel(2);
        let (ready, progress) = std::sync::mpsc::channel();
        let producer = std::thread::spawn(move || {
            let mut stream = Streaming::new(tx, true);
            for k in 0..10000 {
                stream.flash(k as f64, vec![[0.5; 3]; 48 * 27]);
                if stream.finish_frame().is_err() {
                    return k;
                }
                if (k + 1) % OBSERVATION_FRAMES == 0 {
                    ready.send(k / OBSERVATION_FRAMES).unwrap();
                }
            }
            10000
        });
        assert_eq!(progress.recv().unwrap(), 0);
        assert_eq!(progress.recv().unwrap(), 1);
        assert!(progress.recv_timeout(std::time::Duration::from_millis(30)).is_err());
        let mut judge = Judge::new(true);
        judge.replay(rx.recv().unwrap(), 0);
        assert_eq!(progress.recv().unwrap(), 2);
        drop(rx);
        assert_eq!(producer.join().unwrap(), 4 * OBSERVATION_FRAMES - 1);
    }

    #[test]
    fn streamed_frames_preserve_order_and_chunk_offsets() {
        let (tx, rx) = std::sync::mpsc::sync_channel(2);
        let mut stream = Streaming::new(tx, true);
        let id = ContrastTarget::Node("label".into());
        let mut direct = Judge::new(true);
        let mut replayed = Judge::new(true);
        for k in 0..120 {
            let cells = vec![[if k % 2 == 0 { 0.0 } else { 1.0 }; 3]; 48 * 27];
            let t = k as f64 / 30.0;
            let opacity = 0.5 + k as f64 * 0.0004;
            direct.flash(t, cells.clone());
            direct.contrast(&id, opacity, 2.0 + t, t);
            direct.unprobed(&id, 200 + k, opacity);
            stream.flash(t, cells);
            stream.contrast(&id, opacity, 2.0 + t, t);
            stream.unprobed(&id, k, opacity);
            stream.finish_frame().unwrap();
            stream.flush().unwrap();
            replayed.replay(rx.recv().unwrap(), 200);
        }
        assert_eq!(direct.flash.unwrap().verdict(), replayed.flash.unwrap().verdict());
        assert_eq!(direct.lowest, replayed.lowest);
        assert_eq!(direct.unprobed, replayed.unprobed);
    }
}
