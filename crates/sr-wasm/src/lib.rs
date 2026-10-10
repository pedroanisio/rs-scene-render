//! # sr-wasm
//!
//! Runs the WebAssembly modules of `<program>` elements under the determinism rules of SREP 66, in the two modes
//! of SREPs 66 and 69:
//!
//! * [`generate`]: build mode. One call of the module's `generate` export, whose result is the output bytes.
//! * [`Stepper`]: step mode. `init` once, `step` per step, `frame` for the picture.
//!
//! The rules (SREP 66, Determinism 1–6), and how this crate meets each one with Wasmtime 41:
//!
//! 1. **Canonical NaNs**: `Config::cranelift_nan_canonicalization`. Cranelift rewrites every NaN that a float
//!    arithmetic instruction produces, scalar or SIMD, to the positive canonical NaN.
//! 2. **No relaxed SIMD and no threads**: the `threads` feature is not compiled in, and relaxed SIMD is switched off.
//!    A module that uses either fails validation, which is [`Code::Prg11`].
//! 3. **A fixed memory limit**: a [`wasmtime::ResourceLimiter`] that allows growth up to `memoryLimit` MiB.
//!    `memory.grow` returns −1 beyond that limit. A growth the host itself fails to provide traps
//!    ([`Code::Prg13`]); it never returns −1.
//! 4. **Fuel**: `Config::consume_fuel`, with the pinned compiler patch removing its synthetic function-entry charge.
//!    Wasmtime charges 1 per instruction, and 0 for nop, drop, block, loop,
//!    unreachable, return, else and end. It checks the remaining fuel only at function entries and loop headers.
//!    So each call is given `fuel + 1`, and a call fails ([`Code::Prg12`]) when it traps out of fuel, or when it
//!    returns with none left. That outcome is exactly "the call spent more than `fuel`".
//! 5. **Imports**: only `sr.rand_u64`, `sr.rand_f64`, `sr.param_f64` and `sr.param_str`.
//! 6. **Cache**: [`cache_key`] gives the key. The caller stores and looks up outputs.

#![warn(missing_docs)]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use sha2::{Digest, Sha256};
use wasmtime::{Caller, Config, Engine, ExternType, Instance, Linker, Module, ResourceLimiter, Store, Trap, Val};

/// Wasm page size.
const PAGE: usize = 64 * 1024;
/// Largest output a program may return.
pub const MAX_OUTPUT: usize = 256 << 20;
/// Stack available to a module (fixed so that recursion depth does not depend on a host setting).
const STACK: usize = 1 << 20;

/// The diagnostic codes of SREPs 66 and 69 that running a module can raise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    /// The module is not usable: invalid, a forbidden feature or import, a missing export, too much memory.
    Prg11,
    /// Fuel exhausted.
    Prg12,
    /// A trap, or the host could not provide memory.
    Prg13,
    /// The output is unusable.
    Prg14,
    /// `sr.param_f64` read a value that is not a number.
    Prg16,
}

impl Code {
    /// The code as it appears in diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Code::Prg11 => "PRG11",
            Code::Prg12 => "PRG12",
            Code::Prg13 => "PRG13",
            Code::Prg14 => "PRG14",
            Code::Prg16 => "PRG16",
        }
    }
}

/// Why a program failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}: {message}", code.as_str())]
pub struct Error {
    /// Diagnostic code.
    pub code: Code,
    /// What happened.
    pub message: String,
}

fn err(code: Code, message: impl Into<String>) -> Error {
    Error { code, message: message.into() }
}

/// What a program reads from its document.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    /// `project/@seed`.
    pub project_seed: u64,
    /// `program/@seed`.
    pub seed: u64,
    /// The `param` children, in document order (names are unique: rule PRG1).
    pub params: Vec<(String, String)>,
}

/// The limits a document gives a program.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Instructions one call may execute (`program/@fuel`).
    pub fuel: u64,
    /// Linear memory in MiB (`program/@memoryLimit`).
    pub memory_mib: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { fuel: 1_000_000_000, memory_mib: 64 }
    }
}

