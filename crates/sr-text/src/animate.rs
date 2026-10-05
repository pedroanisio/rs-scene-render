//! Text animators (range, wiggly and expression selectors over characters,
//! words, lines or spans, and the 24 presets) and text on a path.

use sr_vector::geom::{p, Xf};
use sr_vector::{Paint, Path};

use crate::font::FontLib;
use crate::glyph::GlyphFx;
use crate::layout::Layout;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Unit {
    #[default]
    Char,
    CharNoSpace,
    Word,
    Line,
    Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shape {
    #[default]
    Square,
    RampUp,
    RampDown,
    Triangle,
    Round,
    Smooth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Order {
    #[default]
    Forward,
    Reverse,
    CenterOut,
    EdgesIn,
    Random,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Combine {
    #[default]
    Add,
    Multiply,
    Replace,
}

/// The 24 presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Typewriter,
    FadeIn,
    FadeOut,
    WordByWord,
    LetterByLetter,
    LineByLine,
    SlideUp,
    SlideDown,
    SlideLeft,
    SlideRight,
    Pop,
    ScaleIn,
    BlurIn,
    Wave,
    Bounce,
    Spin,
    Ascend,
    Shift,
    Scramble,
    Counter,
    Karaoke,
    Highlight,
    TrackingIn,
    MaskReveal,
}

impl Preset {
    /// Parses a preset name.
    pub fn parse(s: &str) -> Option<Preset> {
        use Preset::*;
        Some(match s {
            "typewriter" => Typewriter,
            "fade-in" => FadeIn,
            "fade-out" => FadeOut,
            "word-by-word" => WordByWord,
            "letter-by-letter" => LetterByLetter,
            "line-by-line" => LineByLine,
            "slide-up" => SlideUp,
            "slide-down" => SlideDown,
            "slide-left" => SlideLeft,
            "slide-right" => SlideRight,
            "pop" => Pop,
            "scale-in" => ScaleIn,
            "blur-in" => BlurIn,
            "wave" => Wave,
            "bounce" => Bounce,
            "spin" => Spin,
            "ascend" => Ascend,
            "shift" => Shift,
            "scramble" => Scramble,
            "counter" => Counter,
            "karaoke" => Karaoke,
            "highlight" => Highlight,
            "tracking-in" => TrackingIn,
            "mask-reveal" => MaskReveal,
            _ => return None,
        })
    }
}

/// Animated property values (targets at full selection).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Props {
    /// Variable-font axis targets (`wght 700, wdth 80`).
    pub variation: Option<Vec<([u8; 4], f32)>>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub z_depth: Option<f64>,
    /// Factors (1 = unchanged), as on nodes.
    pub scale: Option<f64>,
    pub scale_x: Option<f64>,
    pub scale_y: Option<f64>,
    pub rotation: Option<f64>,
    pub rotation_x: Option<f64>,
    pub rotation_y: Option<f64>,
    pub skew: Option<f64>,
    /// 0–1 (1 = unchanged).
    pub opacity: Option<f64>,
    pub fill: Option<Paint>,
    pub stroke: Option<Paint>,
    pub stroke_width: Option<f64>,
    pub tracking: Option<f64>,
    pub line_spacing: Option<f64>,
    pub blur: Option<f64>,
    pub baseline_shift: Option<f64>,
    pub char_offset: Option<f64>,
    pub anchor_x: Option<f64>,
    pub anchor_y: Option<f64>,
}

/// How units are selected.
#[derive(Debug, Clone, PartialEq)]
pub enum Selector {
    Range {
        percent: bool,
        start: f64,
        end: f64,
        offset: f64,
        amount: f64,
        shape: Shape,
        smoothness: f64,
        ease_high: f64,
        ease_low: f64,
        order: Order,
        seed: u64,
    },
    Wiggly {
        amount: f64,
        rate: f64,
        seed: u64,
    },
    /// Per-unit amounts in percent (expression selector, evaluated by the caller).
    Values(Vec<f64>),
}

/// One animator with its values at this frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Animator {
    pub unit: Unit,
    /// `unit` was given explicitly (it overrides a preset's unit).
    pub unit_set: bool,
    /// Restrict to the span with this role.
    pub role: Option<String>,
    pub selector: Selector,
    pub props: Props,
    pub combine: Combine,
    /// Preset with its start and duration (seconds on the layer's timeline).
    pub preset: Option<(Preset, f64, f64)>,
    /// Preset per-unit delay, seconds.
    pub stagger: Option<f64>,
    /// Preset overlap between units, 0–1 (the preset's own when `None`).
    pub overlap: Option<f64>,
    pub seed: u64,
}

