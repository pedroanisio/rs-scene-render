//! SREPs 66 and 69 against the kit's own modules (`tests/modules`, copied from sr-core
//! `conformance/srep_cases/assets`, written by its `tools/srep66_cases.py` and `tools/srep69_cases.py`), and the
//! determinism rules against small text modules.

use sr_wasm::{cache_key, generate, Code, Inputs, Limits, Stepper};

const RED: &str = r##"<shape id="g1" shape="rect" width="40" height="20" anchorX="20" anchorY="10" x="480" y="180" fill="#FF0000FF"/>"##;
const BLUE: &str = r##"<shape id="g1" shape="rect" width="40" height="20" anchorX="20" anchorY="10" x="160" y="180" fill="#0000FFFF"/>"##;

fn module(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/modules/{name}.wasm", env!("CARGO_MANIFEST_DIR"))).expect("kit module")
}

fn wat(text: &str) -> Vec<u8> {
    wat::parse_str(text).expect("valid text module")
}

fn run(name: &str, inputs: Inputs, limits: Limits) -> Result<String, sr_wasm::Error> {
    generate(&module(name), &inputs, limits).map(|b| String::from_utf8(b).expect("UTF-8"))
}

fn seeds(project_seed: u64, seed: u64) -> Inputs {
    Inputs { project_seed, seed, params: Vec::new() }
}

fn params(p: &[(&str, &str)]) -> Inputs {
    Inputs { project_seed: 1, seed: 0, params: p.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect() }
}