// ------------------------------------------------------------------ the generator (SREP 66, Semantics 4)

/// The SplitMix64 finaliser.
pub fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// SplitMix64 seeded with `mix64(project_seed ^ mix64(seed))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rng(u64);

impl Rng {
    /// The generator of a program.
    pub fn new(project_seed: u64, seed: u64) -> Rng {
        Rng(mix64(project_seed ^ mix64(seed)))
    }

    /// The next value.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix64(self.0)
    }
}

// ------------------------------------------------------------------ the engine

fn engine() -> &'static Engine {
    static E: OnceLock<Engine> = OnceLock::new();
    E.get_or_init(|| {
        let mut c = Config::new();
        c.consume_fuel(true);
        c.cranelift_nan_canonicalization(true);
        c.wasm_relaxed_simd(false);
        c.wasm_simd(true);
        c.wasm_multi_memory(false);
        c.wasm_memory64(false);
        c.max_wasm_stack(STACK);
        Engine::new(&c).expect("the WebAssembly engine configuration is valid")
    })
}

/// Compiled modules by the SHA-256 of their bytes.
fn compiled(bytes: &[u8]) -> Result<Module, Error> {
    static CACHE: OnceLock<Mutex<HashMap<[u8; 32], Module>>> = OnceLock::new();
    let key: [u8; 32] = Sha256::digest(bytes).into();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(m) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(m.clone());
    }
    let m = Module::new(engine(), bytes).map_err(|e| err(Code::Prg11, format!("invalid module: {e:#}")))?;
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, m.clone());
    Ok(m)
}

/// What the host functions see.
struct Host {
    rng: Rng,
    params: Vec<(String, String)>,
    memory_bytes: usize,
    /// A host-side failure to report instead of the trap it causes.
    failure: Option<Error>,
}

impl ResourceLimiter for Host {
    fn memory_growing(&mut self, _current: usize, desired: usize, maximum: Option<usize>) -> wasmtime::Result<bool> {
        // beyond the module's own maximum or the document's limit, memory.grow returns -1: both are fixed by the
        // module and the document, so the outcome is the same on every host
        Ok(desired <= self.memory_bytes && maximum.is_none_or(|m| desired <= m))
    }

    fn memory_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        // permitted above, so the host could not provide it: trap (PRG13), never -1 (Determinism 3)
        self.failure = Some(err(Code::Prg13, format!("the host could not provide memory below the limit: {error}")));
        Err(error)
    }

    fn table_growing(&mut self, _current: usize, desired: usize, maximum: Option<usize>) -> wasmtime::Result<bool> {
        Ok(desired <= 1 << 20 && maximum.is_none_or(|m| desired <= m))
    }

    fn instances(&self) -> usize {
        1
    }

    fn tables(&self) -> usize {
        16
    }

    fn memories(&self) -> usize {
        1
    }
}

const IMPORTS: [&str; 4] = ["rand_u64", "rand_f64", "param_f64", "param_str"];

