//! MapLibre style layers: the subset a video basemap needs.
//!
//! Layers of type `background`, `fill`, `line`, `symbol` (text), `circle` and
//! `raster`, with `source-layer`, `filter`, `minzoom`/`maxzoom` and
//! `visibility`. Values may be constants, legacy functions (`{"stops": …}`) or
//! expressions: `get`, `has`, `id`, `geometry-type`, `properties`, `zoom`,
//! `literal`, `let`/`var`, comparisons, `!`, `all`, `any`, `case`, `match`,
//! `coalesce`, `in`, `interpolate` (linear, exponential, cubic-bezier),
//! `step`, `concat`, `format`, `length`, string case, arithmetic and maths,
//! type conversions and assertions, `rgb`/`rgba`/`to-color`, and
//! `is-supported-script`. Legacy filters (`["==", "class", "park"]`,
//! `$type`, `$id`, `in`, `!in`, `has`, `!has`, `none`) are read as MapLibre
//! reads them. An expression that fails at run time yields the property's
//! default, and a failing filter rejects the feature, as in MapLibre.
//!
//! Evaluation is checked against MapLibre's own style-spec on a real basemap
//! style and real tiles (`tools/fixtures/make_style_expected.mjs`).

use serde_json::{Map, Value};

/// A value of the expression language.
#[derive(Clone, Debug, PartialEq)]
pub enum V {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    /// Straight RGBA in 0‥1.
    Color([f64; 4]),
    Arr(Vec<V>),
    Obj(Map<String, Value>),
}

impl V {
    fn from_json(v: &Value) -> V {
        match v {
            Value::Null => V::Null,
            Value::Bool(b) => V::Bool(*b),
            Value::Number(n) => V::Num(n.as_f64().unwrap_or(0.0)),
            Value::String(s) => V::Str(s.clone()),
            Value::Array(a) => V::Arr(a.iter().map(V::from_json).collect()),
            Value::Object(o) => V::Obj(o.clone()),
        }
    }
    /// The number, if it is one.
    pub fn num(&self) -> Option<f64> {
        match self {
            V::Num(n) => Some(*n),
            _ => None,
        }
    }
    /// The colour, if it is one or a string naming one.
    pub fn color(&self) -> Option<[f64; 4]> {
        match self {
            V::Color(c) => Some(*c),
            V::Str(s) => parse_color(s),
            _ => None,
        }
    }
    /// As text (MapLibre's `to-string`).
    pub fn text(&self) -> String {
        match self {
            V::Null => String::new(),
            V::Bool(b) => b.to_string(),
            V::Num(n) => js_number(*n),
            V::Str(s) => s.clone(),
            V::Color(c) => {
                let b = |v: f64| (v * 255.0).round() as i64;
                format!("rgba({},{},{},{})", b(c[0]), b(c[1]), b(c[2]), js_number(c[3]))
            }
            V::Arr(a) => format!("[{}]", a.iter().map(V::json_text).collect::<Vec<_>>().join(",")),
            V::Obj(o) => Value::Object(o.clone()).to_string(),
        }
    }
    fn json_text(&self) -> String {
        match self {
            V::Str(s) => Value::String(s.clone()).to_string(),
            other => other.text(),
        }
    }
    fn truthy(&self) -> bool {
        matches!(self, V::Bool(true))
    }
}

/// JavaScript's number formatting for the values maps use.
fn js_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e21 {
        format!("{}", n as i64)
    } else {
        let s = format!("{n}");
        s
    }
}

/// What an expression is evaluated for.
pub struct Ctx<'a> {
    pub zoom: f64,
    pub properties: &'a Map<String, Value>,
    /// `Point`, `LineString` or `Polygon`.
    pub geometry: &'a str,
    pub id: Option<u64>,
}

type R = Result<V, String>;

fn arg(e: &[Value], i: usize) -> Result<&Value, String> {
    e.get(i).ok_or_else(|| format!("{}: missing argument {i}", e[0]))
}