impl Default for Animator {
    fn default() -> Animator {
        Animator {
            unit: Unit::Char,
            unit_set: false,
            role: None,
            selector: Selector::Range {
                percent: true,
                start: 0.0,
                end: 100.0,
                offset: 0.0,
                amount: 100.0,
                shape: Shape::Square,
                smoothness: 1.0,
                ease_high: 0.0,
                ease_low: 0.0,
                order: Order::Forward,
                seed: 0,
            },
            props: Props::default(),
            combine: Combine::Add,
            preset: None,
            stagger: None,
            overlap: None,
            seed: 0,
        }
    }
}

fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Easing curves of the preset table (the Penner curves).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ease {
    Linear,
    QuadIn,
    QuadOut,
    CubicOut,
    ExpoOut,
    BackOut,
    BounceOut,
}

impl Ease {
    /// Eased progress at `u` in [0, 1].
    pub fn at(self, u: f64) -> f64 {
        let out = |f: fn(f64) -> f64| 1.0 - f(1.0 - u);
        match self {
            Ease::Linear => u,
            Ease::QuadIn => u * u,
            Ease::QuadOut => out(|v| v * v),
            Ease::CubicOut => out(|v| v * v * v),
            Ease::ExpoOut => out(|v| if v == 0.0 { 0.0 } else { libm::pow(2.0, 10.0 * v - 10.0) }),
            Ease::BackOut => out(|v| {
                let c1 = 1.70158;
                (c1 + 1.0) * v * v * v - c1 * v * v
            }),
            Ease::BounceOut => bounce_out(u),
        }
    }
}

fn bounce_out(x: f64) -> f64 {
    let (n1, d1) = (7.5625, 2.75);
    if x < 1.0 / d1 {
        n1 * x * x
    } else if x < 2.0 / d1 {
        let x = x - 1.5 / d1;
        n1 * x * x + 0.75
    } else if x < 2.5 / d1 {
        let x = x - 2.25 / d1;
        n1 * x * x + 0.9375
    } else {
        let x = x - 2.625 / d1;
        n1 * x * x + 0.984375
    }
}

/// Unit index of each glyph and the unit count, honouring a role restriction.
fn units(lay: &Layout, unit: Unit, role_span: Option<&dyn Fn(usize) -> bool>) -> (Vec<Option<usize>>, usize) {
    let in_role = |span: usize| role_span.is_none_or(|f| f(span));
    let mut map = vec![None; lay.glyphs.len()];
    match unit {
        Unit::Char | Unit::CharNoSpace => {
            // characters in logical order, counting only those in the role
            let mut idx = vec![None; lay.chars.len()];
            let mut k = 0;
            for (c, ch) in lay.chars.iter().enumerate() {
                if !in_role(lay.char_span[c]) || (unit == Unit::CharNoSpace && ch.is_whitespace()) {
                    continue;
                }
                idx[c] = Some(k);
                k += 1;
            }
            for (g, gl) in lay.glyphs.iter().enumerate() {
                map[g] = idx.get(gl.ch).copied().flatten();
            }
            (map, k)
        }
        Unit::Word | Unit::Span | Unit::Line => {
            let key = |g: &crate::layout::PGlyph| match unit {
                Unit::Word => g.word,
                Unit::Span => g.span,
                _ => g.line,
            };
            let mut keys: Vec<usize> =
                lay.glyphs.iter().filter(|g| in_role(g.span) && !lay.chars[g.ch].is_whitespace()).map(key).collect();
            keys.sort_unstable();
            keys.dedup();
            for (g, gl) in lay.glyphs.iter().enumerate() {
                if in_role(gl.span) && !lay.chars[gl.ch].is_whitespace() {
                    map[g] = keys.binary_search(&key(gl)).ok();
                }
            }
            (map, keys.len())
        }
    }
}

/// Position of unit `i` of `n` in the selection order: forward i, reverse n − 1 − i,
/// center-out 2·|i − c|, edges-in n − 1 − 2·|i − c| (c the middle index), random entry i of
/// the seeded permutation of noise channel 0 ([`sr_vector::d24::permutation`]).
fn order_index(i: usize, n: usize, order: Order, seed: u64) -> f64 {
    let c = (n as f64 - 1.0) * 0.5;
    match order {
        Order::Forward => i as f64,
        Order::Reverse => (n - 1 - i) as f64,
        Order::CenterOut => (i as f64 - c).abs() * 2.0,
        Order::EdgesIn => (n as f64 - 1.0) - (i as f64 - c).abs() * 2.0,
        Order::Random => sr_vector::d24::permutation(seed, 0, n).get(i).copied().unwrap_or(i) as f64,
    }
}

