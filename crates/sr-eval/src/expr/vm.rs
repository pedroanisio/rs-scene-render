//! Compilation of parsed expressions to register bytecode, and the VM.
//!
//! Each expression compiles once. Names resolve at compile time: built-in
//! variables become `Var` loads, `prop("id.property")` becomes a direct slot
//! read (and a dependency edge), `markerTime("id")` becomes a constant, and
//! `Math.*` members become constants or direct function calls. The VM then
//! runs a flat instruction list over a register file with no allocation on
//! the numeric path.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use super::parse::{parse, BinOp, Expr, Logic, Stmt, UnOp};
use crate::rng;

/// A runtime value of the expression language.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum V {
    /// `undefined` / `null`.
    #[default]
    Undef,
    /// Number.
    Num(f64),
    /// Boolean.
    Bool(bool),
    /// String.
    Str(Arc<str>),
    /// Array (vectors, colours, lists).
    Arr(Arc<[V]>),
    /// Record (a data row).
    Obj(Arc<BTreeMap<String, V>>),
}

impl V {
    /// JavaScript `Number(v)`.
    pub fn num(&self) -> f64 {
        match self {
            V::Num(n) => *n,
            V::Bool(b) => *b as u8 as f64,
            V::Str(s) => {
                let t = s.trim();
                if t.is_empty() {
                    0.0
                } else {
                    t.parse().unwrap_or(f64::NAN)
                }
            }
            V::Arr(a) if a.len() == 1 => a[0].num(),
            V::Arr(a) if a.is_empty() => 0.0,
            _ => f64::NAN,
        }
    }

    /// JavaScript truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            V::Undef => false,
            V::Num(n) => *n != 0.0 && !n.is_nan(),
            V::Bool(b) => *b,
            V::Str(s) => !s.is_empty(),
            V::Arr(_) | V::Obj(_) => true,
        }
    }

    /// JavaScript `String(v)`.
    pub fn to_js_string(&self) -> String {
        match self {
            V::Undef => "undefined".into(),
            V::Num(n) => fmt_num(*n),
            V::Bool(b) => b.to_string(),
            V::Str(s) => s.to_string(),
            V::Arr(a) => a.iter().map(V::to_js_string).collect::<Vec<_>>().join(","),
            V::Obj(_) => "[object Object]".into(),
        }
    }

    /// Numeric components: a number is one component, an array its elements.
    pub fn components(&self) -> Option<Vec<f64>> {
        match self {
            V::Arr(a) => Some(a.iter().map(V::num).collect()),
            V::Undef | V::Obj(_) => None,
            v => Some(vec![v.num()]),
        }
    }

    /// An array of numbers.
    pub fn nums(v: &[f64]) -> V {
        V::Arr(v.iter().map(|x| V::Num(*x)).collect())
    }
}

/// JavaScript number formatting for the common cases.
pub fn fmt_num(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else if n == 0.0 {
        "0".into()
    } else {
        format!("{n}")
    }
}

/// Built-in variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Var {
    Time,
    Frame,
    Value,
    Index,
    Count,
    Seed,
    TextIndex,
    TextTotal,
    Fps,
    Duration,
}

/// `loopIn`/`loopOut` modes (After Effects names).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum LoopKind {
    Cycle,
    PingPong,
    Offset,
    Continue,
}

/// Audio analysis bands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum Band {
    Full,
    Low,
    Mid,
    High,
}

impl Band {
    /// Parses `low`, `mid`, `high` or `full`.
    pub fn parse(s: &str) -> Option<Band> {
        match s {
            "full" | "" => Some(Band::Full),
            "low" => Some(Band::Low),
            "mid" => Some(Band::Mid),
            "high" => Some(Band::High),
            _ => None,
        }
    }
}

/// What the VM asks of its environment at run time.
pub trait Host {
    /// A built-in variable.
    fn var(&mut self, v: Var) -> V;
    /// Current value of a resolved property slot.
    fn prop(&mut self, slot: u32) -> V;
    /// Pre-expression value of this property at time `t`.
    fn value_at_time(&mut self, t: f64) -> V;
    /// Template parameter or repeat variable.
    fn param(&mut self, name: &str) -> V;
    /// `loopIn`/`loopOut` over this property's keyframes.
    fn loop_value(&mut self, out: bool, kind: LoopKind, keys: usize) -> V;
    /// Audio amplitude of a track in [0, 1].
    fn audio(&mut self, track: &str, band: Band) -> f64;
    /// Beat position (beats since the grid origin).
    fn beat(&mut self) -> f64;
    /// Uniform random in [0, 1) for a call site and component.
    fn random(&mut self, site: u32, component: u32) -> f64;
    /// Noise seed of this expression.
    fn noise_seed(&mut self) -> u64;
}

