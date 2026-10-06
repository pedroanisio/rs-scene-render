//! Keyframe channels: compiled `animate` and `timeRemap` elements.
//!
//! Compilation resolves every key once: key times (marker snapping, roving
//! keys), values parsed with the property's kind, the curve of each
//! segment with its parameters, Hermite tangents for Catmull-Rom and TCB
//! segments, and arc-length tables for spatial (motion-path) segments.
//! Evaluation is then a binary search and one curve evaluation.

use sr_model::model::{Curve, Extrapolation, Key, StepPosition, TimeBase};
use sr_model::values::LengthUnit;

use crate::curve::{self, Ease, KeyParams};
use crate::expr::vm::LoopKind;
use crate::path::MotionPath;
use crate::value::{PropKind, Value};

#[derive(Debug, Clone)]
struct K {
    t: f64,
    v: Value,
    ease: Ease,
    /// Hermite tangents in segment units: outgoing from this key, incoming to it.
    tan_out: Option<Value>,
    tan_in: Option<Value>,
    /// Spatial Bézier for the segment starting at this key.
    spatial: Option<Box<MotionPath>>,
}

/// A compiled keyframe channel.
#[derive(Debug, Clone)]
pub struct Channel {
    keys: Vec<K>,
    before: Extrapolation,
    after: Extrapolation,
    /// Adds to the base value instead of replacing it.
    pub additive: bool,
    /// Time base of key times.
    pub time_base: TimeBase,
}

/// Inputs for compiling a channel.
pub struct ChannelSpec<'a> {
    /// Keys in document order.
    pub keys: &'a [Key],
    /// Curve for keys without `@interpolation`.
    pub default: Curve,
    /// Extrapolation before the first key.
    pub before: Extrapolation,
    /// Extrapolation after the last key.
    pub after: Extrapolation,
    /// Additive animation.
    pub additive: bool,
    /// Time base.
    pub time_base: TimeBase,
    /// Kind of the animated property.
    pub kind: PropKind,
}

/// Resolves style tokens and markers during compilation.
pub trait Lookup {
    /// Colour of style token `name`.
    fn token(&self, name: &str) -> Option<[f64; 4]>;
    /// Time of marker `id`.
    fn marker(&self, id: &str) -> Option<f64>;
}

fn point(p: &Option<sr_model::values::Point2>) -> Option<[f64; 2]> {
    p.map(|p| [p.x, p.y])
}

impl Channel {
    /// Compiles a channel. Errors name the offending key (1-based).
    pub fn compile(spec: &ChannelSpec<'_>, lookup: &dyn Lookup) -> Result<Channel, String> {
        let tok = |n: &str| lookup.token(n);
        let mut keys = Vec::with_capacity(spec.keys.len());
        for (i, k) in spec.keys.iter().enumerate() {
            let mut t = k.time;
            if let Some(m) = &k.marker {
                t += lookup.marker(m).ok_or_else(|| format!("key {}: no marker '{m}'", i + 1))?;
            }
            let v = spec.kind.parse(&k.value, &tok).map_err(|e| format!("key {}: {e}", i + 1))?;
            let bezier = match &k.bezier {
                Some(b) => Some(curve::parse_bezier(b).ok_or_else(|| {
                    format!("key {}: bezier {b:?} must be four numbers x1,y1,x2,y2 with x1 and x2 in [0, 1]", i + 1)
                })?),
                None => None,
            };
            let params = KeyParams {
                bezier,
                ease_out: point(&k.ease_out),
                next_ease_in: spec.keys.get(i + 1).and_then(|n| point(&n.ease_in)),
                steps: k.steps.map(|s| s.min(u32::MAX as u64) as u32),
                step_start: k.step_position == StepPosition::Start,
                spring: [k.stiffness.get(), k.damping.get(), k.mass.get()],
                overshoot: k.overshoot.as_ref().map(|v| v.get()),
                period: k.period.as_ref().map(|v| v.get()),
            };
            let ease = curve::resolve(k.interpolation.unwrap_or(spec.default), &params);
            keys.push(K { t, v, ease, tan_out: None, tan_in: None, spatial: None });
        }
        if keys.is_empty() {
            return Err("an animation needs at least one key".into());
        }
        for w in keys.windows(2) {
            if w[1].t < w[0].t {
                return Err(format!("key times decrease from {} to {}", w[0].t, w[1].t));
            }
        }
        if spec.kind.continuous() {
            for (i, w) in keys.windows(2).enumerate() {
                if !w[0].v.compatible(&w[1].v)
                    && !matches!((&w[0].v, &w[1].v), (Value::PaintRef(_), _) | (_, Value::PaintRef(_)))
                {
                    return Err(format!("keys {} and {} mix units or shapes and cannot interpolate", i + 1, i + 2));
                }
            }
        }
        retime_roving(&mut keys, spec.keys);
        tangents(&mut keys, spec.keys);
        spatial(&mut keys, spec.keys);
        Ok(Channel { keys, before: spec.before, after: spec.after, additive: spec.additive, time_base: spec.time_base })
    }

