//! Animated property values and their kinds.
//!
//! The kind of a property comes from the XSD type of the attribute it
//! animates: numbers carry the facet range of their type (so an overshooting
//! `back-out` ease cannot drive `opacity` above 1), lengths keep their unit,
//! colours are straight RGBA in the working space, and strings, booleans and
//! paint references are discrete.

use std::sync::Arc;

use sr_model::element::AttrValue;
use sr_model::parse::ParseValue;
use sr_model::values::{Color, Length, LengthUnit, Paint};
use sr_model::xsd::{self, Builtin, SimpleKind, SIMPLE_TYPES};

use crate::expr::vm::V;

/// A property value.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(untagged)]
pub enum Value {
    /// Number.
    Num(f64),
    /// Boolean.
    Bool(bool),
    /// String or enumeration literal.
    Str(Arc<str>),
    /// Length with unit.
    Len(#[serde(serialize_with = "ser_len")] Length),
    /// Two lengths: position, anchor, scale, skew, points.
    Pair(#[serde(serialize_with = "ser_pair")] [Length; 2]),
    /// Straight-alpha RGBA in [0, 1].
    Color([f64; 4]),
    /// `url(#id)` paint reference.
    PaintRef(Arc<str>),
    /// Number list.
    List(Arc<[f64]>),
}

fn ser_len<S: serde::Serializer>(l: &Length, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(l)
}

fn ser_pair<S: serde::Serializer>(p: &[Length; 2], s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeTuple;
    let mut t = s.serialize_tuple(2)?;
    t.serialize_element(&p[0].to_string())?;
    t.serialize_element(&p[1].to_string())?;
    t.end()
}

/// Numeric range from XSD facets.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Range {
    /// Lower bound and whether it is exclusive.
    pub lo: Option<(f64, bool)>,
    /// Upper bound and whether it is exclusive.
    pub hi: Option<(f64, bool)>,
    /// Integer-valued type.
    pub integer: bool,
}

/// Kind of an animatable property.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropKind {
    /// Number within a range.
    Number(Range),
    /// Length; `positive` for positiveLengthType.
    Length {
        /// Must be > 0.
        positive: bool,
    },
    /// Pair of lengths or numbers.
    Pair,
    /// Colour.
    Color,
    /// Colour or paint reference.
    Paint,
    /// Boolean.
    Bool,
    /// Discrete string.
    Str,
    /// Number list.
    List,
}

fn range_of(mut ty: usize) -> Range {
    let mut r = Range::default();
    loop {
        match SIMPLE_TYPES[ty].kind {
            SimpleKind::Restriction(x) => {
                if r.lo.is_none() {
                    r.lo = x.min_inclusive.map(|v| (v, false)).or(x.min_exclusive.map(|v| (v, true)));
                }
                if r.hi.is_none() {
                    r.hi = x.max_inclusive.map(|v| (v, false)).or(x.max_exclusive.map(|v| (v, true)));
                }
                ty = x.base;
            }
            SimpleKind::Builtin(b) => {
                match b {
                    Builtin::Double => {}
                    Builtin::PositiveInteger => {
                        r.integer = true;
                        r.lo = r.lo.or(Some((1.0, false)));
                    }
                    Builtin::NonNegativeInteger | Builtin::UnsignedLong => {
                        r.integer = true;
                        r.lo = r.lo.or(Some((0.0, false)));
                    }
                    _ => r.integer = true,
                }
                return r;
            }
            _ => return r,
        }
    }
}

impl PropKind {
    /// Kind of an attribute of simple type `ty` (index into `SIMPLE_TYPES`).
    pub fn of_simple_type(ty: usize) -> PropKind {
        match SIMPLE_TYPES[ty].name {
            "lengthType" | "relativeLength" => return PropKind::Length { positive: false },
            "positiveLengthType" | "positiveRelativeLength" => return PropKind::Length { positive: true },
            "colorType" => return PropKind::Color,
            "paintType" | "paintRefType" => return PropKind::Paint,
            "pointType" => return PropKind::Pair,
            "numberListType" => return PropKind::List,
            _ => {}
        }
        if let SimpleKind::Restriction(r) = SIMPLE_TYPES[ty].kind {
            if !r.enums.is_empty() {
                return PropKind::Str;
            }
        }
        match xsd::root_builtin(ty) {
            Some(
                Builtin::Double
                | Builtin::Integer
                | Builtin::Int
                | Builtin::NonNegativeInteger
                | Builtin::PositiveInteger
                | Builtin::UnsignedLong,
            ) => PropKind::Number(range_of(ty)),
            Some(Builtin::Boolean) => PropKind::Bool,
            _ => PropKind::Str,
        }
    }