fn compare(op: &str, a: &V, b: &V) -> Result<bool, String> {
    Ok(match op {
        "==" => a == b,
        "!=" => a != b,
        _ => {
            let ord = match (a, b) {
                (V::Num(x), V::Num(y)) => x.partial_cmp(y),
                (V::Str(x), V::Str(y)) => Some(x.cmp(y)),
                _ => return Err(format!("{op}: cannot compare {a:?} and {b:?}")),
            };
            let Some(o) = ord else { return Ok(false) };
            match op {
                "<" => o.is_lt(),
                "<=" => o.is_le(),
                ">" => o.is_gt(),
                _ => o.is_ge(),
            }
        }
    })
}

/// A unit-interval cubic Bézier easing (as in CSS and MapLibre), solved for x by bisection.
fn bezier(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    let c = |t: f64, p1: f64, p2: f64| 3.0 * (1.0 - t) * (1.0 - t) * t * p1 + 3.0 * (1.0 - t) * t * t * p2 + t * t * t;
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if c(mid, x1, x2) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    c((lo + hi) / 2.0, y1, y2)
}

/// Interpolates between two outputs.
fn lerp(a: &V, b: &V, t: f64) -> R {
    Ok(match (a, b) {
        (V::Num(x), V::Num(y)) => V::Num(x + (y - x) * t),
        (V::Arr(x), V::Arr(y)) if x.len() == y.len() => {
            V::Arr(x.iter().zip(y).map(|(p, q)| lerp(p, q, t)).collect::<Result<_, _>>()?)
        }
        _ => match (a.color(), b.color()) {
            (Some(x), Some(y)) => V::Color([0, 1, 2, 3].map(|k| x[k] + (y[k] - x[k]) * t)),
            _ => return Err(format!("cannot interpolate {a:?} and {b:?}")),
        },
    })
}