    /// First and last key time.
    pub fn span(&self) -> (f64, f64) {
        (self.keys[0].t, self.keys[self.keys.len() - 1].t)
    }

    /// Key times after marker offsets and roving-key retiming have been resolved.
    pub fn key_times(&self) -> impl Iterator<Item = f64> + '_ {
        self.keys.iter().map(|k| k.t)
    }

    /// A numeric value's input time on the branch nearest `near`. Bracket within
    /// individual key spans so jumps and nonmonotonic curves are handled too.
    /// Unreachable values use the nearest sampled value; source-history callers
    /// can override the child clock while retaining this composition time.
    pub(crate) fn time_for_value(&self, value: f64, near: f64) -> f64 {
        let at = |t| self.eval(t).as_num().unwrap_or(0.0);
        let mut best = near;
        let mut error = (at(near) - value).abs();
        let mut consider = |t: f64, v: f64| {
            let e = (v - value).abs();
            if e < error - 1e-9 || ((e - error).abs() <= 1e-9 && (t - near).abs() < (best - near).abs()) {
                best = t;
                error = e;
            }
        };
        for key in &self.keys {
            consider(key.t, at(key.t));
        }
        for pair in self.keys.windows(2) {
            let (start, end) = (pair[0].t, pair[1].t);
            let mut left = start;
            let mut a = at(left);
            for step in 1..=128 {
                let right = start + (end - start) * step as f64 / 128.0;
                let b = at(right);
                consider(right, b);
                if (a < value && b > value) || (a > value && b < value) {
                    let (mut lo, mut hi) = (left, right);
                    for _ in 0..48 {
                        let mid = (lo + hi) * 0.5;
                        if (at(mid) < value) == (a < b) {
                            lo = mid;
                        } else {
                            hi = mid;
                        }
                    }
                    let t = (lo + hi) * 0.5;
                    consider(t, at(t));
                    // A step may jump over the requested value. Keep both sides
                    // of its boundary so skipped history maps to the jump.
                    consider(lo, at(lo));
                    consider(hi, at(hi));
                }
                left = right;
                a = b;
            }
        }
        best
    }

    /// Value at time `t` (in the channel's time base).
    pub fn eval(&self, t: f64) -> Value {
        let n = self.keys.len();
        if n == 1 {
            return self.keys[0].v.clone();
        }
        let (t0, tn) = self.span();
        if t < t0 {
            return self.extrapolate(t, self.before, false);
        }
        if t >= tn {
            return self.extrapolate(t, self.after, true);
        }
        self.inside(t)
    }

    fn inside(&self, t: f64) -> Value {
        let n = self.keys.len();
        let i = (self.keys.partition_point(|k| k.t <= t).max(1) - 1).min(n - 2);
        self.segment(i, t)
    }

    fn segment(&self, i: usize, t: f64) -> Value {
        let (a, b) = (&self.keys[i], &self.keys[i + 1]);
        let d = b.t - a.t;
        if d <= 0.0 {
            return b.v.clone();
        }
        let u = ((t - a.t) / d).clamp(0.0, 1.0);
        let e = match a.ease {
            Ease::Spring { stiffness, damping, mass } => curve::spring_segment(t - a.t, d, stiffness, damping, mass),
            Ease::CatmullRom | Ease::Tcb => {
                if let (Some(m0), Some(m1)) = (&a.tan_out, &b.tan_in) {
                    let [h00, h10, h01, h11] = curve::hermite(u);
                    return Value::weighted(&[(&a.v, h00), (m0, h10), (&b.v, h01), (m1, h11)]);
                }
                u
            }
            other => other.apply(u),
        };
        if let Some(sp) = &a.spatial {
            let (p, _) = sp.sample(e, true);
            if let (Value::Pair(pa), Value::Pair(_)) = (&a.v, &b.v) {
                let mut c = *pa;
                c[0].value = p[0];
                c[1].value = p[1];
                // overshooting eases extrapolate along the chord
                if !(0.0..=1.0).contains(&e) {
                    return a.v.lerp(&b.v, e);
                }
                return Value::Pair(c);
            }
        }
        a.v.lerp(&b.v, e)
    }

    fn slope_value(&self, at_end: bool, dt: f64) -> Value {
        // Continue with the velocity at the end key (finite difference).
        let n = self.keys.len();
        let (k, h) = if at_end {
            let k = &self.keys[n - 1];
            (k, -(1e-3f64).min(k.t - self.keys[n - 2].t).max(1e-9))
        } else {
            let k = &self.keys[0];
            (k, (1e-3f64).min(self.keys[1].t - k.t).max(1e-9))
        };
        // a jump has no velocity: a hold ends still, a staircase continues along its chord
        let (a, b) = if at_end { (&self.keys[n - 2], &self.keys[n - 1]) } else { (&self.keys[0], &self.keys[1]) };
        let near = match a.ease {
            Ease::Hold => return k.v.clone(),
            Ease::Steps { .. } if b.t > a.t => a.v.lerp(&b.v, (k.t + h - a.t) / (b.t - a.t)),
            _ => self.inside(k.t + h),
        };
        let w = dt / h;
        Value::weighted(&[(&k.v, 1.0 - w), (&near, w)])
    }

    fn extrapolate(&self, t: f64, mode: Extrapolation, after: bool) -> Value {
        let (t0, tn) = self.span();
        let d = tn - t0;
        let n = self.keys.len();
        match mode {
            Extrapolation::Hold => {
                if after {
                    self.keys[n - 1].v.clone()
                } else {
                    self.keys[0].v.clone()
                }
            }
            Extrapolation::Linear => {
                if after {
                    self.slope_value(true, t - tn)
                } else {
                    self.slope_value(false, t - t0)
                }
            }
            Extrapolation::Loop | Extrapolation::PingPong | Extrapolation::Offset => {
                if d <= 0.0 {
                    return self.keys[if after { n - 1 } else { 0 }].v.clone();
                }
                let x = (t - t0) / d;
                let cycle = libm::floor(x);
                let frac = x - cycle;
                let local = match mode {
                    Extrapolation::PingPong if (cycle as i64).rem_euclid(2) == 1 => tn - frac * d,
                    _ => t0 + frac * d,
                };
                let v = self.inside(local.min(tn - 1e-12 * d.max(1.0)).max(t0));
                if mode == Extrapolation::Offset {
                    let delta = Value::weighted(&[(&self.keys[n - 1].v, cycle), (&self.keys[0].v, -cycle)]);
                    v.add(&delta)
                } else {
                    v
                }
            }
        }
    }

    /// `loopOut`/`loopIn` over the last (first) `count` keys; 0 means all.
    pub fn eval_loop(&self, t: f64, out: bool, kind: LoopKind, count: usize) -> Value {
        let n = self.keys.len();
        if n < 2 {
            return self.eval(t);
        }
        let m = if count == 0 { n } else { count.saturating_add(1).min(n) };
        let (lo, hi) = if out { (n - m, n - 1) } else { (0, m - 1) };
        let (t0, tn) = (self.keys[lo].t, self.keys[hi].t);
        let outside = if out { t > tn } else { t < t0 };
        if !outside || tn <= t0 {
            return self.eval(t);
        }
        if kind == LoopKind::Continue {
            return self.extrapolate(t, Extrapolation::Linear, out);
        }
        let d = tn - t0;
        let x = (t - t0) / d;
        let cycle = libm::floor(x);
        let frac = x - cycle;
        let local =
            if kind == LoopKind::PingPong && (cycle as i64).rem_euclid(2) == 1 { tn - frac * d } else { t0 + frac * d };
        let v = self.inside(local.min(tn - 1e-12 * d.max(1.0)).max(t0));
        if kind == LoopKind::Offset {
            let delta = Value::weighted(&[(&self.keys[hi].v, cycle), (&self.keys[lo].v, -cycle)]);
            v.add(&delta)
        } else {
            v
        }
    }
}