    /// True when values of this kind interpolate continuously.
    pub fn continuous(self) -> bool {
        matches!(
            self,
            PropKind::Number(_)
                | PropKind::Length { .. }
                | PropKind::Pair
                | PropKind::Color
                | PropKind::Paint
                | PropKind::List
        )
    }

    /// Parses a key value. `token` resolves `var(--name)` colours.
    pub fn parse(self, raw: &str, token: &dyn Fn(&str) -> Option<[f64; 4]>) -> Result<Value, String> {
        let raw_t = raw.trim();
        match self {
            PropKind::Number(_) => {
                sr_model::xsd::parse_xsd_double(raw_t).map(Value::Num).ok_or_else(|| format!("{raw:?} is not a number"))
            }
            PropKind::Length { .. } => Length::parse_value(raw_t).map(Value::Len).map_err(|e| e.to_string()),
            PropKind::Pair => {
                let parts: Vec<&str> = raw_t.split([',', ' ']).filter(|p| !p.is_empty()).collect();
                if parts.len() != 2 {
                    return Err(format!("{raw:?} is not an x,y pair"));
                }
                let a = Length::parse_value(parts[0]).map_err(|e| e.to_string())?;
                let b = Length::parse_value(parts[1]).map_err(|e| e.to_string())?;
                Ok(Value::Pair([a, b]))
            }
            PropKind::Color => color(raw_t, token),
            PropKind::Paint => {
                if let Some(id) = raw_t.strip_prefix("url(#").and_then(|r| r.strip_suffix(')')) {
                    Ok(Value::PaintRef(id.into()))
                } else {
                    color(raw_t, token)
                }
            }
            PropKind::Bool => match raw_t {
                "true" | "1" => Ok(Value::Bool(true)),
                "false" | "0" => Ok(Value::Bool(false)),
                _ => Err(format!("{raw:?} is not a boolean")),
            },
            PropKind::Str => Ok(Value::Str(raw.into())),
            PropKind::List => raw_t
                .split([' ', ','])
                .filter(|t| !t.is_empty())
                .map(|t| sr_model::xsd::parse_xsd_double(t).ok_or_else(|| format!("{t:?} is not a number")))
                .collect::<Result<Vec<f64>, _>>()
                .map(|v| Value::List(v.into())),
        }
    }

    /// Converts a static attribute value.
    pub fn from_attr(self, a: &AttrValue, token: &dyn Fn(&str) -> Option<[f64; 4]>) -> Value {
        match (self, a) {
            (_, AttrValue::Num(n)) => Value::Num(*n),
            (_, AttrValue::Bool(b)) => Value::Bool(*b),
            (_, AttrValue::Length(l)) => Value::Len(*l),
            (_, AttrValue::Point(p)) => Value::Pair([Length::px(p.x), Length::px(p.y)]),
            (_, AttrValue::Color(c)) => color_value(c, token).unwrap_or(Value::Color([0.0, 0.0, 0.0, 0.0])),
            (_, AttrValue::Paint(Paint::Color(c))) => {
                color_value(c, token).unwrap_or(Value::Color([0.0, 0.0, 0.0, 0.0]))
            }
            (_, AttrValue::Paint(Paint::Ref(r))) => Value::PaintRef(r.0.as_str().into()),
            (_, AttrValue::Numbers(v)) => Value::List(v.clone().into()),
            (PropKind::Pair, AttrValue::Str(s)) => self.parse(s, token).unwrap_or(self.neutral()),
            (_, AttrValue::Str(s)) => Value::Str(s.as_str().into()),
            (_, AttrValue::Tokens(t)) => Value::Str(t.join(" ").into()),
        }
    }