/// Evaluates an expression.
pub fn eval(e: &Value, cx: &Ctx, scope: &mut Vec<(String, V)>) -> R {
    let Value::Array(a) = e else {
        return Ok(match e {
            Value::Object(o) => V::Obj(o.clone()),
            other => V::from_json(other),
        });
    };
    let Some(Value::String(op)) = a.first() else {
        // an array literal outside `literal`
        return Ok(V::Arr(a.iter().map(|v| eval(v, cx, scope)).collect::<Result<_, _>>()?));
    };
    let ev = |i: usize, scope: &mut Vec<(String, V)>| -> R { eval(arg(a, i)?, cx, scope) };
    Ok(match op.as_str() {
        "literal" => V::from_json(arg(a, 1)?),
        "get" => {
            let k = ev(1, scope)?.text();
            match a.get(2) {
                Some(obj) => match eval(obj, cx, scope)? {
                    V::Obj(o) => o.get(&k).map(V::from_json).unwrap_or(V::Null),
                    _ => V::Null,
                },
                None => cx.properties.get(&k).map(V::from_json).unwrap_or(V::Null),
            }
        }
        "has" => V::Bool(cx.properties.contains_key(&ev(1, scope)?.text())),
        "id" => cx.id.map(|i| V::Num(i as f64)).unwrap_or(V::Null),
        "properties" => V::Obj(cx.properties.clone()),
        "geometry-type" => V::Str(cx.geometry.to_string()),
        "zoom" => V::Num(cx.zoom),
        "let" => {
            let n = scope.len();
            let mut i = 1;
            while i + 1 < a.len() {
                let name = arg(a, i)?.as_str().ok_or("let: names are strings")?.to_string();
                let v = ev(i + 1, scope)?;
                scope.push((name, v));
                i += 2;
            }
            let r = eval(a.last().ok_or("let: no body")?, cx, scope);
            scope.truncate(n);
            r?
        }
        "var" => {
            let name = arg(a, 1)?.as_str().unwrap_or("");
            scope
                .iter()
                .rev()
                .find(|(n, _)| n == name)
                .map(|x| x.1.clone())
                .ok_or_else(|| format!("var {name}: unbound"))?
        }
        "==" | "!=" | "<" | "<=" | ">" | ">=" => V::Bool(compare(op, &ev(1, scope)?, &ev(2, scope)?)?),
        "!" => V::Bool(!ev(1, scope)?.truthy()),
        "all" => {
            for i in 1..a.len() {
                if !ev(i, scope)?.truthy() {
                    return Ok(V::Bool(false));
                }
            }
            V::Bool(true)
        }
        "any" => {
            for i in 1..a.len() {
                if ev(i, scope)?.truthy() {
                    return Ok(V::Bool(true));
                }
            }
            V::Bool(false)
        }
        "case" => {
            let mut i = 1;
            while i + 1 < a.len() {
                if ev(i, scope)?.truthy() {
                    return ev(i + 1, scope);
                }
                i += 2;
            }
            ev(a.len() - 1, scope)?
        }
        "match" => {
            let input = ev(1, scope)?;
            let mut i = 2;
            while i + 1 < a.len() {
                let hit = match &a[i] {
                    Value::Array(labels) => labels.iter().any(|l| V::from_json(l) == input),
                    l => V::from_json(l) == input,
                };
                if hit {
                    return ev(i + 1, scope);
                }
                i += 2;
            }
            ev(a.len() - 1, scope)?
        }
        "coalesce" => {
            for i in 1..a.len() {
                match ev(i, scope) {
                    Ok(V::Null) | Err(_) => continue,
                    Ok(v) => return Ok(v),
                }
            }
            V::Null
        }
        "in" => {
            let (needle, hay) = (ev(1, scope)?, ev(2, scope)?);
            V::Bool(match hay {
                V::Arr(xs) => xs.contains(&needle),
                V::Str(s) => s.contains(&needle.text()),
                _ => return Err("in: the haystack is not an array or string".into()),
            })
        }
        "interpolate" => {
            let kind = arg(a, 1)?.as_array().ok_or("interpolate: bad type")?;
            let x = ev(2, scope)?.num().ok_or("interpolate: input is not a number")?;
            let stops: Vec<(f64, &Value)> =
                a[3..].chunks(2).filter_map(|c| Some((c.first()?.as_f64()?, c.get(1)?))).collect();
            if stops.is_empty() {
                return Err("interpolate: no stops".into());
            }
            if x <= stops[0].0 {
                return eval(stops[0].1, cx, scope);
            }
            if x >= stops[stops.len() - 1].0 {
                return eval(stops[stops.len() - 1].1, cx, scope);
            }
            let k = stops.windows(2).position(|w| x < w[1].0).unwrap_or(stops.len() - 2);
            let ((x0, v0), (x1, v1)) = (stops[k], stops[k + 1]);
            let (d, p) = (x1 - x0, x - x0);
            let t = match kind.first().and_then(Value::as_str) {
                Some("exponential") => {
                    let base = kind.get(1).and_then(Value::as_f64).unwrap_or(1.0);
                    if d == 0.0 {
                        0.0
                    } else if base == 1.0 {
                        p / d
                    } else {
                        (base.powf(p) - 1.0) / (base.powf(d) - 1.0)
                    }
                }
                Some("cubic-bezier") => {
                    let c: Vec<f64> = kind[1..].iter().filter_map(Value::as_f64).collect();
                    let u = if d == 0.0 { 0.0 } else { p / d };
                    if c.len() == 4 {
                        bezier(c[0], c[1], c[2], c[3], u)
                    } else {
                        u
                    }
                }
                _ => {
                    if d == 0.0 {
                        0.0
                    } else {
                        p / d
                    }
                }
            };
            lerp(&eval(v0, cx, scope)?, &eval(v1, cx, scope)?, t)?
        }
        "step" => {
            let x = ev(1, scope)?.num().ok_or("step: input is not a number")?;
            let mut out = arg(a, 2)?;
            let mut i = 3;
            while i + 1 < a.len() {
                if x >= a[i].as_f64().unwrap_or(f64::INFINITY) {
                    out = &a[i + 1];
                } else {
                    break;
                }
                i += 2;
            }
            eval(out, cx, scope)?
        }
        "concat" => V::Str((1..a.len()).map(|i| ev(i, scope).map(|v| v.text())).collect::<Result<String, _>>()?),
        "format" => {
            // sections alternate with option objects; the text is their concatenation
            let mut s = String::new();
            for v in &a[1..] {
                if v.is_object() {
                    continue;
                }
                s.push_str(&eval(v, cx, scope)?.text());
            }
            V::Str(s)
        }
        "to-string" => V::Str(ev(1, scope)?.text()),
        "to-number" => {
            for i in 1..a.len() {
                match ev(i, scope)? {
                    V::Num(n) => return Ok(V::Num(n)),
                    V::Bool(b) => return Ok(V::Num(b as u8 as f64)),
                    V::Str(s) => {
                        if let Ok(n) = s.trim().parse() {
                            return Ok(V::Num(n));
                        }
                    }
                    V::Null => return Ok(V::Num(0.0)),
                    _ => {}
                }
            }
            return Err("to-number: not a number".into());
        }
        "to-boolean" => V::Bool(match ev(1, scope)? {
            V::Null => false,
            V::Bool(b) => b,
            V::Num(n) => n != 0.0 && !n.is_nan(),
            V::Str(s) => !s.is_empty(),
            _ => true,
        }),
        "number" | "string" | "boolean" | "array" | "object" => {
            for i in 1..a.len() {
                let v = ev(i, scope)?;
                let ok = matches!(
                    (op.as_str(), &v),
                    ("number", V::Num(_))
                        | ("string", V::Str(_))
                        | ("boolean", V::Bool(_))
                        | ("array", V::Arr(_))
                        | ("object", V::Obj(_))
                );
                if ok {
                    return Ok(v);
                }
            }
            return Err(format!("{op}: no argument has that type"));
        }
        "typeof" => V::Str(
            match ev(1, scope)? {
                V::Null => "null",
                V::Bool(_) => "boolean",
                V::Num(_) => "number",
                V::Str(_) => "string",
                V::Color(_) => "color",
                V::Arr(_) => "array",
                V::Obj(_) => "object",
            }
            .into(),
        ),
        "length" => V::Num(match ev(1, scope)? {
            V::Str(s) => s.chars().count() as f64,
            V::Arr(x) => x.len() as f64,
            _ => return Err("length: not a string or array".into()),
        }),
        "upcase" => V::Str(ev(1, scope)?.text().to_uppercase()),
        "downcase" => V::Str(ev(1, scope)?.text().to_lowercase()),
        "is-supported-script" => V::Bool(true),
        "to-color" => {
            for i in 1..a.len() {
                if let Some(c) = ev(i, scope)?.color() {
                    return Ok(V::Color(c));
                }
            }
            return Err("to-color: not a colour".into());
        }
        "rgb" | "rgba" => {
            let n = |i: usize, scope: &mut Vec<(String, V)>| -> Result<f64, String> {
                ev(i, scope)?.num().ok_or_else(|| format!("{op}: components are numbers"))
            };
            let alpha = if op == "rgba" { n(4, scope)? } else { 1.0 };
            V::Color([n(1, scope)? / 255.0, n(2, scope)? / 255.0, n(3, scope)? / 255.0, alpha])
        }
        "+" | "*" | "min" | "max" => {
            let xs: Vec<f64> = (1..a.len())
                .map(|i| ev(i, scope)?.num().ok_or_else(|| format!("{op}: not a number")))
                .collect::<Result<_, _>>()?;
            V::Num(match op.as_str() {
                "+" => xs.iter().sum(),
                "*" => xs.iter().product(),
                "min" => xs.iter().cloned().fold(f64::INFINITY, f64::min),
                _ => xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            })
        }
        "-" | "/" | "%" | "^" => {
            let x = ev(1, scope)?.num().ok_or_else(|| format!("{op}: not a number"))?;
            if a.len() == 2 && op == "-" {
                return Ok(V::Num(-x));
            }
            let y = ev(2, scope)?.num().ok_or_else(|| format!("{op}: not a number"))?;
            V::Num(match op.as_str() {
                "-" => x - y,
                "/" => x / y,
                "%" => x % y,
                _ => x.powf(y),
            })
        }
        "abs" | "round" | "floor" | "ceil" | "sqrt" | "ln" | "log10" | "log2" => {
            let x = ev(1, scope)?.num().ok_or_else(|| format!("{op}: not a number"))?;
            V::Num(match op.as_str() {
                "abs" => x.abs(),
                // JavaScript rounds halves towards +∞; MapLibre rounds them away from zero
                "round" => x.signum() * x.abs().round(),
                "floor" => x.floor(),
                "ceil" => x.ceil(),
                "sqrt" => x.sqrt(),
                "ln" => x.ln(),
                "log10" => x.log10(),
                _ => x.log2(),
            })
        }
        "pi" => V::Num(std::f64::consts::PI),
        "e" => V::Num(std::f64::consts::E),
        "feature-state"
        | "global-state"
        | "line-progress"
        | "heatmap-density"
        | "accumulated"
        | "sky-radial-progress" => V::Null,
        other => return Err(format!("unsupported expression {other:?}")),
    })
}