fn range_amount(sel: &Selector, i: usize, n: usize, t: f64) -> f64 {
    match sel {
        Selector::Values(v) => v.get(i).copied().unwrap_or(0.0) / 100.0,
        Selector::Wiggly { amount, rate, seed } => {
            // fractal noise N(seed, 0, t · rate + 7.31 · i)
            sr_vector::d24::noise(*seed, 0, t * rate + i as f64 * 7.31) * amount / 100.0
        }
        Selector::Range {
            percent,
            start,
            end,
            offset,
            amount,
            shape,
            smoothness,
            ease_high,
            ease_low,
            order,
            seed,
        } => {
            let nf = n as f64;
            let (mut s, mut e) = if *percent {
                ((start + offset) / 100.0 * nf, (end + offset) / 100.0 * nf)
            } else {
                (start + offset, end + offset)
            };
            if s > e {
                std::mem::swap(&mut s, &mut e);
            }
            let x = order_index(i, n, *order, *seed);
            let span = (e - s).max(1e-9);
            let u = (x + 0.5 - s) / span;
            let v = match shape {
                Shape::Square => {
                    let cov = ((x + 1.0).min(e) - x.max(s)).clamp(0.0, 1.0);
                    let hard = if cov >= 0.5 { 1.0 } else { 0.0 };
                    hard + (cov - hard) * smoothness
                }
                Shape::RampUp => u.clamp(0.0, 1.0),
                Shape::RampDown => 1.0 - u.clamp(0.0, 1.0),
                Shape::Triangle if (0.0..=1.0).contains(&u) => 1.0 - (2.0 * u - 1.0).abs(),
                Shape::Round if (0.0..=1.0).contains(&u) => libm::sqrt((1.0 - (2.0 * u - 1.0).powi(2)).max(0.0)),
                Shape::Smooth if (0.0..=1.0).contains(&u) => 0.5 - 0.5 * libm::cos(std::f64::consts::TAU * u),
                _ => 0.0,
            };
            let ease = ((ease_high + ease_low) / 200.0).clamp(0.0, 1.0);
            let v = v + (smooth(v) - v) * ease;
            v * amount / 100.0
        }
    }
}

/// How a preset times its units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Unit p shows from start + p·D/M on, without a fade.
    Step,
    /// Units rest in the offset state until their window starts: s = 1 − ease(q).
    In,
    /// s = ease(q).
    Out,
    /// s = sin(2π(1.5·(t − start) − p/8)), faded in and out over 10 % of D.
    Wave,
    /// Hidden before the start; then unrevealed units show seeded random letters.
    Scramble,
    /// Numbers count up (text substitution before layout, [`counter_text`]).
    Counter,
}

/// The preset table:
/// unit, timing mode, ease and overlap.
fn table(kind: Preset) -> (Unit, Mode, Ease, f64) {
    use Ease::*;
    use Mode::*;
    use Preset::*;
    match kind {
        Typewriter => (Unit::Char, Step, Linear, 0.0),
        FadeIn => (Unit::Char, In, QuadOut, 0.6),
        FadeOut => (Unit::Char, Out, QuadIn, 0.6),
        WordByWord => (Unit::Word, Step, Linear, 0.0),
        LetterByLetter => (Unit::Char, In, CubicOut, 0.5),
        LineByLine => (Unit::Line, In, CubicOut, 0.3),
        SlideUp | SlideDown | SlideLeft | SlideRight => (Unit::Word, In, CubicOut, 0.5),
        Pop => (Unit::Word, In, BackOut, 0.5),
        ScaleIn => (Unit::Word, In, CubicOut, 0.5),
        BlurIn => (Unit::Word, In, CubicOut, 0.6),
        Preset::Wave => (Unit::Char, Mode::Wave, Linear, 0.0),
        Bounce => (Unit::Char, In, BounceOut, 0.6),
        Spin => (Unit::Char, In, CubicOut, 0.5),
        Ascend => (Unit::Char, In, ExpoOut, 0.8),
        Shift => (Unit::Char, In, ExpoOut, 0.7),
        Preset::Scramble => (Unit::Char, Mode::Scramble, Linear, 0.0),
        Preset::Counter => (Unit::Char, Mode::Counter, CubicOut, 0.0),
        Karaoke | Highlight => (Unit::Word, Out, Linear, 0.0),
        TrackingIn => (Unit::Char, In, ExpoOut, 0.85),
        MaskReveal => (Unit::Line, In, CubicOut, 0.3),
    }
}

/// Default karaoke fill, #FFD400.
pub const KARAOKE: [f64; 4] = [1.0, 212.0 / 255.0, 0.0, 1.0];
/// Default highlight box paint, #FFD40059.
const HIGHLIGHT: [f64; 4] = [1.0, 212.0 / 255.0, 0.0, 89.0 / 255.0];