    /// Value used when an optional attribute without default is absent.
    pub fn neutral(self) -> Value {
        match self {
            PropKind::Number(r) => Value::Num(match r.lo {
                Some((lo, _)) if lo > 0.0 => lo,
                _ => 0.0,
            }),
            PropKind::Length { .. } => Value::Len(Length::px(0.0)),
            PropKind::Pair => Value::Pair([Length::px(0.0); 2]),
            PropKind::Color | PropKind::Paint => Value::Color([0.0, 0.0, 0.0, 0.0]),
            PropKind::Bool => Value::Bool(false),
            PropKind::Str => Value::Str("".into()),
            PropKind::List => Value::List(Arc::from(Vec::new())),
        }
    }

    /// Clamps a value into the range of its type.
    pub fn clamp(self, v: Value) -> Value {
        match (self, v) {
            (PropKind::Number(r), Value::Num(mut x)) => {
                if x.is_nan() {
                    return Value::Num(x);
                }
                if let Some((lo, excl)) = r.lo {
                    let lo = if excl { lo + f64::EPSILON * lo.abs().max(1.0) } else { lo };
                    x = x.max(lo);
                }
                if let Some((hi, excl)) = r.hi {
                    let hi = if excl { hi - f64::EPSILON * hi.abs().max(1.0) } else { hi };
                    x = x.min(hi);
                }
                if r.integer {
                    x = libm::floor(x + 0.5);
                }
                Value::Num(x)
            }
            (PropKind::Length { positive: true }, Value::Len(l)) => {
                Value::Len(Length { value: l.value.max(1e-9), ..l })
            }
            (PropKind::Color | PropKind::Paint, Value::Color(c)) => Value::Color(c.map(|x| x.clamp(0.0, 1.0))),
            (_, v) => v,
        }
    }

    /// Converts an expression result, using `template` for units and shape.
    /// Returns `None` when the result cannot represent this kind.
    pub fn from_v(self, v: &V, template: &Value) -> Option<Value> {
        let finite = |x: f64| x.is_finite().then_some(x);
        Some(match self {
            PropKind::Number(_) => Value::Num(finite(v.num())?),
            PropKind::Length { .. } => {
                let unit = if let Value::Len(l) = template { l.unit } else { LengthUnit::Px };
                Value::Len(Length { value: finite(v.num())?, unit })
            }
            PropKind::Pair => {
                let c = v.components()?;
                let units = if let Value::Pair(p) = template { [p[0].unit, p[1].unit] } else { [LengthUnit::Px; 2] };
                let (x, y) = match c.len() {
                    1 => (c[0], c[0]),
                    n if n >= 2 => (c[0], c[1]),
                    _ => return None,
                };
                Value::Pair([
                    Length { value: finite(x)?, unit: units[0] },
                    Length { value: finite(y)?, unit: units[1] },
                ])
            }
            PropKind::Color | PropKind::Paint => match v {
                V::Str(s) => self.parse(s, &|_| None).ok()?,
                _ => {
                    let c = v.components()?;
                    if c.len() < 3 || c.iter().any(|x| !x.is_finite()) {
                        return None;
                    }
                    Value::Color([c[0], c[1], c[2], c.get(3).copied().unwrap_or(1.0)])
                }
            },
            PropKind::Bool => Value::Bool(v.truthy()),
            PropKind::Str => Value::Str(v.to_js_string().into()),
            PropKind::List => Value::List(v.components()?.into()),
        })
    }
}

fn color_value(c: &Color, token: &dyn Fn(&str) -> Option<[f64; 4]>) -> Option<Value> {
    match c {
        Color::Rgba(r) => Some(Value::Color([r.r as f64, r.g as f64, r.b as f64, r.a as f64])),
        Color::Token(t) => token(t).map(Value::Color),
    }
}