/// Whether a filter is written as an expression rather than a legacy filter (MapLibre's rule).
pub fn is_expression_filter(f: &Value) -> bool {
    match f {
        Value::Bool(_) => true,
        Value::Array(a) if !a.is_empty() => match a[0].as_str() {
            Some("has") => a.len() >= 2 && a[1] != "$id" && a[1] != "$type",
            Some("in") => a.len() >= 3 && (!a[1].is_string() || a[2].is_array()),
            Some("!in" | "!has" | "none") => false,
            Some("==" | "!=" | ">" | ">=" | "<" | "<=") => a.len() != 3 || a[1].is_array() || a[2].is_array(),
            Some("any" | "all") => a[1..].iter().all(|x| x.is_boolean() || is_expression_filter(x)),
            _ => true,
        },
        _ => false,
    }
}

fn legacy_filter(f: &Value, cx: &Ctx) -> bool {
    let Some(a) = f.as_array() else { return true };
    let op = a.first().and_then(Value::as_str).unwrap_or("");
    let key = |i: usize| -> V {
        match a.get(i).and_then(Value::as_str) {
            Some("$type") => V::Str(cx.geometry.into()),
            Some("$id") => cx.id.map(|i| V::Num(i as f64)).unwrap_or(V::Null),
            Some(k) => cx.properties.get(k).map(V::from_json).unwrap_or(V::Null),
            None => V::Null,
        }
    };
    let has = |i: usize| match a.get(i).and_then(Value::as_str) {
        Some("$type") => true,
        Some("$id") => cx.id.is_some(),
        Some(k) => cx.properties.contains_key(k),
        None => false,
    };
    match op {
        "all" => a[1..].iter().all(|x| legacy_filter(x, cx)),
        "any" => a[1..].iter().any(|x| legacy_filter(x, cx)),
        "none" => !a[1..].iter().any(|x| legacy_filter(x, cx)),
        "has" => has(1),
        "!has" => !has(1),
        "in" | "!in" => {
            let v = key(1);
            let found = a[2..].iter().any(|x| V::from_json(x) == v);
            found == (op == "in")
        }
        "==" | "!=" | "<" | "<=" | ">" | ">=" => {
            let (v, w) = (key(1), a.get(2).map(V::from_json).unwrap_or(V::Null));
            match op {
                "==" => v == w,
                "!=" => v != w,
                // legacy ordering needs both sides of one type
                _ => compare(op, &v, &w).unwrap_or(false),
            }
        }
        _ => true,
    }
}