/// A preset expanded for one frame.
struct Expanded {
    unit: Unit,
    /// Selection per unit.
    amounts: Vec<f64>,
    /// Selection per unit of the opacity offset when its ease differs (pop, bounce).
    opacity: Option<Vec<f64>>,
    props: Props,
    /// Clip each unit to its line box (mask-reveal).
    clip: bool,
    /// Highlight box paint (the highlight preset).
    highlight: Option<Paint>,
}

/// Expands a preset for `n` units at layer time `t` (em: the text size).
fn preset(a: &Animator, kind: Preset, start: f64, dur: f64, n: usize, t: f64, em: f64) -> Expanded {
    use Preset::*;
    let (unit0, mode, ease, overlap0) = table(kind);
    let unit = if a.unit_set { a.unit } else { unit0 };
    let (order, amount, order_seed) = match &a.selector {
        Selector::Range { order, amount, seed, .. } => (*order, amount / 100.0, *seed),
        _ => (Order::Forward, 1.0, 0),
    };
    let pos: Vec<f64> = (0..n).map(|i| order_index(i, n, order, order_seed)).collect();
    let slots = pos.iter().fold(0.0f64, |m, p| m.max(*p)) + 1.0;
    let big_d = dur;
    let o = a.overlap.unwrap_or(overlap0).clamp(0.0, 1.0);
    // a unit lasts d and starts (1 − o)·d after the previous one; @stagger sets that step
    let (d, step) = match a.stagger {
        Some(st) => (if o < 1.0 { st / (1.0 - o) } else { big_d }, st),
        None => {
            let d = big_d / (1.0 + (slots - 1.0) * (1.0 - o));
            (d, d * (1.0 - o))
        }
    };
    let q = |p: f64| -> f64 {
        if d > 0.0 {
            ((t - start - p * step) / d).clamp(0.0, 1.0)
        } else if t >= start + p * step {
            1.0
        } else {
            0.0
        }
    };
    let sel = |e: Ease, p: f64| -> f64 {
        let v = e.at(q(p));
        if mode == Mode::In {
            1.0 - v
        } else {
            v
        }
    };
    let st = big_d / slots.max(1.0);
    let mut amounts: Vec<f64> = match mode {
        Mode::Step => pos.iter().map(|p| if t >= start + p * st - 1e-9 { 0.0 } else { 1.0 }).collect(),
        Mode::Wave => {
            if t < start || t > start + big_d {
                vec![0.0; n]
            } else {
                let env =
                    ((t - start) / (0.1 * big_d).max(1e-9)).min((start + big_d - t) / (0.1 * big_d).max(1e-9)).min(1.0);
                pos.iter().map(|p| libm::sin(std::f64::consts::TAU * (1.5 * (t - start) - p / 8.0)) * env).collect()
            }
        }
        // hidden before the start; the letters are substituted before layout ([`scramble_text`])
        Mode::Scramble => vec![if t < start { 1.0 } else { 0.0 }; n],
        Mode::Counter => vec![0.0; n],
        Mode::In | Mode::Out => pos.iter().map(|p| sel(ease, *p)).collect(),
    };
    for v in &mut amounts {
        *v *= amount;
    }
    // the opacity of pop and bounce eases on its own curve
    let opacity = match kind {
        Pop => Some(Ease::CubicOut),
        Bounce => Some(Ease::QuadOut),
        _ => None,
    }
    .map(|e| pos.iter().map(|p| sel(e, *p) * amount).collect());
    let mut pr = Props::default();
    match kind {
        Typewriter | FadeIn | FadeOut | WordByWord => pr.opacity = Some(0.0),
        LetterByLetter => {
            pr.opacity = Some(0.0);
            pr.y = Some(0.15 * em);
        }
        LineByLine => {
            pr.opacity = Some(0.0);
            pr.y = Some(0.4 * em);
        }
        SlideUp | SlideDown | SlideLeft | SlideRight => {
            pr.opacity = Some(0.0);
            match kind {
                SlideUp => pr.y = Some(0.8 * em),
                SlideDown => pr.y = Some(-0.8 * em),
                SlideLeft => pr.x = Some(em),
                _ => pr.x = Some(-em),
            }
        }
        Pop | ScaleIn => {
            pr.scale = Some(0.0);
            pr.opacity = Some(0.0);
        }
        BlurIn => {
            pr.blur = Some(0.35 * em);
            pr.opacity = Some(0.0);
        }
        Wave => pr.y = Some(-0.25 * em),
        Bounce => {
            pr.y = Some(-em);
            pr.opacity = Some(0.0);
        }
        Spin => {
            pr.rotation = Some(-180.0);
            pr.scale = Some(0.3);
            pr.opacity = Some(0.0);
        }
        Ascend => {
            pr.y = Some(0.5 * em);
            pr.opacity = Some(0.0);
        }
        Shift => {
            pr.x = Some(-0.4 * em);
            pr.opacity = Some(0.0);
        }
        // scramble hides every unit before its start
        Scramble => pr.opacity = Some(0.0),
        Counter | Highlight => {}
        Karaoke => pr.fill = Some(Paint::Solid { rgba: KARAOKE, srgb: true }),
        TrackingIn => {
            // +500 thousandths of an em
            pr.tracking = Some(500.0);
            pr.opacity = Some(0.0);
        }
        MaskReveal => pr.y = Some(1.1 * em),
    }
    // explicit animator values override the preset's (the highlight preset reads @fill as its box paint)
    let o = &a.props;
    macro_rules! over {
        ($($f:ident),*) => { $( if o.$f.is_some() { pr.$f = o.$f.clone(); } )* };
    }
    over!(
        variation,
        x,
        y,
        z_depth,
        scale,
        scale_x,
        scale_y,
        rotation,
        rotation_x,
        rotation_y,
        skew,
        opacity,
        stroke,
        stroke_width,
        tracking,
        line_spacing,
        blur,
        baseline_shift,
        char_offset,
        anchor_x,
        anchor_y
    );
    let highlight = (kind == Highlight).then(|| o.fill.clone().unwrap_or(Paint::Solid { rgba: HIGHLIGHT, srgb: true }));
    if kind != Highlight && o.fill.is_some() {
        pr.fill = o.fill.clone();
    }
    Expanded { unit, amounts, opacity, props: pr, clip: kind == MaskReveal, highlight }
}