fn linker() -> Result<Linker<Host>, Error> {
    let mut l = Linker::new(engine());
    let wire = |e: wasmtime::Error| err(Code::Prg11, format!("host: {e}"));
    l.func_wrap("sr", "rand_u64", |mut c: Caller<'_, Host>| c.data_mut().rng.next() as i64).map_err(wire)?;
    l.func_wrap("sr", "rand_f64", |mut c: Caller<'_, Host>| (c.data_mut().rng.next() >> 11) as f64 * (-53f64).exp2())
        .map_err(wire)?;
    l.func_wrap(
        "sr",
        "param_f64",
        |mut c: Caller<'_, Host>, ptr: i32, len: i32, default: f64| -> wasmtime::Result<f64> {
            let name = read_str(&mut c, ptr, len)?;
            let value = c.data().params.iter().find(|(n, _)| *n == name).map(|(_, v)| v.clone());
            match value {
                None => Ok(default),
                Some(v) => match parse_double(&v) {
                    Some(x) => Ok(x),
                    None => {
                        let e = err(Code::Prg16, format!("parameter {name:?} is not a number: {v:?}"));
                        c.data_mut().failure = Some(e.clone());
                        Err(wasmtime::Error::msg(e.to_string()))
                    }
                },
            }
        },
    )
    .map_err(wire)?;
    l.func_wrap(
        "sr",
        "param_str",
        |mut c: Caller<'_, Host>, name_ptr: i32, name_len: i32, buf_ptr: i32, buf_cap: i32| -> wasmtime::Result<i32> {
            let name = read_str(&mut c, name_ptr, name_len)?;
            let Some(value) = c.data().params.iter().find(|(n, _)| *n == name).map(|(_, v)| v.clone()) else {
                return Ok(-1);
            };
            let bytes = value.as_bytes();
            let n = bytes.len().min(buf_cap.max(0) as usize);
            let mem = memory(&mut c)?;
            let start = buf_ptr as u32 as usize;
            match mem.data_mut(&mut c).get_mut(start..start + n) {
                Some(dst) => dst.copy_from_slice(&bytes[..n]),
                None => return Err(host_trap(&mut c, "param_str: the buffer is outside the module's memory")),
            }
            Ok(i32::try_from(bytes.len()).unwrap_or(i32::MAX))
        },
    )
    .map_err(wire)?;
    Ok(l)
}

fn host_trap(c: &mut Caller<'_, Host>, message: &str) -> wasmtime::Error {
    c.data_mut().failure = Some(err(Code::Prg13, message));
    wasmtime::Error::msg(message.to_string())
}

fn memory(c: &mut Caller<'_, Host>) -> wasmtime::Result<wasmtime::Memory> {
    match c.get_export("memory") {
        Some(wasmtime::Extern::Memory(m)) => Ok(m),
        _ => Err(host_trap(c, "the module exports no memory")),
    }
}

fn read_str(c: &mut Caller<'_, Host>, ptr: i32, len: i32) -> wasmtime::Result<String> {
    let mem = memory(c)?;
    let (start, n) = (ptr as u32 as usize, len as u32 as usize);
    let bytes = mem.data(&*c).get(start..start + n).map(<[u8]>::to_vec);
    let bytes = bytes.ok_or_else(|| host_trap(c, "a parameter name is outside the module's memory"))?;
    String::from_utf8(bytes).map_err(|_| host_trap(c, "a parameter name is not UTF-8"))
}

/// An `xs:double`: optional sign, digits with an optional point, optional exponent; or INF, -INF, NaN.
fn parse_double(s: &str) -> Option<f64> {
    let t = s.trim_matches([' ', '\t', '\n', '\r']);
    match t {
        "INF" | "+INF" => return Some(f64::INFINITY),
        "-INF" => return Some(f64::NEG_INFINITY),
        "NaN" => return Some(f64::NAN),
        _ => {}
    }
    let ok = !t.is_empty()
        && t.chars().all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'))
        && t.chars().any(|c| c.is_ascii_digit());
    if ok {
        t.parse().ok()
    } else {
        None
    }
}

/// Checks the module against SREP 66 before it runs: only the four imports, the exports of the mode.
fn check_interface(m: &Module, exports: &[(&str, &str)]) -> Result<(), Error> {
    for i in m.imports() {
        let allowed = i.module() == "sr" && IMPORTS.contains(&i.name()) && matches!(i.ty(), ExternType::Func(_));
        if !allowed {
            return Err(err(
                Code::Prg11,
                format!("the module imports {}.{}; only sr.rand_u64, sr.rand_f64, sr.param_f64 and sr.param_str are allowed", i.module(), i.name()),
            ));
        }
    }
    for (name, sig) in exports {
        let ok = match (m.get_export(name), *sig) {
            (Some(ExternType::Memory(mt)), "memory") => !mt.is_shared() && !mt.is_64(),
            (Some(ExternType::Func(f)), sig) => func_sig(&f) == sig,
            _ => false,
        };
        if !ok {
            return Err(err(Code::Prg11, format!("the module must export {name} ({sig})")));
        }
    }
    Ok(())
}