#[test]
fn a_constant_fragment_comes_back_as_written() {
    let out = run("srep66-rect", Inputs::default(), Limits::default()).unwrap();
    assert!(out.starts_with(r#"<shape id="g1" shape="rect""#) && out.contains(r#"x="200" y="120""#), "{out}");
}

#[test]
fn the_generator_follows_splitmix64_from_the_seeds() {
    // project seed 1: program seed 4 gives an even first value (red), seed 1 an odd one (blue); the kit chose them
    assert_eq!(run("srep66-seed", seeds(1, 4), Limits::default()).unwrap(), RED);
    assert_eq!(run("srep66-seed", seeds(1, 1), Limits::default()).unwrap(), BLUE);
    let mut r = sr_wasm::Rng::new(1, 4);
    assert_eq!(r.next() & 1, 0);
}

#[test]
fn parameters_are_read_and_absent_ones_give_the_default() {
    assert_eq!(run("srep66-param", params(&[("side", "1")]), Limits::default()).unwrap(), RED);
    assert_eq!(run("srep66-param", params(&[]), Limits::default()).unwrap(), BLUE);
}

#[test]
fn a_parameter_that_is_not_a_number_fails_prg16() {
    let e = run("srep66-param", params(&[("side", "wide")]), Limits::default()).unwrap_err();
    assert_eq!(e.code, Code::Prg16, "{e}");
}

#[test]
fn zero_over_zero_is_the_positive_canonical_nan() {
    // V8 on x86-64 takes the blue branch (0xFFF8...); Determinism 1 requires the red one
    assert_eq!(run("srep66-nan", params(&[("z", "0")]), Limits::default()).unwrap(), RED);
}

#[test]
fn memory_grows_up_to_the_limit_and_no_further() {
    // the module asks for 1100 pages (68.75 MiB): -1 under 64 MiB (red), granted under 128 MiB (blue)
    assert_eq!(run("srep66-grow", Inputs::default(), Limits { memory_mib: 64, ..Limits::default() }).unwrap(), RED);
    assert_eq!(run("srep66-grow", Inputs::default(), Limits { memory_mib: 128, ..Limits::default() }).unwrap(), BLUE);
}

#[test]
fn an_endless_loop_runs_out_of_fuel() {
    let e = run("srep66-loop", Inputs::default(), Limits { fuel: 1_000_000, ..Limits::default() }).unwrap_err();
    assert_eq!(e.code, Code::Prg12, "{e}");
}

#[test]
fn forbidden_imports_and_features_are_refused_before_running() {
    for m in ["srep66-wasi", "srep66-shared", "srep66-relaxed"] {
        let e = run(m, Inputs::default(), Limits::default()).unwrap_err();
        assert_eq!(e.code, Code::Prg11, "{m}: {e}");
    }
}

#[test]
fn data_output_is_returned_as_bytes() {
    assert_eq!(run("srep66-rows", Inputs::default(), Limits::default()).unwrap(), r#"[{"a":1},{"a":2}]"#);
}

/// A module whose generate executes exactly `n` charged instructions: `n - 1` constants dropped (each const is
/// charged, each drop is free), then the result constant.
fn counted(n: u32) -> Vec<u8> {
    let body = "i32.const 0 drop ".repeat(n as usize - 1);
    wat(&format!(r#"(module (memory (export "memory") 1) (func (export "generate") (result i64) {body} i64.const 0))"#))
}

#[test]
fn fuel_is_exact_in_straight_line_code() {
    // Wasmtime checks fuel only at function entry and loop headers; the rule is still exact: n passes, n - 1 fails
    for n in [1u32, 2, 7, 50] {
        assert!(
            generate(&counted(n), &Inputs::default(), Limits { fuel: n as u64, ..Limits::default() }).is_ok(),
            "{n}"
        );
        if n > 1 {
            let e = generate(&counted(n), &Inputs::default(), Limits { fuel: n as u64 - 1, ..Limits::default() })
                .unwrap_err();
            assert_eq!(e.code, Code::Prg12, "{n}: {e}");
        }
    }
}

#[test]
fn nested_calls_pay_for_instructions_without_function_entry_charges() {
    for depth in [1, 4, 10] {
        let mut functions = String::from("(func $f0 (result i64) i64.const 0)");
        for i in 1..=depth {
            functions.push_str(&format!("(func $f{i} (result i64) call $f{})", i - 1));
        }
        let m =
            wat(&format!("(module (memory (export \"memory\") 1) {functions} (export \"generate\" (func $f{depth})))"));
        let fuel = depth as u64 + 1; // one call per level and the result constant
        assert!(generate(&m, &Inputs::default(), Limits { fuel, ..Limits::default() }).is_ok(), "{depth}");
        assert_eq!(
            generate(&m, &Inputs::default(), Limits { fuel: fuel - 1, ..Limits::default() }).unwrap_err().code,
            Code::Prg12
        );
    }
}

#[test]
fn a_host_call_costs_one_instruction_and_its_body_is_free() {
    let m = wat(r#"(module
        (import "sr" "rand_u64" (func $rand (result i64)))
        (memory (export "memory") 1)
        (func (export "generate") (result i64) call $rand drop i64.const 0))"#);
    assert!(generate(&m, &Inputs::default(), Limits { fuel: 2, ..Limits::default() }).is_ok());
    assert_eq!(
        generate(&m, &Inputs::default(), Limits { fuel: 1, ..Limits::default() }).unwrap_err().code,
        Code::Prg12
    );
}

#[test]
fn fuel_counts_loop_iterations_exactly() {
    // 10 iterations of: local.get, i32.const, i32.add, local.tee, i32.const, i32.lt_u, br_if = 7 charged each;
    // then i64.const 1 = 71 in all
    let m = wat(r#"(module (memory (export "memory") 1)
        (func (export "generate") (result i64) (local i32)
          (loop (local.get 0) (i32.const 1) (i32.add) (local.tee 0) (i32.const 10) (i32.lt_u) (br_if 0))
          (i64.const 0)))"#);
    assert!(generate(&m, &Inputs::default(), Limits { fuel: 71, ..Limits::default() }).is_ok());
    assert_eq!(
        generate(&m, &Inputs::default(), Limits { fuel: 70, ..Limits::default() }).unwrap_err().code,
        Code::Prg12
    );
}

#[test]
fn traps_are_prg13_and_bad_output_ranges_prg14() {
    let trap = wat(r#"(module (memory (export "memory") 1) (func (export "generate") (result i64) unreachable))"#);
    assert_eq!(generate(&trap, &Inputs::default(), Limits::default()).unwrap_err().code, Code::Prg13);
    let outside = wat(
        r#"(module (memory (export "memory") 1) (func (export "generate") (result i64) i64.const 0x0000FFFF00000010))"#,
    );
    assert_eq!(generate(&outside, &Inputs::default(), Limits::default()).unwrap_err().code, Code::Prg14);
}

#[test]
fn missing_exports_and_oversized_memory_are_prg11() {
    let no_generate = wat(r#"(module (memory (export "memory") 1))"#);
    assert_eq!(generate(&no_generate, &Inputs::default(), Limits::default()).unwrap_err().code, Code::Prg11);
    let big = wat(r#"(module (memory (export "memory") 2000) (func (export "generate") (result i64) i64.const 0))"#);
    assert_eq!(generate(&big, &Inputs::default(), Limits::default()).unwrap_err().code, Code::Prg11);
}

#[test]
fn the_same_inputs_give_the_same_cache_key_and_any_change_another() {
    let a = cache_key("AB", &params(&[("x", "1"), ("a", "2")]), Limits::default());
    assert_eq!(a, cache_key("ab", &params(&[("a", "2"), ("x", "1")]), Limits::default()));
    assert_ne!(a, cache_key("ab", &params(&[("a", "2"), ("x", "2")]), Limits::default()));
    assert_ne!(a, cache_key("ab", &params(&[("a", "2"), ("x", "1")]), Limits { fuel: 5, ..Limits::default() }));
    assert_eq!(a.len(), 64);
}

// ------------------------------------------------------------------ step mode (SREP 69)

fn first_pixel(s: &mut Stepper) -> [u8; 4] {
    let f = s.frame().unwrap();
    assert_eq!(f.len(), 100 * 100 * 4);
    [f[0], f[1], f[2], f[3]]
}

#[test]
fn a_stepping_program_counts_its_steps() {
    let red = [255, 0, 0, 255];
    let blue = [0, 0, 255, 255];
    let mut s =
        Stepper::new(&module("srep69-counter"), &params(&[("target", "5")]), Limits::default(), 100, 100).unwrap();
    for k in 1..=5 {
        s.step().unwrap();
        assert_eq!(first_pixel(&mut s), if k >= 5 { red } else { blue }, "after {k} steps");
    }
    assert_eq!(s.steps(), 5);
}

#[test]
fn the_generator_continues_across_steps() {
    // the kit: project seed 1, seed 1 gives an odd XOR of five values (red), seed 4 an even one (blue)
    let xor = |seed| {
        let mut r = sr_wasm::Rng::new(1, seed);
        (0..5).fold(0u64, |a, _| a ^ r.next()) & 1
    };
    assert_eq!((xor(1), xor(4)), (1, 0));
    for (seed, want) in [(1u64, [255, 0, 0, 255]), (4, [0, 0, 255, 255])] {
        let mut s = Stepper::new(&module("srep69-rng"), &seeds(1, seed), Limits::default(), 100, 100).unwrap();
        (0..5).for_each(|_| s.step().unwrap());
        assert_eq!(first_pixel(&mut s), want, "seed {seed}");
    }
}

#[test]
fn a_stepping_program_without_step_is_prg11_and_a_wrong_size_prg14() {
    let e = Stepper::new(&module("srep69-no-step"), &Inputs::default(), Limits::default(), 100, 100).err().unwrap();
    assert_eq!(e.code, Code::Prg11);
    let mut s = Stepper::new(&module("srep69-counter"), &params(&[]), Limits::default(), 10, 10).unwrap();
    assert_eq!(s.frame().unwrap_err().code, Code::Prg14);
}