/// The text shown by `counter` presets: every number in `text` (digits with thousands
/// commas and decimals) scaled by `k`, keeping its decimals and its commas.
pub fn counter_text(text: &str, k: f64) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            let c = text[i..].chars().next().unwrap_or(' ');
            out.push(c);
            i += c.len_utf8();
            continue;
        }
        let s = i;
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b',') {
            i += 1;
        }
        if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        let num = &text[s..i];
        let dec = num.split_once('.').map(|(_, f)| f.len()).unwrap_or(0);
        let v = num.replace(',', "").parse::<f64>().unwrap_or(0.0) * k;
        // a number authored with leading zeros ("007") keeps its width while it counts
        let int_width = num.split('.').next().map_or(0, str::len);
        let width = if num.starts_with('0') && int_width > 1 && !num.contains(',') {
            int_width + dec + usize::from(dec > 0)
        } else {
            0
        };
        let digits = format!("{v:0width$.dec$}");
        if num.contains(',') {
            let (int, frac) = digits.split_once('.').map(|(a, b)| (a, Some(b))).unwrap_or((&digits, None));
            let (sign, int) = int.strip_prefix('-').map(|x| ("-", x)).unwrap_or(("", int));
            let mut g = String::new();
            for (j, c) in int.chars().enumerate() {
                if j > 0 && (int.len() - j) % 3 == 0 {
                    g.push(',');
                }
                g.push(c);
            }
            out.push_str(sign);
            out.push_str(&g);
            if let Some(f) = frac {
                out.push('.');
                out.push_str(f);
            }
        } else {
            out.push_str(&digits);
        }
    }
    out
}

/// The progress of a `counter` preset at layer time `t`: cubic-out over the duration,
/// times @amount; `None` once the numbers show their own value.
pub fn counter_progress(start: f64, dur: f64, amount: f64, t: f64) -> Option<f64> {
    let e = Ease::CubicOut.at(((t - start) / dur.max(1e-9)).clamp(0.0, 1.0)) * amount / 100.0;
    (e < 1.0).then_some(e)
}

#[derive(Clone)]
struct Acc {
    /// Uniform scale factor (scale), times sx and sy (scaleX, scaleY).
    sc: f64,
    dx: f64,
    dy: f64,
    z: f64,
    sx: f64,
    sy: f64,
    rot: f64,
    rx: f64,
    ry: f64,
    skew: f64,
    op: f64,
    /// Blur radius in text-box pixels.
    blur: f64,
    /// Variation axis targets and how far toward them (selector share).
    var: Vec<([u8; 4], f32)>,
    var_mix: f64,
    fill: Option<(Paint, f64)>,
    stroke: Option<(Paint, f64)>,
    sw: f64,
    tracking: f64,
    line_sp: f64,
    shift: f64,
    char_off: f64,
    ax: f64,
    ay: f64,
    clip_line: bool,
    /// Highlight box behind the unit.
    highlight: Option<crate::glyph::Highlight>,
    /// Pivot: the centre of the glyph's unit box on its line (the last animator that moved it).
    pivot: Option<(f64, f64)>,
}