/// Whether a filter accepts a feature.
pub fn filter(f: &Value, cx: &Ctx) -> bool {
    if is_expression_filter(f) {
        eval(f, cx, &mut Vec::new()).map(|v| v.truthy()).unwrap_or(false)
    } else {
        legacy_filter(f, cx)
    }
}

/// A legacy function (`{"stops": …}`).
fn function(o: &Map<String, Value>, cx: &Ctx) -> R {
    let stops = o.get("stops").and_then(Value::as_array).ok_or("function without stops")?;
    let input = match o.get("property").and_then(Value::as_str) {
        Some(p) => cx.properties.get(p).map(V::from_json).unwrap_or(V::Null),
        None => V::Num(cx.zoom),
    };
    let kind = o.get("type").and_then(Value::as_str);
    let out = |s: &Value| s.as_array().and_then(|p| p.get(1)).map(V::from_json).unwrap_or(V::Null);
    let at = |s: &Value| s.as_array().and_then(|p| p.first()).cloned().unwrap_or(Value::Null);
    let default = || o.get("default").map(V::from_json).unwrap_or(V::Null);
    if kind == Some("identity") {
        return Ok(if input == V::Null { default() } else { input });
    }
    if kind == Some("categorical") {
        return Ok(stops.iter().find(|s| V::from_json(&at(s)) == input).map(out).unwrap_or_else(default));
    }
    let Some(x) = input.num() else { return Ok(default()) };
    let xs: Vec<f64> = stops.iter().map(|s| at(s).as_f64().unwrap_or(0.0)).collect();
    let interval = kind == Some("interval")
        || stops.first().map(out).is_some_and(|v| matches!(v, V::Str(_) | V::Bool(_)) && v.color().is_none());
    if interval {
        let k = xs.iter().rposition(|s| x >= *s).unwrap_or(0);
        return Ok(out(&stops[k]));
    }
    if x <= xs[0] {
        return Ok(out(&stops[0]));
    }
    if x >= xs[xs.len() - 1] {
        return Ok(out(&stops[stops.len() - 1]));
    }
    let k = xs.windows(2).position(|w| x < w[1]).unwrap_or(xs.len() - 2);
    let base = o.get("base").and_then(Value::as_f64).unwrap_or(1.0);
    let (d, p) = (xs[k + 1] - xs[k], x - xs[k]);
    let t = if base == 1.0 { p / d } else { (base.powf(p) - 1.0) / (base.powf(d) - 1.0) };
    lerp(&out(&stops[k]), &out(&stops[k + 1]), t)
}

