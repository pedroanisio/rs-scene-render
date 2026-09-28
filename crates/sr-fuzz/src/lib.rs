//! Mutation fuzzing of the document pipeline and the expression compiler.
//!
//! Two targets take arbitrary text and must never panic:
//!
//! * **documents**: validation, loading, template instantiation and
//!   evaluation at three times (`sr_model::validate_str`, `load_str`,
//!   `sr_eval::Evaluator`);
//! * **expressions**: parsing, compilation against a stub resolver and
//!   execution against a stub host (`sr_eval::expr`).
//!
//! Inputs come from the conformance corpus and built-in seeds, mutated by
//! byte flips, span deletion and duplication, splices between seeds,
//! attribute-value replacement with boundary values, nesting bombs and
//! truncation; expressions also come from a grammar generator. Every panic
//! is caught and returned with the input that caused it.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use sr_eval::expr::vm::{self, Band, Host, LoopKind, Resolver, Var, V};

/// Xorshift64* generator.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
    pub fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len())]
    }
}

const VALUES: &[&str] = &[
    "",
    "0",
    "-0",
    "1",
    "-1",
    "1e308",
    "-1e308",
    "1e-320",
    "NaN",
    "INF",
    "-INF",
    "+1",
    "99999999999999999999",
    "0.5",
    "-0.5",
    "#",
    "#FFF",
    "#GGGGGG",
    "#00000000",
    "url(#)",
    "url(#x",
    "var(--)",
    "var(--x",
    "10px",
    "50%",
    "10vw",
    "1e9vh",
    "-10vmin",
    "0..",
    "1/0",
    "30000/1001",
    "00:00:00:00",
    "99:99:99:99",
    "true",
    "false",
    "none",
    "auto",
    "\u{0}",
    "\u{FFFF}",
    "😀",
    "a b c",
    "x,y",
    "1,2,3,4,5,6,7,8",
    "M0 0 L",
    "M 1e308 1e308 L -1e308 -1e308 Z",
    "C",
    "&amp;",
    "&#0;",
    "&#x110000;",
];

const TOKENS: &[&str] = &[
    "<",
    ">",
    "/>",
    "</group>",
    "<group>",
    "<group id=\"g\">",
    "<layer/>",
    "<shape/>",
    "\"",
    "'",
    "=",
    "&",
    "&lt;",
    "<![CDATA[",
    "]]>",
    "<!--",
    "-->",
    "<?xml version=\"1.0\"?>",
    "<!DOCTYPE x [<!ENTITY a \"aaaa\">]>",
    "&a;",
    " id=\"dup\"",
    " start=\"-1\"",
    " duration=\"0\"",
    " fps=\"0\"",
    " width=\"0\"",
    "<animate property=\"x\"><key time=\"0\" value=\"0\"/></animate>",
    "<repeat count=\"1000000\">",
    "<instance symbol=\"s\"/>",
    "<include src=\"self.xml\"/>",
    " expression=\"time*\"",
];

/// Built-in seed documents touching every major feature.
pub const SEEDS: &[&str] = &[
    r##"<scene version="1.1"><project width="64" height="64" fps="30" duration="2"/><parameters><param id="p" type="number" default="3"/></parameters><composition><group id="g" x="10"><shape id="s" shape="rect" width="10" height="10" fill="#FF0000"><animate property="x"><key time="0" value="0"/><key time="1" value="50" easeOut="0.5,0.5"/></animate></shape><shape id="e" shape="ellipse" width="8" height="8" fill="#00FF00" expression="x: prop('s.x') + wiggle(2, 5); rotation: time * 90"/></group><repeat id="r" count="3"><shape id="c" shape="rect" width="4" height="4" x="${index * 10}"/></repeat></composition></scene>"##,
    r##"<scene version="1.1"><project width="64" height="64" fps="30" duration="2"/><composition><shape id="b" shape="rect" width="10" height="10" fill="#FFFFFF"><rigidBody/></shape><particleEmitter id="pe" preset="sparks" x="32" y="32"><burst time="0.5" count="10"/></particleEmitter><object3D id="o" primitive="sphere" radius="10" x="32" y="32"/></composition><physics bounds="floor"><forceField id="f" type="vortex" strength="10"/><constraint id="k" type="pin" a="b" x="0" y="0"/></physics></scene>"##,
];

/// Seeds: built-ins plus every document of the conformance corpus.
pub fn corpus(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = SEEDS.iter().map(|s| s.to_string()).collect();
    for dir in ["valid", "invalid"] {
        if let Ok(rd) = std::fs::read_dir(root.join(dir)) {
            let mut files: Vec<_> =
                rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "xml")).collect();
            files.sort();
            for f in files {
                if let Ok(s) = std::fs::read_to_string(&f) {
                    out.push(s);
                }
            }
        }
    }
    out
}