impl Default for Acc {
    fn default() -> Acc {
        Acc {
            sc: 1.0,
            dx: 0.0,
            dy: 0.0,
            z: 0.0,
            sx: 1.0,
            sy: 1.0,
            rot: 0.0,
            rx: 0.0,
            ry: 0.0,
            skew: 0.0,
            op: 1.0,
            blur: 0.0,
            var: Vec::new(),
            var_mix: 0.0,
            fill: None,
            stroke: None,
            sw: 0.0,
            tracking: 0.0,
            line_sp: 0.0,
            shift: 0.0,
            char_off: 0.0,
            ax: 0.0,
            ay: 0.0,
            clip_line: false,
            highlight: None,
            pivot: None,
        }
    }
}

/// The text shown by a `scramble` preset at layer time `t`: from `start` on, character
/// unit p (in `order`; newlines are not units) stays scrambled until start + (p + 1)·D/M,
/// showing a random letter or digit that changes 20 times per second ([`scramble_char`]).
/// `pick(i)` says whether character i belongs to the animated span.
pub fn scramble_text(
    chars: &[char],
    pick: &dyn Fn(usize) -> bool,
    a: &Animator,
    start: f64,
    dur: f64,
    t: f64,
) -> Vec<char> {
    let mut out = chars.to_vec();
    if t < start {
        return out;
    }
    let (order, order_seed) = match &a.selector {
        Selector::Range { order, seed, .. } => (*order, *seed),
        _ => (Order::Forward, 0),
    };
    let units: Vec<usize> = (0..chars.len()).filter(|&i| chars[i] != '\n' && pick(i)).collect();
    let n = units.len();
    let pos: Vec<f64> = (0..n).map(|k| order_index(k, n, order, order_seed)).collect();
    let slots = pos.iter().fold(0.0f64, |m, p| m.max(*p)) + 1.0;
    let st = dur / slots.max(1.0);
    let tick = libm::floor(t * 20.0) as i64;
    for (k, &i) in units.iter().enumerate() {
        if t < start + (pos[k] + 1.0) * st {
            if let Some(c) = scramble_char(chars[i], a.seed, tick, i) {
                out[i] = c;
            }
        }
    }
    out
}

fn gl_style(lay: &Layout, g: usize) -> usize {
    lay.glyphs[g].style
}