/// The default of a paint or layout property (MapLibre's specification).
pub fn default_of(name: &str) -> V {
    match name {
        "fill-color" | "line-color" | "circle-color" | "text-color" | "background-color" => {
            V::Color([0.0, 0.0, 0.0, 1.0])
        }
        "text-halo-color" | "circle-stroke-color" => V::Color([0.0, 0.0, 0.0, 0.0]),
        "fill-outline-color" => V::Null,
        "fill-opacity" | "line-opacity" | "circle-opacity" | "text-opacity" | "background-opacity"
        | "raster-opacity" => V::Num(1.0),
        "line-width" => V::Num(1.0),
        "circle-radius" => V::Num(5.0),
        "text-size" => V::Num(16.0),
        "text-max-width" => V::Num(10.0),
        "text-padding" => V::Num(2.0),
        "symbol-spacing" => V::Num(250.0),
        "text-offset" => V::Arr(vec![V::Num(0.0), V::Num(0.0)]),
        "text-anchor" => V::Str("center".into()),
        "text-transform" => V::Str("none".into()),
        "symbol-placement" => V::Str("point".into()),
        "line-cap" => V::Str("butt".into()),
        "line-join" => V::Str("miter".into()),
        "visibility" => V::Str("visible".into()),
        _ => V::Num(0.0),
    }
}

/// Evaluates a paint or layout property value (constant, legacy function or expression), falling
/// back to the property's default when evaluation fails.
pub fn property(name: &str, v: &Value, cx: &Ctx) -> V {
    let r = match v {
        Value::Object(o) if o.contains_key("stops") => function(o, cx),
        Value::String(s) if name == "text-field" && s.contains('{') => Ok(V::Str(tokens(s, cx))),
        other => eval(other, cx, &mut Vec::new()),
    };
    match r {
        Ok(V::Null) | Err(_) => default_of(name),
        Ok(V::Str(s)) if name.ends_with("-color") => parse_color(&s).map(V::Color).unwrap_or_else(|| default_of(name)),
        Ok(v) => v,
    }
}