fn distance(a: &Value, b: &Value) -> f64 {
    let (mut x, mut y) = (Vec::new(), Vec::new());
    if a.components(&mut x) && b.components(&mut y) && x.len() == y.len() {
        libm::sqrt(x.iter().zip(&y).map(|(p, q)| (p - q) * (p - q)).sum())
    } else {
        0.0
    }
}

/// Roving keys get times proportional to the value-space distance travelled
/// between the surrounding fixed keys, which gives constant speed.
fn retime_roving(keys: &mut [K], src: &[Key]) {
    let n = keys.len();
    let fixed: Vec<usize> = (0..n).filter(|&i| i == 0 || i == n - 1 || !src[i].roving).collect();
    for w in fixed.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b - a < 2 {
            continue;
        }
        let mut cum = vec![0.0];
        for i in a..b {
            let d = distance(&keys[i].v, &keys[i + 1].v);
            cum.push(cum.last().unwrap() + d);
        }
        let total = *cum.last().unwrap();
        if total <= 0.0 {
            continue;
        }
        let (ta, tb) = (keys[a].t, keys[b].t);
        for i in a + 1..b {
            keys[i].t = ta + (tb - ta) * cum[i - a] / total;
        }
    }
}

fn diff(a: &Value, b: &Value) -> Option<Value> {
    a.compatible(b).then(|| Value::weighted(&[(b, 1.0), (a, -1.0)]))
}