fn func_sig(f: &wasmtime::FuncType) -> String {
    let t = |v: wasmtime::ValType| match v {
        wasmtime::ValType::I32 => "i32",
        wasmtime::ValType::I64 => "i64",
        wasmtime::ValType::F32 => "f32",
        wasmtime::ValType::F64 => "f64",
        _ => "other",
    };
    let p: Vec<_> = f.params().map(t).collect();
    let r: Vec<_> = f.results().map(t).collect();
    format!("[{}] -> [{}]", p.join(" "), r.join(" "))
}

/// A live module with its store.
struct Live {
    store: Store<Host>,
    instance: Instance,
    fuel: u64,
}

impl Live {
    fn new(bytes: &[u8], inputs: &Inputs, limits: Limits, exports: &[(&str, &str)]) -> Result<Live, Error> {
        let module = compiled(bytes)?;
        check_interface(&module, exports)?;
        let memory_bytes = (limits.memory_mib as usize).saturating_mul(16 * PAGE);
        let host = Host {
            rng: Rng::new(inputs.project_seed, inputs.seed),
            params: inputs.params.clone(),
            memory_bytes,
            failure: None,
        };
        let mut store = Store::new(engine(), host);
        store.limiter(|h| h as &mut dyn ResourceLimiter);
        for export in module.exports().filter(|e| e.name() == "memory") {
            if let ExternType::Memory(mt) = export.ty() {
                if (mt.minimum() as usize).saturating_mul(PAGE) > memory_bytes {
                    return Err(err(
                        Code::Prg11,
                        format!("the module declares {} pages of memory, above memoryLimit", mt.minimum()),
                    ));
                }
            }
        }
        // instantiation runs data and element segments, and a start function if there is one: give it fuel too
        store.set_fuel(limits.fuel.saturating_add(1)).map_err(|e| err(Code::Prg11, e.to_string()))?;
        let instance = match linker()?.instantiate(&mut store, &module) {
            Ok(i) => i,
            Err(e) => return Err(classify(&mut store, e, limits.fuel, "instantiate")),
        };
        Ok(Live { store, instance, fuel: limits.fuel })
    }

    /// Calls an export with fuel + 1, and classifies the outcome (Determinism 4).
    fn call(&mut self, name: &str, args: &[Val], results: &mut [Val]) -> Result<(), Error> {
        let f = self
            .instance
            .get_func(&mut self.store, name)
            .ok_or_else(|| err(Code::Prg11, format!("no export {name}")))?;
        self.store.set_fuel(self.fuel.saturating_add(1)).map_err(|e| err(Code::Prg13, e.to_string()))?;
        self.store.data_mut().failure = None;
        match f.call(&mut self.store, args, results) {
            Ok(()) if self.store.get_fuel().unwrap_or(0) == 0 => {
                Err(err(Code::Prg12, format!("{name} spent more than its fuel of {} instructions", self.fuel)))
            }
            Ok(()) => Ok(()),
            Err(e) => Err(classify(&mut self.store, e, self.fuel, name)),
        }
    }

    /// The bytes at `(ptr << 32) | len`, as a call returned it.
    fn output(&mut self, packed: i64) -> Result<Vec<u8>, Error> {
        let packed = packed as u64;
        let (ptr, len) = ((packed >> 32) as usize, (packed & 0xFFFF_FFFF) as usize);
        if len > MAX_OUTPUT {
            return Err(err(Code::Prg14, format!("the output is {len} bytes, above {MAX_OUTPUT}")));
        }
        let mem = self
            .instance
            .get_memory(&mut self.store, "memory")
            .ok_or_else(|| err(Code::Prg11, "the module exports no memory"))?;
        mem.data(&self.store)
            .get(ptr..ptr + len)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| err(Code::Prg14, format!("the output ({ptr}, {len}) lies outside the module's memory")))
    }
}