/// Replaces `{name}` tokens with property values (legacy `text-field`).
fn tokens(s: &str, cx: &Ctx) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        match rest[i..].find('}') {
            Some(j) => {
                let k = &rest[i + 1..i + j];
                out.push_str(&cx.properties.get(k).map(V::from_json).unwrap_or(V::Null).text());
                rest = &rest[i + j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// A style layer.
#[derive(Clone, Debug)]
pub struct Layer {
    pub id: String,
    /// `background`, `fill`, `line`, `symbol`, `circle`, `raster`, …
    pub kind: String,
    pub source_layer: Option<String>,
    pub filter: Option<Value>,
    pub minzoom: f64,
    pub maxzoom: f64,
    pub paint: Map<String, Value>,
    pub layout: Map<String, Value>,
}

impl Layer {
    /// Whether the layer draws at `zoom`.
    pub fn visible_at(&self, zoom: f64) -> bool {
        self.layout.get("visibility").and_then(Value::as_str) != Some("none")
            && zoom >= self.minzoom
            && zoom < self.maxzoom
    }
    /// A paint property.
    pub fn paint(&self, name: &str, cx: &Ctx) -> V {
        self.paint.get(name).map(|v| property(name, v, cx)).unwrap_or_else(|| default_of(name))
    }
    /// A layout property.
    pub fn layout(&self, name: &str, cx: &Ctx) -> V {
        self.layout.get(name).map(|v| property(name, v, cx)).unwrap_or_else(|| default_of(name))
    }
    /// Whether the layer's filter accepts a feature.
    pub fn accepts(&self, cx: &Ctx) -> bool {
        self.filter.as_ref().is_none_or(|f| filter(f, cx))
    }
}

/// A style: its layers in drawing order.
#[derive(Clone, Debug, Default)]
pub struct Style {
    pub layers: Vec<Layer>,
}

impl Style {
    /// Reads a MapLibre style document (or a bare array of layers).
    pub fn parse(json: &str) -> Result<Style, String> {
        let v: Value = serde_json::from_str(json).map_err(|e| format!("style: {e}"))?;
        let layers = match &v {
            Value::Array(a) => a.clone(),
            Value::Object(o) => o.get("layers").and_then(Value::as_array).cloned().ok_or("style has no layers")?,
            _ => return Err("style is not a style document".into()),
        };
        let mut out = Vec::new();
        for l in layers {
            let o = l.as_object().ok_or("a style layer is not an object")?;
            let s = |k: &str| o.get(k).and_then(Value::as_str).map(str::to_string);
            out.push(Layer {
                id: s("id").unwrap_or_default(),
                kind: s("type").unwrap_or_default(),
                source_layer: s("source-layer"),
                filter: o.get("filter").cloned(),
                minzoom: o.get("minzoom").and_then(Value::as_f64).unwrap_or(0.0),
                maxzoom: o.get("maxzoom").and_then(Value::as_f64).unwrap_or(24.0),
                paint: o.get("paint").and_then(Value::as_object).cloned().unwrap_or_default(),
                layout: o.get("layout").and_then(Value::as_object).cloned().unwrap_or_default(),
            });
        }
        Ok(Style { layers: out })
    }
}

/// A built-in style by name: `protomaps-light` or `protomaps-dark`, the Protomaps basemap styles
/// (BSD-3-Clause, see `styles/NOTICE`) for the Protomaps tile schema.
pub fn builtin(name: &str) -> Option<&'static str> {
    match name {
        "protomaps-light" => Some(include_str!("../styles/protomaps-light.json")),
        "protomaps-dark" => Some(include_str!("../styles/protomaps-dark.json")),
        _ => None,
    }
}

/// Parses a CSS colour: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`/`rgba()`, `hsl()`/`hsla()`
/// and the named colours.
pub fn parse_color(s: &str) -> Option<[f64; 4]> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(h) = s.strip_prefix('#') {
        let d = |c: char| c.to_digit(16).map(|v| v as f64);
        let cs: Vec<char> = h.chars().collect();
        return match cs.len() {
            3 | 4 => {
                let v: Option<Vec<f64>> = cs.iter().map(|c| d(*c).map(|x| x * 17.0 / 255.0)).collect();
                let v = v?;
                Some([v[0], v[1], v[2], v.get(3).copied().unwrap_or(1.0)])
            }
            6 | 8 => {
                let v: Option<Vec<f64>> = cs.chunks(2).map(|p| Some((d(p[0])? * 16.0 + d(p[1])?) / 255.0)).collect();
                let v = v?;
                Some([v[0], v[1], v[2], v.get(3).copied().unwrap_or(1.0)])
            }
            _ => None,
        };
    }
    if let Some((f, rest)) = s.split_once('(') {
        let inner = rest.strip_suffix(')')?;
        let parts: Vec<&str> = inner.split([',', ' ', '/']).map(str::trim).filter(|p| !p.is_empty()).collect();
        let num = |p: &str, scale: f64| -> Option<f64> {
            match p.strip_suffix('%') {
                Some(q) => q.parse::<f64>().ok().map(|v| v / 100.0 * scale),
                None => p.parse::<f64>().ok(),
            }
        };
        let alpha = |p: Option<&&str>| p.map(|a| num(a, 1.0)).unwrap_or(Some(1.0));
        return match f.trim() {
            "rgb" | "rgba" if parts.len() >= 3 => {
                let c = |i: usize| num(parts[i], 255.0).map(|v| (v / 255.0).clamp(0.0, 1.0));
                Some([c(0)?, c(1)?, c(2)?, alpha(parts.get(3))?.clamp(0.0, 1.0)])
            }
            "hsl" | "hsla" if parts.len() >= 3 => {
                let h = parts[0].trim_end_matches("deg").parse::<f64>().ok()?.rem_euclid(360.0) / 360.0;
                let (sat, l) = (num(parts[1], 1.0)?.clamp(0.0, 1.0), num(parts[2], 1.0)?.clamp(0.0, 1.0));
                let q = if l < 0.5 { l * (1.0 + sat) } else { l + sat - l * sat };
                let p = 2.0 * l - q;
                let hue = |t: f64| {
                    let t = t.rem_euclid(1.0);
                    if t < 1.0 / 6.0 {
                        p + (q - p) * 6.0 * t
                    } else if t < 0.5 {
                        q
                    } else if t < 2.0 / 3.0 {
                        p + (q - p) * (2.0 / 3.0 - t) * 6.0
                    } else {
                        p
                    }
                };
                Some([hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0), alpha(parts.get(3))?.clamp(0.0, 1.0)])
            }
            _ => None,
        };
    }
    if s == "transparent" {
        return Some([0.0; 4]);
    }
    let hex = NAMED.iter().find(|(n, _)| *n == s)?.1;
    parse_color(&format!("#{hex:06x}"))
}