/// Per-glyph effects of `anims` at layer time `t`; `counter` text replaces digits for the counter preset.
/// `roles` gives each span's role.
pub fn apply(
    lib: &FontLib,
    lay: &Layout,
    roles: &[Option<String>],
    anims: &[Animator],
    t: f64,
) -> (Vec<GlyphFx>, Vec<bool>) {
    let n = lay.glyphs.len();
    let mut acc = vec![Acc::default(); n];
    let em = lay.styles.first().map(|s| s.size).unwrap_or(32.0);
    for a in anims {
        let role_fn = a.role.as_ref().map(|r| {
            let r = r.clone();
            move |span: usize| -> bool { roles.get(span).and_then(|x| x.as_deref()) == Some(r.as_str()) }
        });
        let role_ok: Option<&dyn Fn(usize) -> bool> = role_fn.as_ref().map(|f| f as &dyn Fn(usize) -> bool);
        let ex = match a.preset {
            // counters substitute the text before layout
            Some((Preset::Counter, ..)) => continue,
            Some((kind, start, dur)) => {
                let unit = if a.unit_set { a.unit } else { table(kind).0 };
                let (_, cnt) = units(lay, unit, role_ok);
                preset(a, kind, start, dur, cnt, t, em)
            }
            None => {
                let (_, cnt) = units(lay, a.unit, role_ok);
                let v: Vec<f64> = (0..cnt).map(|i| range_amount(&a.selector, i, cnt, t)).collect();
                Expanded {
                    unit: a.unit,
                    amounts: v,
                    opacity: None,
                    props: a.props.clone(),
                    clip: false,
                    highlight: None,
                }
            }
        };
        let (unit, amounts, props, clip) = (ex.unit, &ex.amounts, &ex.props, ex.clip);
        let (map, _) = units(lay, unit, role_ok);
        // unit boxes per line: x extent, and the line box's vertical centre
        let mut boxes: std::collections::HashMap<(usize, usize), (f64, f64, f64)> = Default::default();
        for (g, gl) in lay.glyphs.iter().enumerate() {
            let Some(u) = map[g] else { continue };
            let lr = lay.lines.get(gl.line).map(|l| l.rect).unwrap_or([0.0; 4]);
            let b = boxes.entry((gl.line, u)).or_insert((f64::INFINITY, f64::NEG_INFINITY, lr[1] + lr[3] * 0.5));
            b.0 = b.0.min(gl.x.min(gl.x + gl.advance));
            b.1 = b.1.max(gl.x.max(gl.x + gl.advance));
        }
        for (g, ag) in acc.iter_mut().enumerate() {
            let Some(u) = map[g] else { continue };
            if let Some(b) = boxes.get(&(lay.glyphs[g].line, u)) {
                ag.pivot = Some(((b.0 + b.1) * 0.5, b.2));
            }
            let s = amounts.get(u).copied().unwrap_or(0.0);
            if let Some(hp) = &ex.highlight {
                if s > 1e-4 {
                    ag.highlight = Some(crate::glyph::Highlight {
                        paint: hp.clone(),
                        fraction: s.min(1.0),
                        unit: u,
                        pad: 0.0,
                        radius: 0.0,
                    });
                }
            }
            if clip {
                ag.clip_line = true;
            }
            let s_op = ex.opacity.as_ref().and_then(|o| o.get(u).copied()).unwrap_or(s);
            if s.abs() < 1e-6 && s_op.abs() < 1e-6 {
                continue;
            }
            // combining animators: additive offsets add (replace moves toward the value),
            // factors multiply by 1 + (v − 1)·s (replace moves toward the value)
            let add = |cur: &mut f64, v: Option<f64>| {
                if let Some(v) = v {
                    match a.combine {
                        Combine::Replace => *cur += (v - *cur) * s,
                        Combine::Multiply => *cur *= 1.0 + (v - 1.0) * s,
                        Combine::Add => *cur += v * s,
                    }
                }
            };
            let fac = |cur: &mut f64, v: Option<f64>, s: f64| {
                if let Some(v) = v {
                    match a.combine {
                        Combine::Replace => *cur += (v - *cur) * s,
                        _ => *cur *= 1.0 + (v - 1.0) * s,
                    }
                }
            };
            add(&mut ag.dx, props.x);
            add(&mut ag.dy, props.y);
            add(&mut ag.z, props.z_depth);
            add(&mut ag.rot, props.rotation);
            add(&mut ag.rx, props.rotation_x);
            add(&mut ag.ry, props.rotation_y);
            add(&mut ag.skew, props.skew);
            add(&mut ag.sw, props.stroke_width);
            // thousandths of an em, accumulated along the line below
            add(&mut ag.tracking, props.tracking);
            add(&mut ag.line_sp, props.line_spacing);
            add(&mut ag.shift, props.baseline_shift);
            add(&mut ag.char_off, props.char_offset);
            if props.anchor_x.is_some() {
                ag.ax = props.anchor_x.unwrap_or(0.0);
            }
            if props.anchor_y.is_some() {
                ag.ay = props.anchor_y.unwrap_or(0.0);
            }
            // scale, scaleX and scaleY are factors (5.23); the unit's x scale is scale · scaleX
            fac(&mut ag.sc, props.scale, s);
            fac(&mut ag.sx, props.scale_x, s);
            fac(&mut ag.sy, props.scale_y, s);
            fac(&mut ag.op, props.opacity, s_op);
            if let Some(b) = props.blur {
                ag.blur += (b * s).max(0.0);
            }
            if let Some(v) = &props.variation {
                ag.var = v.clone();
                ag.var_mix = s.clamp(0.0, 1.0);
            }
            if let Some(f) = &props.fill {
                ag.fill = Some((f.clone(), s.clamp(0.0, 1.0)));
            }
            if let Some(f) = &props.stroke {
                ag.stroke = Some((f.clone(), s.clamp(0.0, 1.0)));
            }
        }
    }
    // tracking accumulates along each line, which keeps its alignment
    let mut shift_x = vec![0.0; n];
    for l in &lay.lines {
        let mut run = 0.0;
        for g in l.glyphs.clone() {
            shift_x[g] = run;
            run += acc[g].tracking / 1000.0 * lay.styles[gl_style(lay, g)].size;
        }
        for g in l.glyphs.clone() {
            shift_x[g] -= run * lay.align_shift;
        }
    }
    let mut out = Vec::with_capacity(n);
    for (g, (gl, a)) in lay.glyphs.iter().zip(&acc).enumerate() {
        let (px, py) = a.pivot.unwrap_or((gl.x + gl.advance * 0.5, gl.y));
        let anchor = p(px + a.ax, py + a.ay);
        let persp = 1000.0 / (1000.0 + a.z).max(1.0);
        let sx = a.sc * a.sx * libm::cos(a.ry.to_radians()) * persp;
        let sy = a.sc * a.sy * libm::cos(a.rx.to_radians()) * persp;
        let xf = Xf::translate(anchor.x + a.dx + shift_x[g], anchor.y + a.dy + a.line_sp * gl.line as f64 - a.shift)
            .mul(&Xf::rotate(a.rot))
            .mul(&Xf::skew(a.skew, 0.0))
            .mul(&Xf::scale(sx, sy))
            .mul(&Xf::translate(-anchor.x, -anchor.y));
        let mut fx = GlyphFx {
            xf,
            opacity: a.op.clamp(0.0, 1.0),
            blur: a.blur,
            variation: a.var.clone(),
            variation_mix: a.var_mix,
            ..Default::default()
        };
        if let Some((f, m)) = &a.fill {
            fx.fill = Some(f.clone());
            fx.fill_mix = *m;
        }
        if let Some((f, m)) = &a.stroke {
            fx.stroke = Some(f.clone());
            fx.stroke_mix = *m;
        }
        fx.stroke_width = a.sw;
        fx.highlight = a.highlight.clone();
        let off = a.char_off.round() as i64;
        let ch = lay.chars.get(gl.ch).copied().unwrap_or(' ');
        let ch2 = if off != 0 { Some(offset_char(ch, off)) } else { None };
        if let Some(c2) = ch2.filter(|c| *c != ch) {
            fx.gid = lib.with_face(gl.face, &[], |f| f.glyph_index(c2).map(|x| x.0)).flatten();
        }
        out.push(fx);
    }
    let clips = acc.iter().map(|a| a.clip_line).collect();
    (out, clips)
}