fn classify(store: &mut Store<Host>, e: wasmtime::Error, fuel: u64, what: &str) -> Error {
    if let Some(f) = store.data_mut().failure.take() {
        return f;
    }
    match e.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel) => err(Code::Prg12, format!("{what} spent more than its fuel of {fuel} instructions")),
        Some(t) => err(Code::Prg13, format!("{what} trapped: {t}")),
        None => err(Code::Prg13, format!("{what} failed: {e:#}")),
    }
}

/// Build mode (SREP 66): runs `generate` once and returns the output bytes.
pub fn generate(module: &[u8], inputs: &Inputs, limits: Limits) -> Result<Vec<u8>, Error> {
    let mut live = Live::new(module, inputs, limits, &[("memory", "memory"), ("generate", "[] -> [i64]")])?;
    let mut out = [Val::I64(0)];
    live.call("generate", &[], &mut out)?;
    let Val::I64(packed) = out[0] else { return Err(err(Code::Prg11, "generate returned no i64")) };
    live.output(packed)
}

/// Step mode (SREP 69): a live module that advances step by step.
pub struct Stepper {
    live: Live,
    width: u32,
    height: u32,
    steps: u64,
}

impl Stepper {
    /// Instantiates the module and calls `init`.
    pub fn new(module: &[u8], inputs: &Inputs, limits: Limits, width: u32, height: u32) -> Result<Stepper, Error> {
        let mut live = Live::new(
            module,
            inputs,
            limits,
            &[("memory", "memory"), ("init", "[] -> []"), ("step", "[i64] -> []"), ("frame", "[] -> [i64]")],
        )?;
        live.call("init", &[], &mut [])?;
        Ok(Stepper { live, width, height, steps: 0 })
    }

    /// Steps taken so far.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// Calls `step` with the next step index.
    pub fn step(&mut self) -> Result<(), Error> {
        self.live.call("step", &[Val::I64(self.steps as i64)], &mut [])?;
        self.steps += 1;
        Ok(())
    }

    /// Calls `frame`: the RGBA picture, `width · height · 4` bytes, rows from the top, straight alpha.
    pub fn frame(&mut self) -> Result<Vec<u8>, Error> {
        let mut out = [Val::I64(0)];
        self.live.call("frame", &[], &mut out)?;
        let Val::I64(packed) = out[0] else { return Err(err(Code::Prg11, "frame returned no i64")) };
        let bytes = self.live.output(packed)?;
        let want = self.width as usize * self.height as usize * 4;
        if bytes.len() != want {
            return Err(err(
                Code::Prg14,
                format!("frame returned {} bytes; a {}x{} picture is {want}", bytes.len(), self.width, self.height),
            ));
        }
        Ok(bytes)
    }
}

/// The cache key of a program's output (SREP 66, Determinism 6), as lowercase hex.
pub fn cache_key(module_sha256: &str, inputs: &Inputs, limits: Limits) -> String {
    let mut params: Vec<&(String, String)> = inputs.params.iter().collect();
    params.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut h = Sha256::new();
    for part in [
        "sr-program-1",
        &module_sha256.to_ascii_lowercase(),
        &inputs.project_seed.to_string(),
        &inputs.seed.to_string(),
        &limits.fuel.to_string(),
        &limits.memory_mib.to_string(),
    ] {
        h.update(part.as_bytes());
        h.update([0]);
    }
    for (n, v) in params {
        h.update(n.as_bytes());
        h.update([0]);
        h.update(v.as_bytes());
        h.update([0]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_matches_the_reference_sequence() {
        // Vigna's splitmix64.c with x = 0: the first output is mix64(0x9E3779B97F4A7C15)
        let mut r = Rng(0);
        assert_eq!(r.next(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(r.next(), 0x6E78_9E6A_A1B9_65F4);
    }

    #[test]
    fn doubles_parse_as_xs_double() {
        assert_eq!(parse_double(" 1.5e2 "), Some(150.0));
        assert_eq!(parse_double("-INF"), Some(f64::NEG_INFINITY));
        assert_eq!(parse_double("1,5"), None);
        assert_eq!(parse_double("abc"), None);
    }
}