const NAMED: [(&str, u32); 148] = [
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xa9a9a9),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("grey", 0x808080),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightgrey", 0xd3d3d3),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse() {
        assert_eq!(parse_color("#f00"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(parse_color("rgba(255, 0, 0, 0.5)"), Some([1.0, 0.0, 0.0, 0.5]));
        let h = parse_color("hsl(120, 100%, 50%)").unwrap();
        assert!((h[1] - 1.0).abs() < 1e-12 && h[0].abs() < 1e-12);
        assert_eq!(parse_color("rebeccapurple"), parse_color("#663399"));
    }

    #[test]
    fn legacy_and_expression_filters() {
        let props = serde_json::json!({"class": "park", "rank": 3}).as_object().unwrap().clone();
        let cx = Ctx { zoom: 10.0, properties: &props, geometry: "Polygon", id: Some(7) };
        let f = |s: &str| filter(&serde_json::from_str(s).unwrap(), &cx);
        assert!(f(r#"["==", "class", "park"]"#));
        assert!(f(r#"["all", ["==", "$type", "Polygon"], ["in", "class", "park", "wood"], ["!has", "name"]]"#));
        assert!(f(r#"["all", ["==", ["get", "class"], "park"], [">=", ["get", "rank"], 2]]"#));
        assert!(!f(r#"["<", ["get", "class"], 3]"#), "type mismatch rejects");
        assert!(f(r#"["match", ["get", "class"], ["wood", "park"], true, false]"#));
    }
}