fn char_floor(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// One mutation of a document.
pub fn mutate_doc(r: &mut Rng, seeds: &[String], src: &str) -> String {
    let mut s = src.to_string();
    for _ in 0..1 + r.below(4) {
        let n = s.len().max(1);
        let a = char_floor(&s, r.below(n));
        let b = char_floor(&s, (a + 1 + r.below(64)).min(s.len()));
        match r.below(8) {
            0 => {
                // flip one byte into another printable or control character
                let c = (b' ' + r.below(95) as u8) as char;
                s.replace_range(a..char_floor(&s, a + 1).max(a), &c.to_string());
            }
            1 => s.replace_range(a..b, ""),
            2 => {
                let span = s[a..b].to_string();
                s.insert_str(a, &span);
            }
            3 => s.insert_str(a, r.pick(TOKENS)),
            4 => {
                // splice a span of another seed
                let o = r.pick(seeds);
                let oa = char_floor(o, r.below(o.len().max(1)));
                let ob = char_floor(o, (oa + r.below(200)).min(o.len()));
                s.insert_str(a, &o[oa..ob]);
            }
            5 => {
                // replace an attribute value with a boundary value
                if let Some(q) = s[a..].find("=\"") {
                    let v0 = a + q + 2;
                    if let Some(e) = s[v0..].find('"') {
                        let v = r.pick(VALUES).to_string();
                        s.replace_range(v0..v0 + e, &v);
                    }
                }
            }
            6 => {
                // nesting bomb
                let depth = 1 + r.below(400);
                let open = "<group>".repeat(depth);
                let close = "</group>".repeat(depth);
                if let Some(p) = s.find("<composition>") {
                    s.insert_str(p + 13, &format!("{open}{close}"));
                }
            }
            _ => s.truncate(char_floor(&s, a)),
        }
    }
    s
}

/// A random expression from a small grammar.
pub fn gen_expr(r: &mut Rng, depth: u32) -> String {
    let leaf = |r: &mut Rng| -> String {
        match r.below(8) {
            0 => format!("{}", r.below(1000) as f64 / 7.0),
            1 => r
                .pick(&[
                    "time",
                    "frame",
                    "value",
                    "index",
                    "count",
                    "seed",
                    "fps",
                    "duration",
                    "textIndex",
                    "textTotal",
                ])
                .to_string(),
            2 => format!("\"{}\"", r.pick(&["a", "", "x.y", "😀"])),
            3 => r.pick(&["true", "false", "null", "undefined", "NaN", "Infinity"]).to_string(),
            4 => format!("[{}, {}]", r.below(9), r.below(9)),
            5 => "prop(\"s.x\")".into(),
            6 => r.pick(&["1e308", "-1e308", "5e-324", "0/0", "-0"]).to_string(),
            _ => format!("{}", r.next_u64() % 100),
        }
    };
    if depth == 0 {
        return leaf(r);
    }
    let sub = |r: &mut Rng| gen_expr(r, depth - 1);
    match r.below(14) {
        0 | 1 => leaf(r),
        2 => format!(
            "({} {} {})",
            sub(r),
            r.pick(&["+", "-", "*", "/", "%", "**", "==", "!=", "<", ">=", "&&", "||", "??"]),
            sub(r)
        ),
        3 => format!("{}{}", r.pick(&["-", "!", "+"]), sub(r)),
        4 => format!("{} ? {} : {}", sub(r), sub(r), sub(r)),
        5 => {
            let f = r.pick(&[
                "Math.sin",
                "Math.sqrt",
                "Math.pow",
                "Math.max",
                "clamp",
                "linear",
                "ease",
                "wiggle",
                "random",
                "noise",
                "valueAtTime",
                "loopOut",
                "loopIn",
                "audioAmplitude",
                "beat",
                "markerTime",
                "param",
                "rgb",
                "hsl",
                "length",
                "normalize",
            ]);
            let args: Vec<String> = (0..r.below(5)).map(|_| sub(r)).collect();
            format!("{f}({})", args.join(", "))
        }
        6 => format!("{}[{}]", sub(r), sub(r)),
        7 => format!("{}.{}", sub(r), r.pick(&["x", "y", "length", "toFixed(2)", "__proto__"])),
        8 => format!("let v = {}; v * {}", sub(r), sub(r)),
        9 => format!("if ({}) {{ {} }} else {{ {} }}", sub(r), sub(r), sub(r)),
        10 => format!("{}; return {}", sub(r), sub(r)),
        11 => format!("function f(a) {{ return a + {} }} f({})", sub(r), sub(r)),
        12 => format!("for (let i = 0; i < {}; i++) {{ {} }}", r.below(5), sub(r)),
        _ => format!("`{}`", sub(r)),
    }
}

/// One mutation of an expression.
pub fn mutate_expr(r: &mut Rng, src: &str) -> String {
    let mut s = src.to_string();
    let n = s.len().max(1);
    let a = char_floor(&s, r.below(n));
    match r.below(4) {
        0 => s.insert_str(
            a,
            r.pick(&["(", ")", "[", "]", "{", "}", ",", ";", "\"", "'", "`", ".", "=>", "...", "//", "/*", "\\"]),
        ),
        1 => s.truncate(a),
        2 => {
            let b = char_floor(&s, (a + r.below(8)).min(s.len()));
            s.replace_range(a..b, "");
        }
        _ => s.insert_str(a, &gen_expr(r, 2)),
    }
    s
}

struct Resolve;

impl Resolver for Resolve {
    fn prop(&mut self, path: &str) -> Result<u32, String> {
        if path.contains('.') {
            Ok((path.len() % 7) as u32)
        } else {
            Err(format!("unknown property {path}"))
        }
    }
    fn marker(&mut self, id: &str) -> Option<f64> {
        (!id.is_empty()).then_some(1.0)
    }
}

struct Stub;

impl Host for Stub {
    fn var(&mut self, v: Var) -> V {
        match v {
            Var::Time => V::Num(1.25),
            Var::Frame => V::Num(37.0),
            _ => V::Num(3.0),
        }
    }
    fn prop(&mut self, slot: u32) -> V {
        V::Num(slot as f64)
    }
    fn value_at_time(&mut self, t: f64) -> V {
        V::Num(t)
    }
    fn param(&mut self, _name: &str) -> V {
        V::Num(2.0)
    }
    fn loop_value(&mut self, _out: bool, _kind: LoopKind, keys: usize) -> V {
        V::Num(keys as f64)
    }
    fn audio(&mut self, _track: &str, _band: Band) -> f64 {
        0.5
    }
    fn beat(&mut self) -> f64 {
        4.0
    }
    fn random(&mut self, site: u32, component: u32) -> f64 {
        ((site * 31 + component) % 97) as f64 / 97.0
    }
    fn noise_seed(&mut self) -> u64 {
        7
    }
}

/// Runs the document pipeline on `input`.
pub fn run_doc(input: &str) {
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    let _ = sr_model::validate_str(input, &opts);
    if let Ok(doc) = sr_model::load_str(input, &opts) {
        if let Ok(ev) = sr_eval::Evaluator::new(&doc, &Default::default()) {
            let d = ev.program().duration;
            for t in [0.0, d * 0.5, d] {
                let _ = ev.evaluate(t);
            }
        }
    }
}

/// Runs the expression pipeline on `input`.
pub fn run_expr(input: &str) {
    let _ = sr_eval::expr::parse::parse(input);
    if let Ok(code) = vm::compile(input, &mut Resolve) {
        let mut regs = Vec::new();
        let _ = vm::run(&code, &mut Stub, &mut regs);
    }
}

/// A panic found by the fuzzer.
#[derive(Debug, Clone)]
pub struct Crash {
    pub target: &'static str,
    pub input: String,
    pub message: String,
}

/// Runs `f(input)`, turning a panic into a [`Crash`].
pub fn guard(target: &'static str, input: &str, f: fn(&str)) -> Option<Crash> {
    catch_unwind(AssertUnwindSafe(|| f(input))).err().map(|e| Crash {
        target,
        input: input.to_string(),
        message: e
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default(),
    })
}

/// Statistics of a campaign.
#[derive(Debug, Default, Clone)]
pub struct Campaign {
    pub docs: u64,
    pub exprs: u64,
    pub crashes: Vec<Crash>,
}

/// Fuzzes until `stop()` returns true.
pub fn fuzz(seed: u64, seeds: &[String], mut stop: impl FnMut(&Campaign) -> bool) -> Campaign {
    let mut r = Rng(seed | 1);
    let mut c = Campaign::default();
    let exprs0: Vec<String> = (0..64).map(|_| gen_expr(&mut r, 4)).collect();
    while !stop(&c) {
        if r.below(3) == 0 {
            let base = r.pick(&exprs0).clone();
            let input = if r.below(2) == 0 { gen_expr(&mut r, 5) } else { mutate_expr(&mut r, &base) };
            if let Some(k) = guard("expression", &input, run_expr) {
                c.crashes.push(k);
            }
            c.exprs += 1;
        } else {
            let base = r.pick(seeds).clone();
            let input = mutate_doc(&mut r, seeds, &base);
            if let Some(k) = guard("document", &input, run_doc) {
                c.crashes.push(k);
            }
            c.docs += 1;
        }
    }
    c
}