/// Hermite tangents (in segment units) for Catmull-Rom and TCB segments.
fn tangents(keys: &mut [K], src: &[Key]) {
    let n = keys.len();
    if n < 2 || !keys.iter().any(|k| matches!(k.ease, Ease::CatmullRom | Ease::Tcb)) {
        return;
    }
    for i in 0..n {
        let prev = if i > 0 { Some(i - 1) } else { None };
        let next = if i + 1 < n { Some(i + 1) } else { None };
        let d_prev = prev.and_then(|p| diff(&keys[p].v, &keys[i].v));
        let d_next = next.and_then(|q| diff(&keys[i].v, &keys[q].v));
        let (dp, dn) = match (d_prev, d_next) {
            (Some(a), Some(b)) => (a, b),
            (Some(a), None) => (a.clone(), a),
            (None, Some(b)) => (b.clone(), b),
            (None, None) => continue,
        };
        let dt_prev = prev.map(|p| keys[i].t - keys[p].t).unwrap_or(0.0);
        let dt_next = next.map(|q| keys[q].t - keys[i].t).unwrap_or(0.0);
        // tangent shape: TCB parameters of this key; Catmull-Rom is TCB with t = c = b = 0
        let tcb = matches!(keys[i].ease, Ease::Tcb) || prev.is_some_and(|p| matches!(keys[p].ease, Ease::Tcb));
        let (te, co, bi) =
            if tcb { (src[i].tension.get(), src[i].continuity.get(), src[i].bias.get()) } else { (0.0, 0.0, 0.0) };
        let out_w = ((1.0 - te) * (1.0 - co) * (1.0 + bi) / 2.0, (1.0 - te) * (1.0 + co) * (1.0 - bi) / 2.0);
        let in_w = ((1.0 - te) * (1.0 + co) * (1.0 + bi) / 2.0, (1.0 - te) * (1.0 - co) * (1.0 - bi) / 2.0);
        let mut tan_out = Value::weighted(&[(&dp, out_w.0), (&dn, out_w.1)]);
        let mut tan_in = Value::weighted(&[(&dp, in_w.0), (&dn, in_w.1)]);
        // non-uniform key spacing (Kochanek & Bartels 1984, eq. 9)
        if dt_prev > 0.0 && dt_next > 0.0 {
            let s = dt_prev + dt_next;
            tan_out = Value::weighted(&[(&tan_out, 2.0 * dt_next / s)]);
            tan_in = Value::weighted(&[(&tan_in, 2.0 * dt_prev / s)]);
        }
        keys[i].tan_out = Some(tan_out);
        keys[i].tan_in = Some(tan_in);
    }
}

