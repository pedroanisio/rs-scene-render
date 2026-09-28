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
    /// Percent (100 = unchanged).
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
    /// Restrict to the span with this role.
    pub role: Option<String>,
    pub selector: Selector,
    pub props: Props,
    pub combine: Combine,
    /// Preset with its start and duration (seconds on the layer's timeline).
    pub preset: Option<(Preset, f64, f64)>,
    /// Preset per-unit delay, seconds.
    pub stagger: Option<f64>,
    /// Preset overlap between units, 0–1.
    pub overlap: f64,
    pub seed: u64,
}

impl Default for Animator {
    fn default() -> Animator {
        Animator {
            unit: Unit::Char,
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
            overlap: 0.0,
            seed: 0,
        }
    }
}

fn hash(seed: u64, a: u64, b: u64) -> f64 {
    let mut z = seed ^ a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
}

fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn ease_out_back(x: f64) -> f64 {
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (x - 1.0).powi(3) + c1 * (x - 1.0).powi(2)
}

fn ease_out_bounce(x: f64) -> f64 {
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

fn order_index(i: usize, n: usize, order: Order, seed: u64) -> f64 {
    let c = (n as f64 - 1.0) * 0.5;
    match order {
        Order::Forward => i as f64,
        Order::Reverse => (n - 1 - i) as f64,
        Order::CenterOut => (i as f64 - c).abs() * 2.0 * (n as f64 - 1.0) / (n as f64 - 1.0).max(1.0) * 0.5,
        Order::EdgesIn => (c - (i as f64 - c).abs()) * 2.0 * 0.5,
        Order::Random => {
            let mut v: Vec<(f64, usize)> = (0..n).map(|k| (hash(seed, k as u64, 7), k)).collect();
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            v.iter().position(|x| x.1 == i).unwrap_or(i) as f64
        }
    }
}

fn range_amount(sel: &Selector, i: usize, n: usize, t: f64) -> f64 {
    match sel {
        Selector::Values(v) => v.get(i).copied().unwrap_or(0.0) / 100.0,
        Selector::Wiggly { amount, rate, seed } => {
            sr_vector::modifiers::smooth_noise(*seed, i as u64, t * rate) * amount / 100.0
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

/// Expands a preset: per-unit amounts, properties and unit.
fn preset(
    a: &Animator,
    kind: Preset,
    start: f64,
    dur: f64,
    n: usize,
    t: f64,
    em: f64,
) -> (Unit, Vec<f64>, Props, bool) {
    use Preset::*;
    let dur = dur.max(1e-6);
    let base_unit = match kind {
        WordByWord | Ascend | Highlight => Unit::Word,
        LineByLine | MaskReveal => Unit::Line,
        _ => Unit::Char,
    };
    let unit = if a.unit != Unit::Char { a.unit } else { base_unit };
    let default_overlap = match kind {
        Typewriter | WordByWord => 0.0,
        FadeIn | FadeOut | ScaleIn | BlurIn => 0.5,
        TrackingIn => 1.0,
        LineByLine => 0.3,
        _ => 0.35,
    };
    let overlap = if a.overlap > 0.0 { a.overlap } else { default_overlap };
    let nf = n.max(1) as f64;
    let progress = |i: usize| -> f64 {
        match a.stagger {
            Some(st) => ((t - start - i as f64 * st) / dur).clamp(0.0, 1.0),
            None => {
                let pp = (t - start) / dur;
                let w = 1.0 + overlap * (nf - 1.0);
                ((pp * (nf - 1.0 + w) - i as f64) / w).clamp(0.0, 1.0)
            }
        }
    };
    let hard = matches!(kind, Typewriter | WordByWord);
    let amounts: Vec<f64> = (0..n)
        .map(|i| {
            let pi = progress(i);
            match kind {
                FadeOut => smooth(pi),
                Wave => {
                    let pp = (t - start) / dur;
                    if (0.0..=1.0).contains(&pp) {
                        libm::sin(std::f64::consts::TAU * (pp * 2.0 - i as f64 / nf))
                    } else {
                        0.0
                    }
                }
                Pop => 1.0 - ease_out_back(pi),
                Bounce => 1.0 - ease_out_bounce(pi),
                Karaoke | Highlight => {
                    if matches!(kind, Highlight) {
                        // the current unit only
                        let pp = ((t - start) / dur).clamp(0.0, 1.0) * nf;
                        let d = (pp - (i as f64 + 0.5)).abs();
                        (1.0 - d).clamp(0.0, 1.0)
                    } else {
                        pi
                    }
                }
                _ if hard => {
                    if pi < 1.0 {
                        1.0
                    } else {
                        0.0
                    }
                }
                _ => 1.0 - smooth(pi),
            }
        })
        .collect();
    let mut pr = Props::default();
    match kind {
        Typewriter | FadeIn | FadeOut | WordByWord | LetterByLetter => pr.opacity = Some(0.0),
        LineByLine => {
            pr.opacity = Some(0.0);
            pr.y = Some(0.3 * em);
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
        Pop => pr.scale = Some(0.0),
        ScaleIn => {
            pr.scale = Some(0.0);
            pr.opacity = Some(0.0);
        }
        BlurIn => {
            pr.blur = Some(12.0);
            pr.opacity = Some(0.0);
        }
        Wave => pr.y = Some(-0.25 * em),
        Bounce => pr.y = Some(-em),
        Spin => {
            pr.rotation = Some(-180.0);
            pr.scale = Some(50.0);
            pr.opacity = Some(0.0);
        }
        Ascend => {
            pr.y = Some(em);
            pr.opacity = Some(0.0);
        }
        Shift => {
            pr.x = Some(-0.5 * em);
            pr.opacity = Some(0.0);
        }
        Scramble => pr.char_offset = Some(1.0),
        Counter => {}
        Karaoke => {
            pr.fill = Some(a.props.fill.clone().unwrap_or(Paint::Solid { rgba: [1.0, 0.83, 0.0, 1.0], srgb: true }))
        }
        Highlight => {
            pr.fill = Some(a.props.fill.clone().unwrap_or(Paint::Solid { rgba: [1.0, 0.83, 0.0, 1.0], srgb: true }));
            pr.scale = Some(110.0);
        }
        TrackingIn => {
            pr.tracking = Some(0.5 * em);
            pr.opacity = Some(0.0);
        }
        MaskReveal => pr.y = Some(1.2 * em),
    }
    // explicit animator values override the preset's
    let o = &a.props;
    macro_rules! over {
        ($($f:ident),*) => { $( if o.$f.is_some() { pr.$f = o.$f.clone(); } )* };
    }
    over!(
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
    (unit, amounts, pr, matches!(kind, MaskReveal))
}

#[derive(Clone)]
struct Acc {
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
}

impl Default for Acc {
    fn default() -> Acc {
        Acc {
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
        }
    }
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
    let mut counter: Option<(f64, f64)> = None;
    for a in anims {
        let role_fn = a.role.as_ref().map(|r| {
            let r = r.clone();
            move |span: usize| -> bool { roles.get(span).and_then(|x| x.as_deref()) == Some(r.as_str()) }
        });
        let role_ok: Option<&dyn Fn(usize) -> bool> = role_fn.as_ref().map(|f| f as &dyn Fn(usize) -> bool);
        let (unit, amounts, props, clip) = match a.preset {
            Some((kind, start, dur)) => {
                let (u0, _, _, _) = preset(a, kind, start, dur, 1, t, em);
                let (_, cnt) = units(lay, u0, role_ok);
                if kind == Preset::Counter {
                    let pp = ((t - start) / dur.max(1e-6)).clamp(0.0, 1.0);
                    counter = Some((smooth(pp), 0.0));
                }
                preset(a, kind, start, dur, cnt, t, em)
            }
            None => {
                let (_, cnt) = units(lay, a.unit, role_ok);
                let v: Vec<f64> = (0..cnt).map(|i| range_amount(&a.selector, i, cnt, t)).collect();
                (a.unit, v, a.props.clone(), false)
            }
        };
        let (map, _) = units(lay, unit, role_ok);
        for (g, ag) in acc.iter_mut().enumerate() {
            let Some(u) = map[g] else { continue };
            let s = amounts.get(u).copied().unwrap_or(0.0);
            if s == 0.0 && !clip {
                continue;
            }
            let add = |cur: &mut f64, v: Option<f64>| {
                if let Some(v) = v {
                    match a.combine {
                        Combine::Replace => *cur = v * s,
                        _ => *cur += v * s,
                    }
                }
            };
            let fac = |cur: &mut f64, v: Option<f64>, unit_scale: f64| {
                if let Some(v) = v {
                    let f = 1.0 + (v / unit_scale - 1.0) * s;
                    match a.combine {
                        Combine::Add => *cur += f - 1.0,
                        Combine::Multiply => *cur *= f,
                        Combine::Replace => *cur = f,
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
            fac(&mut ag.sx, props.scale.or(props.scale_x), 100.0);
            fac(&mut ag.sy, props.scale.or(props.scale_y), 100.0);
            fac(&mut ag.op, props.opacity, 1.0);
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
            if clip {
                ag.clip_line = true;
            }
        }
    }
    // tracking accumulates along each line, spreading from the line centre
    let mut shift_x = vec![0.0; n];
    for l in &lay.lines {
        let mut run = 0.0;
        for g in l.glyphs.clone() {
            shift_x[g] = run;
            run += acc[g].tracking;
        }
        for g in l.glyphs.clone() {
            shift_x[g] -= run * 0.5;
        }
    }
    let mut out = Vec::with_capacity(n);
    for (g, (gl, a)) in lay.glyphs.iter().zip(&acc).enumerate() {
        let anchor = p(gl.x + gl.advance * 0.5 + a.ax, gl.y + a.ay);
        let persp = 1000.0 / (1000.0 + a.z).max(1.0);
        let sx = a.sx * libm::cos(a.ry.to_radians()) * persp;
        let sy = a.sy * libm::cos(a.rx.to_radians()) * persp;
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
        let off = a.char_off.round() as i64;
        let ch = lay.chars.get(gl.ch).copied().unwrap_or(' ');
        let ch2 = if let Some((k, _)) = counter {
            counter_char(lay, gl.ch, k)
        } else if off != 0 {
            Some(offset_char(ch, off))
        } else {
            None
        };
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

/// The digit shown at character `ci` while the first number in the text counts up to its value.
fn counter_char(lay: &Layout, ci: usize, k: f64) -> Option<char> {
    let chars = &lay.chars;
    let start = chars.iter().position(|c| c.is_ascii_digit())?;
    let mut end = start;
    while end < chars.len()
        && (chars[end].is_ascii_digit()
            || ((chars[end] == ',' || chars[end] == '.') && chars.get(end + 1).is_some_and(|c| c.is_ascii_digit())))
    {
        end += 1;
    }
    if ci < start || ci >= end || !chars[ci].is_ascii_digit() {
        return None;
    }
    let digits: String = chars[start..end].iter().filter(|c| c.is_ascii_digit()).collect();
    let value: f64 = digits.parse().ok()?;
    let shown = format!("{:0width$}", (value * k).round() as u64, width = digits.len());
    let pos = chars[start..=ci].iter().filter(|c| c.is_ascii_digit()).count() - 1;
    shown.chars().nth(pos)
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