/// Compile-time name resolution.
pub trait Resolver {
    /// Resolves `prop("id.property")` to a slot.
    fn prop(&mut self, path: &str) -> Result<u32, String>;
    /// Resolves `markerTime("id")`.
    fn marker(&mut self, id: &str) -> Option<f64>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Func {
    Param,
    ValueAtTime,
    Wiggle,
    Noise,
    Random,
    LoopIn,
    LoopOut,
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    Clamp,
    Lerp,
    Smoothstep,
    Spring,
    Audio,
    Beat,
    Length,
    Normalize,
    Dot,
    Add,
    Sub,
    Mul,
    Div,
    DegToRad,
    RadToDeg,
    Number,
    String,
    IsNaN,
    Abs,
    Sign,
    Floor,
    Ceil,
    Round,
    Trunc,
    Min,
    Max,
    Pow,
    Sqrt,
    Cbrt,
    Exp,
    Log,
    Log2,
    Log10,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Hypot,
    MathRandom,
}

impl Func {
    fn lookup(name: &str) -> Option<(Func, usize, usize)> {
        use Func::*;
        const MANY: usize = 64;
        Some(match name {
            "param" => (Param, 1, 1),
            "valueAtTime" => (ValueAtTime, 1, 1),
            "wiggle" => (Wiggle, 2, 5),
            "noise" => (Noise, 1, 3),
            "random" => (Random, 0, 2),
            "loopIn" => (LoopIn, 0, 2),
            "loopOut" => (LoopOut, 0, 2),
            "linear" => (Linear, 3, 5),
            "ease" => (Ease, 3, 5),
            "easeIn" => (EaseIn, 3, 5),
            "easeOut" => (EaseOut, 3, 5),
            "clamp" => (Clamp, 3, 3),
            "lerp" => (Lerp, 3, 3),
            "smoothstep" => (Smoothstep, 3, 3),
            "spring" => (Spring, 1, 4),
            "audioAmplitude" => (Audio, 1, 2),
            "beat" => (Beat, 0, 0),
            "length" => (Length, 1, 2),
            "normalize" => (Normalize, 1, 1),
            "dot" => (Dot, 2, 2),
            "add" => (Add, 2, 2),
            "sub" => (Sub, 2, 2),
            "mul" => (Mul, 2, 2),
            "div" => (Div, 2, 2),
            "degreesToRadians" => (DegToRad, 1, 1),
            "radiansToDegrees" => (RadToDeg, 1, 1),
            "Number" | "parseFloat" => (Number, 1, 1),
            "String" => (String, 1, 1),
            "isNaN" => (IsNaN, 1, 1),
            "Math.abs" => (Abs, 1, 1),
            "Math.sign" => (Sign, 1, 1),
            "Math.floor" => (Floor, 1, 1),
            "Math.ceil" => (Ceil, 1, 1),
            "Math.round" => (Round, 1, 1),
            "Math.trunc" => (Trunc, 1, 1),
            "Math.min" => (Min, 0, MANY),
            "Math.max" => (Max, 0, MANY),
            "Math.pow" => (Pow, 2, 2),
            "Math.sqrt" => (Sqrt, 1, 1),
            "Math.cbrt" => (Cbrt, 1, 1),
            "Math.exp" => (Exp, 1, 1),
            "Math.log" => (Log, 1, 1),
            "Math.log2" => (Log2, 1, 1),
            "Math.log10" => (Log10, 1, 1),
            "Math.sin" => (Sin, 1, 1),
            "Math.cos" => (Cos, 1, 1),
            "Math.tan" => (Tan, 1, 1),
            "Math.asin" => (Asin, 1, 1),
            "Math.acos" => (Acos, 1, 1),
            "Math.atan" => (Atan, 1, 1),
            "Math.atan2" => (Atan2, 2, 2),
            "Math.hypot" => (Hypot, 0, MANY),
            "Math.random" => (MathRandom, 0, 0),
            _ => return None,
        })
    }
}

/// Every callable name, for suggestions.
pub const FUNCTION_NAMES: &[&str] = &[
    "param",
    "prop",
    "valueAtTime",
    "wiggle",
    "noise",
    "random",
    "loopIn",
    "loopOut",
    "linear",
    "ease",
    "easeIn",
    "easeOut",
    "clamp",
    "lerp",
    "smoothstep",
    "spring",
    "audioAmplitude",
    "beat",
    "markerTime",
    "length",
    "normalize",
    "dot",
    "add",
    "sub",
    "mul",
    "div",
    "degreesToRadians",
    "radiansToDegrees",
    "Number",
    "parseFloat",
    "String",
    "isNaN",
];

type Reg = u16;

#[derive(Debug, Clone, PartialEq)]
enum Op {
    Const(Reg, u32),
    Var(Reg, Var),
    Mov(Reg, Reg),
    Bin(BinOp, Reg, Reg, Reg),
    NumBin(BinOp, Reg, Reg, Reg),
    Un(UnOp, Reg, Reg),
    Jmp(u32),
    JmpIfNot(Reg, u32),
    JmpIf(Reg, u32),
    JmpIfNotNullish(Reg, u32),
    Call(Func, Reg, Reg, u8, u32),
    Arr(Reg, Reg, u16),
    Member(Reg, Reg, u32),
    Index(Reg, Reg, Reg),
    Prop(Reg, u32),
    Ret(Reg),
}

/// A compiled expression.
#[derive(Debug, Clone)]
pub struct Code {
    ops: Vec<Op>,
    consts: Vec<V>,
    nregs: usize,
    /// Property slots this expression reads through `prop()`.
    pub deps: Vec<u32>,
    /// True when the expression reads `value`, `valueAtTime`, `wiggle` or `loopIn/Out`.
    pub reads_value: bool,
    /// Number of `random` call sites.
    pub random_sites: u32,
}

/// A compile error with its source offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    /// Message.
    pub message: String,
    /// Byte offset in the source.
    pub offset: usize,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (at offset {})", self.message, self.offset)
    }
}

struct Compiler<'r> {
    ops: Vec<Op>,
    consts: Vec<V>,
    const_ix: HashMap<String, u32>,
    nregs: usize,
    scopes: Vec<HashMap<String, Reg>>,
    result: Reg,
    returns: Vec<usize>,
    deps: Vec<u32>,
    reads_value: bool,
    random_sites: u32,
    resolver: &'r mut dyn Resolver,
}

const MAX_REGS: usize = 4096;

impl<'r> Compiler<'r> {
    fn reg(&mut self, at: usize) -> Result<Reg, CompileError> {
        if self.nregs >= MAX_REGS {
            return Err(CompileError { message: "expression is too large".into(), offset: at });
        }
        self.nregs += 1;
        Ok((self.nregs - 1) as Reg)
    }

    fn konst(&mut self, v: V) -> u32 {
        let key = format!("{v:?}");
        if let Some(&i) = self.const_ix.get(&key) {
            return i;
        }
        self.consts.push(v);
        let i = (self.consts.len() - 1) as u32;
        self.const_ix.insert(key, i);
        i
    }