fn color(raw: &str, token: &dyn Fn(&str) -> Option<[f64; 4]>) -> Result<Value, String> {
    let c = Color::parse_value(raw).map_err(|e| e.to_string())?;
    color_value(&c, token).ok_or_else(|| format!("{raw}: unknown style token"))
}

impl Value {
    /// Expression-language view: lengths become their magnitude, pairs and
    /// colours become arrays.
    pub fn to_v(&self) -> V {
        match self {
            Value::Num(n) => V::Num(*n),
            Value::Bool(b) => V::Bool(*b),
            Value::Str(s) => V::Str(s.clone()),
            Value::Len(l) => V::Num(l.value),
            Value::Pair(p) => V::nums(&[p[0].value, p[1].value]),
            Value::Color(c) => V::nums(c),
            Value::PaintRef(r) => V::Str(format!("url(#{r})").into()),
            Value::List(l) => V::nums(l),
        }
    }

    /// Number, when this is numeric.
    pub fn as_num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            Value::Len(l) => Some(l.value),
            _ => None,
        }
    }

    /// Numeric components (lengths contribute their magnitude).
    pub fn components(&self, out: &mut Vec<f64>) -> bool {
        out.clear();
        match self {
            Value::Num(n) => out.push(*n),
            Value::Len(l) => out.push(l.value),
            Value::Pair(p) => out.extend([p[0].value, p[1].value]),
            Value::Color(c) => out.extend(c),
            Value::List(l) => out.extend(l.iter()),
            _ => return false,
        }
        true
    }

    /// A value shaped like `self` with new components.
    pub fn with_components(&self, c: &[f64]) -> Value {
        match self {
            Value::Num(_) => Value::Num(c[0]),
            Value::Len(l) => Value::Len(Length { value: c[0], unit: l.unit }),
            Value::Pair(p) => {
                Value::Pair([Length { value: c[0], unit: p[0].unit }, Length { value: c[1], unit: p[1].unit }])
            }
            Value::Color(_) => Value::Color([c[0], c[1], c[2], c[3]]),
            Value::List(_) => Value::List(c.into()),
            other => other.clone(),
        }
    }

    /// True when `self` and `other` interpolate componentwise.
    pub fn compatible(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Num(_), Value::Num(_)) | (Value::Color(_), Value::Color(_)) => true,
            (Value::Len(a), Value::Len(b)) => a.unit == b.unit,
            (Value::Pair(a), Value::Pair(b)) => a[0].unit == b[0].unit && a[1].unit == b[1].unit,
            (Value::List(a), Value::List(b)) => a.len() == b.len(),
            _ => false,
        }
    }

    /// Linear interpolation; discrete values switch at `u >= 1`.
    #[inline]
    pub fn lerp(&self, b: &Value, u: f64) -> Value {
        match (self, b) {
            (Value::Num(x), Value::Num(y)) => Value::Num(x + (y - x) * u),
            (Value::Len(x), Value::Len(y)) if x.unit == y.unit => {
                Value::Len(Length { value: x.value + (y.value - x.value) * u, unit: x.unit })
            }
            (Value::Pair(x), Value::Pair(y)) if x[0].unit == y[0].unit && x[1].unit == y[1].unit => Value::Pair([
                Length { value: x[0].value + (y[0].value - x[0].value) * u, unit: x[0].unit },
                Length { value: x[1].value + (y[1].value - x[1].value) * u, unit: x[1].unit },
            ]),
            (Value::Color(x), Value::Color(y)) => Value::Color([0, 1, 2, 3].map(|i| x[i] + (y[i] - x[i]) * u)),
            _ if self.compatible(b) => {
                let (mut a, mut c) = (Vec::with_capacity(4), Vec::with_capacity(4));
                self.components(&mut a);
                b.components(&mut c);
                let m: Vec<f64> = a.iter().zip(&c).map(|(x, y)| x + (y - x) * u).collect();
                self.with_components(&m)
            }
            _ => {
                if u >= 1.0 {
                    b.clone()
                } else {
                    self.clone()
                }
            }
        }
    }

    /// Weighted sum Σ wᵢ·vᵢ over compatible values (Hermite splines). Falls
    /// back to the first value when values are not numeric or not compatible.
    pub fn weighted(terms: &[(&Value, f64)]) -> Value {
        let first = terms[0].0;
        // allocation-free paths for scalars
        match first {
            Value::Num(_) if terms.iter().all(|(v, _)| matches!(v, Value::Num(_))) => {
                return Value::Num(terms.iter().map(|(v, w)| if let Value::Num(x) = v { x * w } else { 0.0 }).sum());
            }
            Value::Len(l) if terms.iter().all(|(v, _)| matches!(v, Value::Len(m) if m.unit == l.unit)) => {
                let s = terms.iter().map(|(v, w)| if let Value::Len(m) = v { m.value * w } else { 0.0 }).sum();
                return Value::Len(Length { value: s, unit: l.unit });
            }
            _ => {}
        }
        let mut acc = Vec::new();
        let mut c = Vec::new();
        if !first.components(&mut acc) {
            return first.clone();
        }
        acc.iter_mut().for_each(|x| *x *= terms[0].1);
        for (v, w) in &terms[1..] {
            if !first.compatible(v) || !v.components(&mut c) {
                return first.clone();
            }
            acc.iter_mut().zip(&c).for_each(|(a, x)| *a += w * x);
        }
        first.with_components(&acc)
    }

    /// `self + other` for additive animation; non-numeric values are replaced.
    pub fn add(&self, other: &Value) -> Value {
        match (self, other) {
            (Value::Num(a), Value::Num(b)) => Value::Num(a + b),
            (Value::Len(a), Value::Num(b)) => Value::Len(Length { value: a.value + b, unit: a.unit }),
            _ if self.compatible(other) => Value::weighted(&[(self, 1.0), (other, 1.0)]),
            _ => other.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(name: &str) -> PropKind {
        PropKind::of_simple_type(SIMPLE_TYPES.iter().position(|t| t.name == name).unwrap())
    }

    #[test]
    fn kinds_follow_the_schema() {
        assert_eq!(
            kind("unitDecimal"),
            PropKind::Number(Range { lo: Some((0.0, false)), hi: Some((1.0, false)), integer: false })
        );
        assert!(matches!(kind("positiveDecimal"), PropKind::Number(Range { lo: Some((0.0, true)), .. })));
        assert_eq!(kind("lengthType"), PropKind::Length { positive: false });
        assert_eq!(kind("paintType"), PropKind::Paint);
        assert_eq!(kind("blendType"), PropKind::Str);
        assert_eq!(kind("xs:boolean"), PropKind::Bool);
        assert!(matches!(kind("xs:positiveInteger"), PropKind::Number(Range { integer: true, .. })));
    }

    #[test]
    fn interpolation_and_clamping() {
        let none = |_: &str| None;
        let a = PropKind::Pair.parse("0,50%", &none).unwrap();
        let b = PropKind::Pair.parse("100,100%", &none).unwrap();
        assert_eq!(a.lerp(&b, 0.5), Value::Pair([Length::px(50.0), Length { value: 75.0, unit: LengthUnit::Percent }]));
        let c = PropKind::Color.parse("#ff0000", &none).unwrap();
        let d = PropKind::Color.parse("0,0,1,0", &none).unwrap();
        assert_eq!(c.lerp(&d, 0.5), Value::Color([0.5, 0.0, 0.5, 0.5]));
        assert_eq!(Value::Str("a".into()).lerp(&Value::Str("b".into()), 0.99), Value::Str("a".into()));
        assert_eq!(kind("unitDecimal").clamp(Value::Num(1.2)), Value::Num(1.0));
        assert_eq!(kind("xs:positiveInteger").clamp(Value::Num(2.6)), Value::Num(3.0));
        let tok = |n: &str| (n == "brand").then_some([1.0, 0.5, 0.0, 1.0]);
        assert_eq!(PropKind::Paint.parse("var(--brand)", &tok).unwrap(), Value::Color([1.0, 0.5, 0.0, 1.0]));
        assert_eq!(PropKind::Paint.parse("url(#g)", &tok).unwrap(), Value::PaintRef("g".into()));
    }
}