/// Spatial Bézier segments for pairs with spatial tangents (pixels only).
fn spatial(keys: &mut [K], src: &[Key]) {
    for i in 0..keys.len().saturating_sub(1) {
        let (so, si) = (point(&src[i].spatial_out), point(&src[i + 1].spatial_in));
        if so.is_none() && si.is_none() {
            continue;
        }
        let (Value::Pair(a), Value::Pair(b)) = (&keys[i].v, &keys[i + 1].v) else { continue };
        if a.iter().chain(b.iter()).any(|l| l.unit != LengthUnit::Px) {
            continue;
        }
        let so = so.unwrap_or([0.0, 0.0]);
        let si = si.unwrap_or([0.0, 0.0]);
        let d = format!(
            "M{} {} C{} {} {} {} {} {}",
            a[0].value,
            a[1].value,
            a[0].value + so[0],
            a[1].value + so[1],
            b[0].value + si[0],
            b[1].value + si[1],
            b[0].value,
            b[1].value
        );
        keys[i].spatial = MotionPath::parse(&d).ok().map(Box::new);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sr_model::LoadOptions;

    struct NoLookup;
    impl Lookup for NoLookup {
        fn token(&self, _: &str) -> Option<[f64; 4]> {
            None
        }
        fn marker(&self, id: &str) -> Option<f64> {
            (id == "m").then_some(10.0)
        }
    }

    fn keys(xml_keys: &str) -> Vec<Key> {
        let xml = format!(
            "<scene version=\"1.1\"><project width=\"10\" height=\"10\" fps=\"30\" duration=\"5\"/>\
             <markers><marker id=\"m\" time=\"10\"/></markers><composition><shape id=\"s\" shape=\"rect\" width=\"1\" height=\"1\">\
             <animate property=\"x\">{xml_keys}</animate></shape></composition></scene>"
        );
        let doc = sr_model::load_str(&xml, &LoadOptions::without_assets()).unwrap();
        let sr_model::model::Node::Shape(s) = &doc.scene.composition.children[0] else { panic!() };
        match &s.children[0] {
            sr_model::model::ShapeChild::NodeBehaviour(sr_model::model::NodeBehaviour::Animation(
                sr_model::model::Animation::Animate(a),
            )) => a.keys.clone(),
            _ => panic!(),
        }
    }

    fn chan(k: &[Key], kind: PropKind, before: Extrapolation, after: Extrapolation) -> Channel {
        Channel::compile(
            &ChannelSpec {
                keys: k,
                default: Curve::Linear,
                before,
                after,
                additive: false,
                time_base: TimeBase::Composition,
                kind,
            },
            &NoLookup,
        )
        .unwrap()
    }

    fn num(v: Value) -> f64 {
        v.as_num().unwrap()
    }

    const N: PropKind = PropKind::Number(crate::value::Range { lo: None, hi: None, integer: false });

    #[test]
    fn linear_hold_and_extrapolation() {
        let k = keys(
            r#"<key time="0" value="0"/><key time="1" value="10"/><key time="2" value="10" interpolation="hold"/><key time="3" value="30"/>"#,
        );
        let c = chan(&k, N, Extrapolation::Hold, Extrapolation::Hold);
        assert_eq!(num(c.eval(0.5)), 5.0);
        assert_eq!(num(c.eval(2.9)), 10.0);
        assert_eq!(num(c.eval(3.0)), 30.0);
        assert_eq!(num(c.eval(-1.0)), 0.0);
        let l = chan(&k, N, Extrapolation::Linear, Extrapolation::Loop);
        assert!((num(l.eval(-1.0)) + 10.0).abs() < 1e-6, "{}", num(l.eval(-1.0)));
        assert!((num(l.eval(3.5)) - num(l.eval(0.5))).abs() < 1e-9);
        let p = chan(&k, N, Extrapolation::Hold, Extrapolation::PingPong);
        assert!((num(p.eval(3.5)) - num(p.eval(2.5))).abs() < 1e-9);
        let o = chan(&k, N, Extrapolation::Hold, Extrapolation::Offset);
        assert!((num(o.eval(3.5)) - (num(o.eval(0.5)) + 30.0)).abs() < 1e-9);
        assert!((num(c.eval_loop(4.5, true, LoopKind::Cycle, 1)) - num(c.eval(2.5))).abs() < 1e-9);
    }

    #[test]
    fn loops_over_odd_counts_and_tiny_spans() {
        let k = keys(r#"<key time="0" value="0"/><key time="1" value="10"/><key time="2" value="30"/>"#);
        let c = chan(&k, N, Extrapolation::Hold, Extrapolation::Hold);
        // loopOut("cycle", Infinity) reaches here as usize::MAX: all keys
        let all = num(c.eval_loop(2.5, true, LoopKind::Cycle, 0));
        assert_eq!(num(c.eval_loop(2.5, true, LoopKind::Cycle, usize::MAX)), all);
        assert_eq!(num(c.eval_loop(-0.5, false, LoopKind::Cycle, usize::MAX)), num(c.eval(1.5)));
        let k = keys(r#"<key time="0" value="0"/><key time="0.0000000000001" value="10"/>"#);
        let c = chan(&k, N, Extrapolation::Hold, Extrapolation::Hold);
        for kind in [LoopKind::Cycle, LoopKind::PingPong, LoopKind::Offset, LoopKind::Continue] {
            assert!(num(c.eval_loop(1.0, true, kind, 0)).is_finite(), "{kind:?}");
        }
        let c = chan(&k, N, Extrapolation::Loop, Extrapolation::Loop);
        assert!(num(c.eval(1.0)).is_finite() && num(c.eval(-1.0)).is_finite());
    }

    #[test]
    fn linear_extrapolation_past_a_jump() {
        // a hold segment ends still; a staircase continues along its chord
        let k = keys(
            r#"<key time="0" value="0"/><key time="1" value="10" interpolation="hold"/><key time="2" value="20"/>"#,
        );
        let c = chan(&k, N, Extrapolation::Linear, Extrapolation::Linear);
        assert_eq!(num(c.eval(3.0)), 20.0);
        assert_eq!(num(c.eval_loop(3.0, true, LoopKind::Continue, 0)), 20.0);
        let k = keys(
            r#"<key time="0" value="0"/><key time="1" value="10" interpolation="steps" steps="4"/><key time="2" value="20"/>"#,
        );
        let c = chan(&k, N, Extrapolation::Linear, Extrapolation::Linear);
        assert!((num(c.eval(3.0)) - 30.0).abs() < 1e-9, "{}", num(c.eval(3.0)));
        let k = keys(
            r#"<key time="0" value="0" interpolation="hold"/><key time="1" value="10"/><key time="2" value="20"/>"#,
        );
        let c = chan(&k, N, Extrapolation::Linear, Extrapolation::Linear);
        assert_eq!(num(c.eval(-1.0)), 0.0);
        assert!((num(c.eval(3.0)) - 30.0).abs() < 1e-6, "smooth ends keep their velocity");
    }

    #[test]
    fn curves_markers_and_splines() {
        let k = keys(r#"<key time="0" value="0" interpolation="ease-in-out"/><key time="0" marker="m" value="100"/>"#);
        let c = chan(&k, N, Extrapolation::Hold, Extrapolation::Hold);
        assert_eq!(c.span(), (0.0, 10.0));
        assert!((num(c.eval(5.0)) - 50.0).abs() < 1e-9);
        assert!(num(c.eval(2.0)) < 20.0);
        let cr = keys(
            r#"<key time="0" value="0" interpolation="catmull-rom"/><key time="1" value="10" interpolation="catmull-rom"/><key time="2" value="0"/>"#,
        );
        let c = chan(&cr, N, Extrapolation::Hold, Extrapolation::Hold);
        assert!(num(c.eval(0.9)) > 9.0 && num(c.eval(1.0)) == 10.0);
        assert!(num(c.eval(1.1)) > 9.0, "smooth through the peak");
        let sp = keys(r#"<key time="0" value="0" interpolation="spring"/><key time="2" value="100"/>"#);
        let c = chan(&sp, N, Extrapolation::Hold, Extrapolation::Hold);
        assert!(num(c.eval(0.4)) > 100.0, "spring overshoots: {}", num(c.eval(0.4)));
        assert_eq!(num(c.eval(2.0)), 100.0);
    }

    #[test]
    fn roving_and_spatial() {
        let k =
            keys(r#"<key time="0" value="0"/><key time="0.1" value="90" roving="true"/><key time="1" value="100"/>"#);
        let c = chan(&k, N, Extrapolation::Hold, Extrapolation::Hold);
        assert!((num(c.eval(0.5)) - 50.0).abs() < 1e-9, "constant speed through a roving key");
    }

    #[test]
    fn errors_are_located() {
        let k =
            keys(r#"<key time="0" value="0" interpolation="cubic-bezier" bezier="2,0,0,1"/><key time="1" value="1"/>"#);
        let e = Channel::compile(
            &ChannelSpec {
                keys: &k,
                default: Curve::Linear,
                before: Extrapolation::Hold,
                after: Extrapolation::Hold,
                additive: false,
                time_base: TimeBase::Composition,
                kind: N,
            },
            &NoLookup,
        )
        .unwrap_err();
        assert!(e.contains("key 1") && e.contains("x1 and x2"), "{e}");
        let k = keys(r#"<key time="0" value="10%"/><key time="1" value="10"/>"#);
        let e = Channel::compile(
            &ChannelSpec {
                keys: &k,
                default: Curve::Linear,
                before: Extrapolation::Hold,
                after: Extrapolation::Hold,
                additive: false,
                time_base: TimeBase::Composition,
                kind: PropKind::Length { positive: false },
            },
            &NoLookup,
        )
        .unwrap_err();
        assert!(e.contains("mix units"), "{e}");
    }
}