    fn lookup(&self, name: &str) -> Option<Reg> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn stmt(&mut self, s: &Stmt) -> Result<(), CompileError> {
        match s {
            Stmt::Let(name, e, at) => {
                let r = self.reg(*at)?;
                let v = self.expr(e)?;
                self.ops.push(Op::Mov(r, v));
                self.scopes.last_mut().unwrap().insert(name.clone(), r);
            }
            Stmt::Assign(name, op, e, at) => {
                // Assigning an undeclared name declares it (CONVENTIONS 5.1); it lives in the
                // outermost scope so it stays visible after the block that assigned it.
                let r = match self.lookup(name) {
                    Some(r) => r,
                    None => {
                        let r = self.reg(*at)?;
                        self.scopes[0].insert(name.clone(), r);
                        r
                    }
                };
                let v = self.expr(e)?;
                match op {
                    None => self.ops.push(Op::Mov(r, v)),
                    Some(b) => self.ops.push(Op::Bin(*b, r, r, v)),
                }
            }
            Stmt::If(c, then, els) => {
                let cr = self.expr(c)?;
                let jf = self.ops.len();
                self.ops.push(Op::JmpIfNot(cr, 0));
                self.scoped(then)?;
                if let Some(e) = els {
                    let jend = self.ops.len();
                    self.ops.push(Op::Jmp(0));
                    let here = self.ops.len() as u32;
                    self.ops[jf] = Op::JmpIfNot(cr, here);
                    self.scoped(e)?;
                    let end = self.ops.len() as u32;
                    self.ops[jend] = Op::Jmp(end);
                } else {
                    let here = self.ops.len() as u32;
                    self.ops[jf] = Op::JmpIfNot(cr, here);
                }
            }
            Stmt::Block(b) => {
                self.scopes.push(HashMap::new());
                for s in b {
                    self.stmt(s)?;
                }
                self.scopes.pop();
            }
            Stmt::Return(e) => {
                let r = match e {
                    Some(e) => self.expr(e)?,
                    None => {
                        let r = self.reg(0)?;
                        let k = self.konst(V::Undef);
                        self.ops.push(Op::Const(r, k));
                        r
                    }
                };
                self.ops.push(Op::Mov(self.result, r));
                self.returns.push(self.ops.len());
                self.ops.push(Op::Jmp(0));
            }
            Stmt::Expr(e) => {
                let r = self.expr(e)?;
                self.ops.push(Op::Mov(self.result, r));
            }
        }
        Ok(())
    }

    fn scoped(&mut self, s: &Stmt) -> Result<(), CompileError> {
        self.scopes.push(HashMap::new());
        self.stmt(s)?;
        self.scopes.pop();
        Ok(())
    }