/// Shifts letters and digits within their class.
pub fn offset_char(c: char, off: i64) -> char {
    let cyc = |base: u8, len: i64| -> char { (base + ((c as u8 - base) as i64 + off).rem_euclid(len) as u8) as char };
    match c {
        'a'..='z' => cyc(b'a', 26),
        'A'..='Z' => cyc(b'A', 26),
        '0'..='9' => cyc(b'0', 10),
        _ => c,
    }
}

/// A scrambled character: a letter of the same case or a digit, drawn from the seeded noise hash as
/// pool[⌊U(seed, character index, tick) · |pool|⌋]; other characters are kept.
fn scramble_char(c: char, seed: u64, tick: i64, ci: usize) -> Option<char> {
    let pool: &[u8] = if c.is_uppercase() {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZ"
    } else if c.is_ascii_digit() {
        b"0123456789"
    } else if c.is_alphabetic() {
        b"abcdefghijklmnopqrstuvwxyz"
    } else {
        return None;
    };
    let u = sr_vector::d24::d24_unit(seed, ci as u64, tick as u64);
    Some(pool[((u * pool.len() as f64) as usize).min(pool.len() - 1)] as char)
}

/// Text on a path (`textPath`).
#[derive(Debug, Clone, PartialEq)]
pub struct OnPath {
    pub path: Path,
    pub start_offset: f64,
    pub first_margin: f64,
    pub last_margin: f64,
    pub reverse: bool,
    pub perpendicular: bool,
    pub force_alignment: bool,
}

/// Transforms that move each glyph from its line position onto the path.
pub fn on_path(lay: &Layout, op: &OnPath) -> Vec<Xf> {
    let polys = op.path.flatten(0.05);
    let mut pts: Vec<sr_vector::P> = polys
        .iter()
        .flat_map(|q| {
            let mut v = q.pts.clone();
            if q.closed && !v.is_empty() {
                v.push(v[0]);
            }
            v
        })
        .collect();
    if op.reverse {
        pts.reverse();
    }
    let len: f64 = pts.windows(2).map(|w| w[0].dist(w[1])).sum();
    let base = lay.lines.first().map(|l| l.baseline).unwrap_or(0.0);
    let x0 = lay.lines.first().map(|l| l.rect[0]).unwrap_or(0.0);
    let lw = lay.lines.first().map(|l| l.rect[2]).unwrap_or(1.0).max(1e-9);
    lay.glyphs
        .iter()
        .map(|g| {
            let cx = g.x + g.advance * 0.5 - x0;
            let s = if op.force_alignment {
                op.first_margin + cx / lw * (len - op.first_margin - op.last_margin)
            } else {
                op.start_offset + op.first_margin + cx
            };
            let Some((pt, tg)) = sr_vector::measure::at_length(&pts, s.clamp(0.0, len)) else { return Xf::IDENTITY };
            let ang = if op.perpendicular { tg.angle().to_degrees() } else { 0.0 };
            Xf::translate(pt.x, pt.y).mul(&Xf::rotate(ang)).mul(&Xf::translate(-(g.x + g.advance * 0.5), -base))
        })
        .collect()
}