    fn string_arg<'e>(&self, args: &'e [Expr], i: usize, f: &str, at: usize) -> Result<&'e str, CompileError> {
        match args.get(i) {
            Some(Expr::Str(s)) => Ok(s),
            _ => Err(CompileError { message: format!("{f}() needs a string literal argument"), offset: at }),
        }
    }

    fn expr(&mut self, e: &Expr) -> Result<Reg, CompileError> {
        Ok(match e {
            Expr::Num(n) => self.load(V::Num(*n))?,
            Expr::Str(s) => self.load(V::Str(s.clone()))?,
            Expr::Bool(b) => self.load(V::Bool(*b))?,
            Expr::Undefined => self.load(V::Undef)?,
            Expr::Array(items) => {
                let first = self.nregs as Reg;
                let regs: Vec<Reg> = (0..items.len()).map(|_| self.reg(0)).collect::<Result<_, _>>()?;
                for (it, r) in items.iter().zip(&regs) {
                    let v = self.expr(it)?;
                    self.ops.push(Op::Mov(*r, v));
                }
                let d = self.reg(0)?;
                self.ops.push(Op::Arr(d, first, items.len() as u16));
                d
            }
            Expr::Ident(name, at) => {
                if let Some(r) = self.lookup(name) {
                    return Ok(r);
                }
                let var = match name.as_str() {
                    "time" => Var::Time,
                    "frame" => Var::Frame,
                    "value" => {
                        self.reads_value = true;
                        Var::Value
                    }
                    "index" => Var::Index,
                    "count" => Var::Count,
                    "seed" => Var::Seed,
                    "textIndex" => Var::TextIndex,
                    "textTotal" => Var::TextTotal,
                    "fps" => Var::Fps,
                    "duration" => Var::Duration,
                    _ => {
                        let hint = crate::suggest(
                            name,
                            FUNCTION_NAMES.iter().copied().chain(["time", "frame", "value", "index", "count", "seed"]),
                        )
                        .map(|s| format!("; did you mean '{s}'?"))
                        .unwrap_or_default();
                        return Err(CompileError { message: format!("unknown name '{name}'{hint}"), offset: *at });
                    }
                };
                let r = self.reg(*at)?;
                self.ops.push(Op::Var(r, var));
                r
            }
            Expr::Unary(op, a) => {
                let a = self.expr(a)?;
                let d = self.reg(0)?;
                self.ops.push(Op::Un(*op, d, a));
                d
            }
            Expr::Binary(op, a, b) => {
                let (ra, rb) = (self.expr(a)?, self.expr(b)?);
                let d = self.reg(0)?;
                let numeric = is_numeric(a) && is_numeric(b);
                self.ops.push(if numeric { Op::NumBin(*op, d, ra, rb) } else { Op::Bin(*op, d, ra, rb) });
                d
            }
            Expr::Logical(op, a, b) => {
                let d = self.reg(0)?;
                let ra = self.expr(a)?;
                self.ops.push(Op::Mov(d, ra));
                let j = self.ops.len();
                self.ops.push(Op::Jmp(0));
                let rb = self.expr(b)?;
                self.ops.push(Op::Mov(d, rb));
                let end = self.ops.len() as u32;
                self.ops[j] = match op {
                    Logic::And => Op::JmpIfNot(d, end),
                    Logic::Or => Op::JmpIf(d, end),
                    Logic::Nullish => Op::JmpIfNotNullish(d, end),
                };
                d
            }
            Expr::Cond(c, a, b) => {
                let d = self.reg(0)?;
                let rc = self.expr(c)?;
                let jf = self.ops.len();
                self.ops.push(Op::JmpIfNot(rc, 0));
                let ra = self.expr(a)?;
                self.ops.push(Op::Mov(d, ra));
                let jend = self.ops.len();
                self.ops.push(Op::Jmp(0));
                let else_at = self.ops.len() as u32;
                let rb = self.expr(b)?;
                self.ops.push(Op::Mov(d, rb));
                let end = self.ops.len() as u32;
                self.ops[jf] = Op::JmpIfNot(rc, else_at);
                self.ops[jend] = Op::Jmp(end);
                d
            }
            Expr::Member(obj, name, at) => {
                if let Expr::Ident(o, _) = &**obj {
                    if o == "Math" && self.lookup("Math").is_none() {
                        let c = match name.as_str() {
                            "PI" => std::f64::consts::PI,
                            "E" => std::f64::consts::E,
                            "SQRT2" => std::f64::consts::SQRT_2,
                            "SQRT1_2" => std::f64::consts::FRAC_1_SQRT_2,
                            "LN2" => std::f64::consts::LN_2,
                            "LN10" => std::f64::consts::LN_10,
                            "LOG2E" => std::f64::consts::LOG2_E,
                            "LOG10E" => std::f64::consts::LOG10_E,
                            _ => {
                                return Err(CompileError {
                                    message: format!("Math.{name} is not a constant"),
                                    offset: *at,
                                });
                            }
                        };
                        return self.load(V::Num(c));
                    }
                }
                let o = self.expr(obj)?;
                let k = self.konst(V::Str(name.as_str().into()));
                let d = self.reg(*at)?;
                self.ops.push(Op::Member(d, o, k));
                d
            }
            Expr::Index(obj, idx) => {
                let (o, i) = (self.expr(obj)?, self.expr(idx)?);
                let d = self.reg(0)?;
                self.ops.push(Op::Index(d, o, i));
                d
            }
            Expr::Call(callee, args, at) => self.call(callee, args, *at)?,
        })
    }

    fn load(&mut self, v: V) -> Result<Reg, CompileError> {
        let k = self.konst(v);
        let r = self.reg(0)?;
        self.ops.push(Op::Const(r, k));
        Ok(r)
    }

    fn call(&mut self, callee: &Expr, args: &[Expr], at: usize) -> Result<Reg, CompileError> {
        let name = match callee {
            Expr::Ident(n, _) if self.lookup(n).is_none() => n.clone(),
            Expr::Member(o, m, _) if matches!(&**o, Expr::Ident(i, _) if i == "Math") => format!("Math.{m}"),
            _ => return Err(CompileError { message: "only built-in functions can be called".into(), offset: at }),
        };
        match name.as_str() {
            "prop" => {
                if args.len() != 1 {
                    return Err(CompileError { message: "prop() takes one argument".into(), offset: at });
                }
                let path = self.string_arg(args, 0, "prop", at)?.to_string();
                let slot = self.resolver.prop(&path).map_err(|m| CompileError { message: m, offset: at })?;
                if !self.deps.contains(&slot) {
                    self.deps.push(slot);
                }
                let d = self.reg(at)?;
                self.ops.push(Op::Prop(d, slot));
                return Ok(d);
            }
            "markerTime" => {
                if args.len() != 1 {
                    return Err(CompileError { message: "markerTime() takes one argument".into(), offset: at });
                }
                let id = self.string_arg(args, 0, "markerTime", at)?.to_string();
                let t = self
                    .resolver
                    .marker(&id)
                    .ok_or_else(|| CompileError { message: format!("no marker '{id}'"), offset: at })?;
                return self.load(V::Num(t));
            }
            _ => {}
        }
        let Some((f, lo, hi)) = Func::lookup(&name) else {
            let hint = crate::suggest(&name, FUNCTION_NAMES.iter().copied())
                .map(|s| format!("; did you mean '{s}'?"))
                .unwrap_or_default();
            return Err(CompileError { message: format!("unknown function '{name}'{hint}"), offset: at });
        };
        if args.len() < lo || args.len() > hi {
            let arity = if lo == hi {
                lo.to_string()
            } else if hi > 16 {
                format!("at least {lo}")
            } else {
                format!("{lo} to {hi}")
            };
            return Err(CompileError {
                message: format!("{name}() takes {arity} arguments, got {}", args.len()),
                offset: at,
            });
        }
        match f {
            Func::ValueAtTime | Func::Wiggle | Func::LoopIn | Func::LoopOut => self.reads_value = true,
            _ => {}
        }
        if f == Func::Audio {
            if let Some(Expr::Str(b)) = args.get(1) {
                if Band::parse(b).is_none() {
                    return Err(CompileError {
                        message: format!("unknown audio band {b:?}; use low, mid, high or full"),
                        offset: at,
                    });
                }
            }
        }
        if matches!(f, Func::LoopIn | Func::LoopOut) {
            if let Some(Expr::Str(k)) = args.first() {
                if loop_kind(k).is_none() {
                    return Err(CompileError {
                        message: format!("unknown loop type {k:?}; use cycle, pingpong, offset or continue"),
                        offset: at,
                    });
                }
            }
        }
        let first = self.nregs as Reg;
        let regs: Vec<Reg> = (0..args.len()).map(|_| self.reg(at)).collect::<Result<_, _>>()?;
        for (a, r) in args.iter().zip(&regs) {
            let v = self.expr(a)?;
            self.ops.push(Op::Mov(*r, v));
        }
        let site = if matches!(f, Func::Random | Func::MathRandom) {
            self.random_sites += 1;
            self.random_sites - 1
        } else {
            0
        };
        let d = self.reg(at)?;
        self.ops.push(Op::Call(f, d, first, args.len() as u8, site));
        Ok(d)
    }
}

fn is_numeric(e: &Expr) -> bool {
    match e {
        Expr::Num(_) => true,
        Expr::Unary(UnOp::Neg | UnOp::Plus, a) => is_numeric(a),
        Expr::Binary(op, a, b) => {
            matches!(op, BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem | BinOp::Pow | BinOp::Add)
                && is_numeric(a)
                && is_numeric(b)
        }
        Expr::Ident(n, _) => {
            matches!(n.as_str(), "time" | "frame" | "index" | "count" | "fps" | "duration" | "textIndex" | "textTotal")
        }
        Expr::Member(o, _, _) => matches!(&**o, Expr::Ident(i, _) if i == "Math"),
        _ => false,
    }
}

fn loop_kind(s: &str) -> Option<LoopKind> {
    match s {
        "cycle" | "" => Some(LoopKind::Cycle),
        "pingpong" | "ping-pong" => Some(LoopKind::PingPong),
        "offset" => Some(LoopKind::Offset),
        "continue" => Some(LoopKind::Continue),
        _ => None,
    }
}

/// Parses and compiles an expression.
pub fn compile(src: &str, resolver: &mut dyn Resolver) -> Result<Code, CompileError> {
    let prog = parse(src).map_err(|e| CompileError { message: e.message, offset: e.offset })?;
    let mut c = Compiler {
        ops: Vec::new(),
        consts: Vec::new(),
        const_ix: HashMap::new(),
        nregs: 1,
        scopes: vec![HashMap::new()],
        result: 0,
        returns: Vec::new(),
        deps: Vec::new(),
        reads_value: false,
        random_sites: 0,
        resolver,
    };
    let k = c.konst(V::Undef);
    c.ops.push(Op::Const(0, k));
    for s in &prog {
        c.stmt(s)?;
    }
    let end = c.ops.len() as u32;
    for &j in &c.returns {
        c.ops[j] = Op::Jmp(end);
    }
    c.ops.push(Op::Ret(0));
    Ok(Code {
        ops: c.ops,
        consts: c.consts,
        nregs: c.nregs,
        deps: c.deps,
        reads_value: c.reads_value,
        random_sites: c.random_sites,
    })
}

// ------------------------------------------------------------------ VM

fn num_bin(op: BinOp, a: f64, b: f64) -> V {
    match op {
        BinOp::Add => V::Num(a + b),
        BinOp::Sub => V::Num(a - b),
        BinOp::Mul => V::Num(a * b),
        BinOp::Div => V::Num(a / b),
        BinOp::Rem => V::Num(a % b),
        BinOp::Pow => V::Num(libm::pow(a, b)),
        BinOp::Lt => V::Bool(a < b),
        BinOp::Le => V::Bool(a <= b),
        BinOp::Gt => V::Bool(a > b),
        BinOp::Ge => V::Bool(a >= b),
        BinOp::Eq | BinOp::StrictEq => V::Bool(a == b),
        BinOp::Ne | BinOp::StrictNe => V::Bool(a != b),
    }
}

fn elementwise(op: BinOp, a: &V, b: &V) -> V {
    let (x, y) = (a.components().unwrap_or_default(), b.components().unwrap_or_default());
    let n = x.len().max(y.len());
    let pick = |v: &[f64], i: usize| if v.len() == 1 { v[0] } else { v.get(i).copied().unwrap_or(0.0) };
    V::Arr((0..n).map(|i| num_bin(op, pick(&x, i), pick(&y, i))).collect())
}

fn loose_eq(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Undef, V::Undef) => true,
        (V::Undef, _) | (_, V::Undef) => false,
        (V::Str(x), V::Str(y)) => x == y,
        (V::Arr(x), V::Arr(y)) => Arc::ptr_eq(x, y) || x == y,
        (V::Obj(x), V::Obj(y)) => Arc::ptr_eq(x, y),
        _ => a.num() == b.num(),
    }
}

fn strict_eq(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Num(x), V::Num(y)) => x == y,
        (V::Bool(x), V::Bool(y)) => x == y,
        (V::Str(x), V::Str(y)) => x == y,
        (V::Undef, V::Undef) => true,
        (V::Arr(x), V::Arr(y)) => Arc::ptr_eq(x, y) || x == y,
        (V::Obj(x), V::Obj(y)) => Arc::ptr_eq(x, y),
        _ => false,
    }
}

fn bin(op: BinOp, a: &V, b: &V) -> V {
    if let (V::Num(x), V::Num(y)) = (a, b) {
        return num_bin(op, *x, *y);
    }
    match op {
        BinOp::Eq => V::Bool(loose_eq(a, b)),
        BinOp::Ne => V::Bool(!loose_eq(a, b)),
        BinOp::StrictEq => V::Bool(strict_eq(a, b)),
        BinOp::StrictNe => V::Bool(!strict_eq(a, b)),
        BinOp::Add if matches!(a, V::Str(_)) || matches!(b, V::Str(_)) => {
            V::Str(format!("{}{}", a.to_js_string(), b.to_js_string()).into())
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => match (a, b) {
            (V::Str(x), V::Str(y)) => V::Bool(match op {
                BinOp::Lt => x < y,
                BinOp::Le => x <= y,
                BinOp::Gt => x > y,
                _ => x >= y,
            }),
            _ => num_bin(op, a.num(), b.num()),
        },
        _ if matches!(a, V::Arr(_)) || matches!(b, V::Arr(_)) => elementwise(op, a, b),
        _ => num_bin(op, a.num(), b.num()),
    }
}

fn map_num(v: &V, f: impl Fn(f64) -> f64) -> V {
    match v {
        V::Arr(a) => V::Arr(a.iter().map(|x| V::Num(f(x.num()))).collect()),
        other => V::Num(f(other.num())),
    }
}

fn interp(a: &V, b: &V, t: f64) -> V {
    match (a, b) {
        (V::Arr(_), _) | (_, V::Arr(_)) => {
            let (x, y) = (a.components().unwrap_or_default(), b.components().unwrap_or_default());
            let n = x.len().max(y.len());
            let pick = |v: &[f64], i: usize| if v.len() == 1 { v[0] } else { v.get(i).copied().unwrap_or(0.0) };
            V::Arr((0..n).map(|i| V::Num(pick(&x, i) + (pick(&y, i) - pick(&x, i)) * t)).collect())
        }
        _ => V::Num(a.num() + (b.num() - a.num()) * t),
    }
}

/// Runs compiled code. `regs` is scratch space reused across calls.
pub fn run(code: &Code, host: &mut dyn Host, regs: &mut Vec<V>) -> V {
    regs.clear();
    regs.resize(code.nregs, V::Undef);
    let mut pc = 0usize;
    loop {
        match &code.ops[pc] {
            Op::Const(d, k) => regs[*d as usize] = code.consts[*k as usize].clone(),
            Op::Var(d, v) => regs[*d as usize] = host.var(*v),
            Op::Mov(d, s) => regs[*d as usize] = regs[*s as usize].clone(),
            Op::NumBin(op, d, a, b) => {
                let r = match (&regs[*a as usize], &regs[*b as usize]) {
                    (V::Num(x), V::Num(y)) => num_bin(*op, *x, *y),
                    (x, y) => bin(*op, x, y),
                };
                regs[*d as usize] = r;
            }
            Op::Bin(op, d, a, b) => {
                let r = bin(*op, &regs[*a as usize], &regs[*b as usize]);
                regs[*d as usize] = r;
            }
            Op::Un(op, d, a) => {
                let v = &regs[*a as usize];
                regs[*d as usize] = match op {
                    UnOp::Not => V::Bool(!v.truthy()),
                    UnOp::Neg => map_num(v, |x| -x),
                    UnOp::Plus => match v {
                        V::Arr(_) => v.clone(),
                        other => V::Num(other.num()),
                    },
                };
            }
            Op::Jmp(t) => {
                pc = *t as usize;
                continue;
            }
            Op::JmpIfNot(c, t) => {
                if !regs[*c as usize].truthy() {
                    pc = *t as usize;
                    continue;
                }
            }
            Op::JmpIf(c, t) => {
                if regs[*c as usize].truthy() {
                    pc = *t as usize;
                    continue;
                }
            }
            Op::JmpIfNotNullish(c, t) => {
                if !matches!(regs[*c as usize], V::Undef) {
                    pc = *t as usize;
                    continue;
                }
            }
            Op::Arr(d, first, n) => {
                let a: Arc<[V]> = regs[*first as usize..*first as usize + *n as usize].iter().cloned().collect();
                regs[*d as usize] = V::Arr(a);
            }
            Op::Member(d, o, k) => {
                let V::Str(name) = &code.consts[*k as usize] else { unreachable!() };
                let r = match (&regs[*o as usize], &**name) {
                    (V::Arr(a), "length") => V::Num(a.len() as f64),
                    (V::Str(s), "length") => V::Num(s.chars().count() as f64),
                    (V::Arr(a), "x") => a.first().cloned().unwrap_or_default(),
                    (V::Arr(a), "y") => a.get(1).cloned().unwrap_or_default(),
                    (V::Arr(a), "z") => a.get(2).cloned().unwrap_or_default(),
                    (V::Obj(m), n) => m.get(n).cloned().unwrap_or_default(),
                    _ => V::Undef,
                };
                regs[*d as usize] = r;
            }
            Op::Index(d, o, i) => {
                let idx = regs[*i as usize].clone();
                let r = match (&regs[*o as usize], &idx) {
                    (V::Arr(a), i) => {
                        let n = i.num();
                        if n >= 0.0 && n.fract() == 0.0 {
                            a.get(n as usize).cloned().unwrap_or_default()
                        } else {
                            V::Undef
                        }
                    }
                    (V::Str(s), i) => {
                        let n = i.num();
                        if n >= 0.0 && n.fract() == 0.0 {
                            s.chars().nth(n as usize).map(|c| V::Str(c.to_string().into())).unwrap_or_default()
                        } else {
                            V::Undef
                        }
                    }
                    (V::Obj(m), k) => m.get(&k.to_js_string()).cloned().unwrap_or_default(),
                    _ => V::Undef,
                };
                regs[*d as usize] = r;
            }
            Op::Prop(d, slot) => regs[*d as usize] = host.prop(*slot),
            Op::Call(f, d, first, argc, site) => {
                let args = &regs[*first as usize..*first as usize + *argc as usize];
                let r = call(*f, args, *site, host);
                regs[*d as usize] = r;
            }
            Op::Ret(r) => return std::mem::take(&mut regs[*r as usize]),
        }
        pc += 1;
    }
}

fn arg(args: &[V], i: usize) -> f64 {
    args.get(i).map(V::num).unwrap_or(f64::NAN)
}

fn ease_args(args: &[V], f: impl Fn(f64) -> f64) -> V {
    // (t, v0, v1) with t in [0,1]; or (t, t0, t1, v0, v1)
    let (u, a, b) = if args.len() == 3 {
        (arg(args, 0), &args[1], &args[2])
    } else if args.len() == 5 {
        let (t, t0, t1) = (arg(args, 0), arg(args, 1), arg(args, 2));
        let u = if t1 == t0 {
            if t >= t1 {
                1.0
            } else {
                0.0
            }
        } else {
            (t - t0) / (t1 - t0)
        };
        (u, &args[3], &args[4])
    } else {
        return V::Undef;
    };
    interp(a, b, f(u.clamp(0.0, 1.0)))
}

fn call(f: Func, args: &[V], site: u32, host: &mut dyn Host) -> V {
    use Func::*;
    let m = |g: fn(f64) -> f64| map_num(args.first().unwrap_or(&V::Undef), g);
    match f {
        Param => host.param(&args[0].to_js_string()),
        ValueAtTime => host.value_at_time(arg(args, 0)),
        Wiggle => {
            let (freq, amp) = (arg(args, 0), &args[1]);
            let octaves = args.get(2).map(V::num).unwrap_or(1.0).clamp(1.0, 16.0) as u32;
            let mult = args.get(3).map(V::num).unwrap_or(0.5);
            let t = match args.get(4) {
                Some(v) => v.num(),
                None => host.var(Var::Time).num(),
            };
            let seed = host.noise_seed();
            let value = host.var(Var::Value);
            let base = value.components().unwrap_or_else(|| vec![0.0]);
            let amps = amp.components().unwrap_or_else(|| vec![0.0]);
            let out: Vec<f64> = base
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let a = if amps.len() == 1 { amps[0] } else { amps.get(i).copied().unwrap_or(0.0) };
                    b + a * rng::fbm1(rng::hash(&[seed, i as u64]), t * freq, octaves, mult)
                })
                .collect();
            if matches!(value, V::Arr(_)) {
                V::nums(&out)
            } else {
                V::Num(out[0])
            }
        }
        Noise => {
            let seed = host.noise_seed();
            let mut c = Vec::new();
            for a in args {
                c.extend(a.components().unwrap_or_default());
            }
            V::Num(match c.len() {
                0 => 0.0,
                1 => rng::noise1(seed, c[0]),
                2 => rng::noise2(seed, c[0], c[1]),
                _ => rng::noise3(seed, c[0], c[1], c[2]),
            })
        }
        Random | MathRandom => match args.len() {
            0 => V::Num(host.random(site, 0)),
            1 => match &args[0] {
                V::Arr(hi) => {
                    V::Arr(hi.iter().enumerate().map(|(i, h)| V::Num(host.random(site, i as u32) * h.num())).collect())
                }
                h => V::Num(host.random(site, 0) * h.num()),
            },
            _ => match (&args[0], &args[1]) {
                (V::Arr(_), _) | (_, V::Arr(_)) => {
                    let (lo, hi) = (args[0].components().unwrap_or_default(), args[1].components().unwrap_or_default());
                    let n = lo.len().max(hi.len());
                    let pick = |v: &[f64], i: usize| if v.len() == 1 { v[0] } else { v.get(i).copied().unwrap_or(0.0) };
                    V::Arr(
                        (0..n)
                            .map(|i| {
                                let (a, b) = (pick(&lo, i), pick(&hi, i));
                                V::Num(a + (b - a) * host.random(site, i as u32))
                            })
                            .collect(),
                    )
                }
                (a, b) => {
                    let (a, b) = (a.num(), b.num());
                    V::Num(a + (b - a) * host.random(site, 0))
                }
            },
        },
        LoopIn | LoopOut => {
            let kind = args
                .first()
                .map(|k| loop_kind(&k.to_js_string()).unwrap_or(LoopKind::Cycle))
                .unwrap_or(LoopKind::Cycle);
            let n = args.get(1).map(V::num).unwrap_or(0.0).max(0.0) as usize;
            host.loop_value(f == LoopOut, kind, n)
        }
        Linear => ease_args(args, |u| u),
        Ease => ease_args(args, |u| crate::curve::cubic_bezier(0.33, 0.0, 0.67, 1.0, u)),
        EaseIn => ease_args(args, |u| crate::curve::cubic_bezier(0.33, 0.0, 1.0, 1.0, u)),
        EaseOut => ease_args(args, |u| crate::curve::cubic_bezier(0.0, 0.0, 0.67, 1.0, u)),
        Clamp => {
            let (lo, hi) = (arg(args, 1), arg(args, 2));
            map_num(&args[0], move |x| x.max(lo).min(hi))
        }
        Lerp => interp(&args[0], &args[1], arg(args, 2)),
        Smoothstep => {
            let (e0, e1, x) = (arg(args, 0), arg(args, 1), arg(args, 2));
            let t = if e1 == e0 {
                if x < e0 {
                    0.0
                } else {
                    1.0
                }
            } else {
                ((x - e0) / (e1 - e0)).clamp(0.0, 1.0)
            };
            V::Num(t * t * (3.0 - 2.0 * t))
        }
        Spring => V::Num(crate::curve::spring(
            arg(args, 0),
            args.get(1).map(V::num).unwrap_or(100.0),
            args.get(2).map(V::num).unwrap_or(10.0),
            args.get(3).map(V::num).unwrap_or(1.0),
        )),
        Audio => {
            let band = args.get(1).and_then(|b| Band::parse(&b.to_js_string())).unwrap_or(Band::Full);
            V::Num(host.audio(&args[0].to_js_string(), band))
        }
        Beat => V::Num(host.beat()),
        Length => {
            let a = args[0].components().unwrap_or_default();
            let v: Vec<f64> = match args.get(1) {
                Some(b) => {
                    let b = b.components().unwrap_or_default();
                    (0..a.len().max(b.len()))
                        .map(|i| a.get(i).copied().unwrap_or(0.0) - b.get(i).copied().unwrap_or(0.0))
                        .collect()
                }
                None => a,
            };
            V::Num(libm::sqrt(v.iter().map(|x| x * x).sum()))
        }
        Normalize => {
            let a = args[0].components().unwrap_or_default();
            let l = libm::sqrt(a.iter().map(|x| x * x).sum());
            V::nums(&a.iter().map(|x| if l > 0.0 { x / l } else { 0.0 }).collect::<Vec<_>>())
        }
        Dot => {
            let (a, b) = (args[0].components().unwrap_or_default(), args[1].components().unwrap_or_default());
            V::Num(a.iter().zip(&b).map(|(x, y)| x * y).sum())
        }
        Add => elementwise(BinOp::Add, &args[0], &args[1]),
        Sub => elementwise(BinOp::Sub, &args[0], &args[1]),
        Mul => elementwise(BinOp::Mul, &args[0], &args[1]),
        Div => elementwise(BinOp::Div, &args[0], &args[1]),
        DegToRad => m(|x| x.to_radians()),
        RadToDeg => m(|x| x.to_degrees()),
        Number => V::Num(args[0].num()),
        String => V::Str(args[0].to_js_string().into()),
        IsNaN => V::Bool(args[0].num().is_nan()),
        Abs => m(f64::abs),
        Sign => m(|x| {
            if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                x
            }
        }),
        Floor => m(libm::floor),
        Ceil => m(libm::ceil),
        Round => m(|x| libm::floor(x + 0.5)),
        Trunc => m(libm::trunc),
        Min => V::Num(args.iter().map(V::num).fold(f64::INFINITY, |a, b| {
            if a.is_nan() || b.is_nan() {
                f64::NAN
            } else {
                a.min(b)
            }
        })),
        Max => V::Num(args.iter().map(V::num).fold(f64::NEG_INFINITY, |a, b| {
            if a.is_nan() || b.is_nan() {
                f64::NAN
            } else {
                a.max(b)
            }
        })),
        Pow => V::Num(libm::pow(arg(args, 0), arg(args, 1))),
        Sqrt => m(libm::sqrt),
        Cbrt => m(libm::cbrt),
        Exp => m(libm::exp),
        Log => m(libm::log),
        Log2 => m(libm::log2),
        Log10 => m(libm::log10),
        Sin => m(libm::sin),
        Cos => m(libm::cos),
        Tan => m(libm::tan),
        Asin => m(libm::asin),
        Acos => m(libm::acos),
        Atan => m(libm::atan),
        Atan2 => V::Num(libm::atan2(arg(args, 0), arg(args, 1))),
        Hypot => V::Num(libm::sqrt(args.iter().map(|a| a.num() * a.num()).sum())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoRes;
    impl Resolver for NoRes {
        fn prop(&mut self, path: &str) -> Result<u32, String> {
            if path == "a.x" {
                Ok(7)
            } else {
                Err(format!("no property {path}"))
            }
        }
        fn marker(&mut self, id: &str) -> Option<f64> {
            (id == "drop").then_some(4.2)
        }
    }

    struct H {
        t: f64,
        value: V,
    }
    impl Host for H {
        fn var(&mut self, v: Var) -> V {
            match v {
                Var::Time => V::Num(self.t),
                Var::Value => self.value.clone(),
                Var::Index => V::Num(2.0),
                _ => V::Num(0.0),
            }
        }
        fn prop(&mut self, slot: u32) -> V {
            V::Num(slot as f64 * 10.0)
        }
        fn value_at_time(&mut self, t: f64) -> V {
            V::Num(t * 100.0)
        }
        fn param(&mut self, name: &str) -> V {
            V::Str(format!("<{name}>").into())
        }
        fn loop_value(&mut self, _out: bool, _k: LoopKind, _n: usize) -> V {
            V::Num(-1.0)
        }
        fn audio(&mut self, _t: &str, _b: Band) -> f64 {
            0.5
        }
        fn beat(&mut self) -> f64 {
            3.5
        }
        fn random(&mut self, site: u32, c: u32) -> f64 {
            rng::unit(rng::hash(&[site as u64, c as u64]))
        }
        fn noise_seed(&mut self) -> u64 {
            1
        }
    }

    fn eval(src: &str) -> V {
        let code = compile(src, &mut NoRes).unwrap_or_else(|e| panic!("{src}: {e}"));
        run(&code, &mut H { t: 2.0, value: V::nums(&[10.0, 20.0]) }, &mut Vec::new())
    }

    #[test]
    fn arithmetic_and_control_flow() {
        assert_eq!(eval("1 + 2 * 3"), V::Num(7.0));
        assert_eq!(eval("2 ** 3 ** 2"), V::Num(512.0));
        assert_eq!(eval("time * 10 + index"), V::Num(22.0));
        assert_eq!(eval("let a = 1; a += 4; a"), V::Num(5.0));
        assert_eq!(eval("if (time > 1) { 'late' } else { 'early' }"), V::Str("late".into()));
        assert_eq!(eval("if (time > 5) 1; else return 2; 3"), V::Num(2.0));
        assert_eq!(eval("0 || 'x'"), V::Str("x".into()));
        assert_eq!(eval("undefined ?? 4"), V::Num(4.0));
        assert_eq!(eval("0 ?? 4"), V::Num(0.0));
        assert_eq!(eval("time > 1 ? [1,2] : 0"), V::nums(&[1.0, 2.0]));
        assert_eq!(eval("'n=' + 2.5"), V::Str("n=2.5".into()));
        assert_eq!(eval("'2' == 2"), V::Bool(true));
        assert_eq!(eval("'2' === 2"), V::Bool(false));
    }

    #[test]
    fn vectors_and_builtins() {
        assert_eq!(eval("value + [1, 1]"), V::nums(&[11.0, 21.0]));
        assert_eq!(eval("value * 2"), V::nums(&[20.0, 40.0]));
        assert_eq!(eval("value[1] + value.length"), V::Num(22.0));
        assert_eq!(eval("length([3, 4])"), V::Num(5.0));
        assert_eq!(eval("clamp(12, 0, 10)"), V::Num(10.0));
        assert_eq!(eval("linear(time, 0, 4, 0, 100)"), V::Num(50.0));
        assert_eq!(eval("prop('a.x') + markerTime('drop')"), V::Num(74.2));
        assert_eq!(eval("valueAtTime(time - 1)"), V::Num(100.0));
        assert_eq!(eval("Math.max(1, 5, 3) + Math.PI * 0"), V::Num(5.0));
        assert_eq!(eval("param('city')"), V::Str("<city>".into()));
        assert_eq!(eval("audioAmplitude('music', 'low') + beat()"), V::Num(4.0));
        let r = eval("random(10, 20)").num();
        assert!((10.0..20.0).contains(&r));
        assert_eq!(eval("random(10, 20)"), eval("random(10, 20)"), "deterministic");
        let w = eval("wiggle(2, 30)");
        let V::Arr(w) = w else { panic!() };
        assert!((w[0].num() - 10.0).abs() <= 30.0 && w[0].num() != 10.0);
    }

    #[test]
    fn bare_assignment_declares() {
        assert_eq!(eval("t0 = 1.5; x = time - t0\n x * 2"), V::Num(1.0));
        assert_eq!(eval("if (time > 1) { k = 3 } k"), V::Num(3.0));
        assert_eq!(eval("[4, 5, 6][index - 1]"), V::Num(5.0));
        assert_eq!(eval("[4, 5, 6][index + 1]"), V::Undef);
    }

    #[test]
    fn compile_errors() {
        for (src, msg) in [
            ("tim + 1", "did you mean 'time'"),
            ("wigle(1, 2)", "did you mean 'wiggle'"),
            ("prop(name)", "string literal"),
            ("prop('b.y')", "no property b.y"),
            ("markerTime('nope')", "no marker"),
            ("clamp(1, 2)", "takes 3 arguments"),
            ("loopOut('bounce')", "unknown loop type"),
            ("audioAmplitude('m', 'bass')", "unknown audio band"),
            ("Math.TAU", "not a constant"),
            ("value.map(1)", "only built-in functions"),
        ] {
            let e = compile(src, &mut NoRes).unwrap_err();
            assert!(e.message.contains(msg), "{src}: {}", e.message);
        }
    }

    #[test]
    fn deps_are_collected() {
        let c = compile("prop('a.x') * 2 + prop('a.x')", &mut NoRes).unwrap();
        assert_eq!(c.deps, vec![7]);
        assert!(!c.reads_value);
        assert!(compile("wiggle(1, 2)", &mut NoRes).unwrap().reads_value);
    }
}
