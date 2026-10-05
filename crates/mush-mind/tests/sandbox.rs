//! **The sandbox, proved** (17b cut M2.1; doc 17 §7, §8, §13; D-UM-24 to
//! D-UM-31). Every test here drives the crate through its public surface —
//! [`Sandbox::new`] with a test API, [`Sandbox::compile`],
//! [`Sandbox::think`] — the way 17b cut M2.4's runner will.

use std::cell::Cell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mush_mind::memory::{self, Limits, MemError, MemFault, Memory, Value};
use mush_mind::rhai::{Dynamic, EvalAltResult, INT, Map};
use mush_mind::{
    CompileError, Compiled, Digest, EngineLimits, Fault, MAX_ARRAY_SIZE, MAX_MAP_SIZE,
    MAX_MEMORY_ANY_CLASS, MAX_STRING_BYTES, Outcome, RHAI_ABI, Sandbox, Think, budget,
    system_error, trace,
};

/// The ABI every test compiles under.
const ABI: &str = "rhai=1.26.0;api=test;stock=test";

/// A generous ceiling for the tests that are not about the budget.
const ROOMY: u64 = 1_000_000;

/// Stock class I limits (doc 17 §9): 32 keys, 24-byte keys, 32-byte strings,
/// 32-int arrays.
const CLASS_I: Limits = Limits {
    keys: 32,
    key_chars: 24,
    string_bytes: 32,
    array_len: 32,
};

thread_local! {
    /// What `ctx_probe()` answers: the think's "fact", set by the test.
    static PROBE: Cell<i64> = const { Cell::new(0) };
    /// How many times `ctx_probe()` ran.
    static PROBE_CALLS: Cell<u64> = const { Cell::new(0) };
}

/// The sandbox with the test API a game would register on it: a kernel that
/// charges a fee, a zero-argument context function, a native that panics and
/// a station function that refuses.
fn sandbox() -> Sandbox {
    Sandbox::new(
        &EngineLimits {
            max_ops_backstop: ROOMY,
        },
        |engine| {
            engine.register_fn("fee_test", |n: INT| {
                budget::charge(u64::try_from(n).unwrap_or(0));
            });
            engine.register_fn("ctx_probe", || -> INT {
                PROBE_CALLS.with(|calls| calls.set(calls.get() + 1));
                PROBE.with(Cell::get)
            });
            engine.register_fn("boom", || -> INT { panic!("a native function panicked") });
            engine.register_fn("refuse", || -> Result<(), Box<EvalAltResult>> {
                Err(system_error("no"))
            });
        },
    )
}

/// A digest of `source` (FNV-1a spread over the 32 bytes): one source, one
/// key, so every test script caches apart.
fn digest_of(source: &str) -> Digest {
    let mut out = [0u8; 32];
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (round, chunk) in out.chunks_mut(8).enumerate() {
        for byte in source.bytes().chain([round as u8]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        chunk.copy_from_slice(&hash.to_le_bytes());
    }
    Digest(out)
}

/// Compile `source` on `sandbox`.
fn compile(sandbox: &Sandbox, source: &str) -> Result<Arc<Compiled>, CompileError> {
    sandbox.compile(source, digest_of(source), ABI)
}

/// One think with explicit arguments and memory.
fn think_with(
    sandbox: &Sandbox,
    compiled: &Compiled,
    args: Vec<Dynamic>,
    memory: &[u8],
    limits: &Limits,
    ops_limit: u64,
    fee_per_think: u64,
) -> (Result<Outcome, Fault>, Vec<String>) {
    sandbox.think(Think {
        compiled,
        args,
        memory,
        limits,
        ops_limit,
        fee_per_think,
    })
}

/// One think of `source` with two unit arguments, empty memory, class I and
/// a roomy budget; the source must compile.
fn think(sandbox: &Sandbox, source: &str) -> (Result<Outcome, Fault>, Vec<String>) {
    let compiled = compile(sandbox, source).unwrap_or_else(|e| panic!("{source}: {e}"));
    think_with(
        sandbox,
        &compiled,
        vec![Dynamic::UNIT, Dynamic::UNIT],
        b"",
        &CLASS_I,
        ROOMY,
        20,
    )
}

/// The memory a clean think wrote, as text.
fn wrote(result: &Result<Outcome, Fault>) -> String {
    match result {
        Ok(outcome) => String::from_utf8(outcome.memory.clone()).unwrap(),
        Err(fault) => panic!("expected a clean think, got {fault:?}"),
    }
}

// ---------------------------------------------------------------------------
// 1–2. The budget (D-UM-26)
// ---------------------------------------------------------------------------

/// (1) Rhai's operation count is a function of the AST and the engine
/// version, and no clock is read: the same script trips the same budget at
/// the same operation every time — and at the FIRST operation that passes
/// it.
#[test]
fn the_budget_trips_at_the_same_op_on_two_runs() {
    let sandbox = sandbox();
    let source = "fn think(a, b) { let x = 0; loop { x += 1; } }";
    let compiled = compile(&sandbox, source).unwrap();
    let mut spent = Vec::new();
    for _ in 0..2 {
        let (result, _) = think_with(
            &sandbox,
            &compiled,
            vec![Dynamic::UNIT, Dynamic::UNIT],
            b"",
            &CLASS_I,
            500,
            20,
        );
        assert_eq!(result, Err(Fault::Ops));
        assert_eq!(Fault::Ops.cause(), "ops");
        spent.push(budget::spent());
    }
    assert_eq!(
        spent[0], spent[1],
        "two runs tripped at different operations"
    );
    let (ops, fee) = spent[0];
    assert_eq!(fee, 20, "the per-think fee is charged before the first op");
    assert_eq!(
        ops + fee,
        501,
        "the trip lands at the first op past the limit"
    );
}

/// (2) A kernel's fee lands the trip at the next operation; a fee charged by
/// the last call before `think` returns trips at the re-check after the
/// call.
#[test]
fn a_kernel_fee_lands_the_trip_at_the_next_op_and_a_trailing_fee_after_the_return() {
    let sandbox = sandbox();
    let run = |source: &str| {
        let compiled = compile(&sandbox, source).unwrap();
        let (result, _) = think_with(
            &sandbox,
            &compiled,
            vec![Dynamic::UNIT, Dynamic::UNIT],
            b"",
            &CLASS_I,
            500,
            0,
        );
        (result, budget::spent())
    };

    // Under the limit on its operations alone.
    let (alone, (ops_alone, fee_alone)) =
        run("fn think(a, b) { fee_test(0); let x = 0; while x < 50 { x += 1; } }");
    assert_eq!(fee_alone, 0);
    assert!(ops_alone < 500 && ops_alone > 100, "ops {ops_alone}");
    // The outcome carries what the think spent, each in its own field.
    let outcome = alone.unwrap();
    assert_eq!((outcome.ops, outcome.fee), (ops_alone, 0));

    // The per-think fee and a kernel's fee add up in `Outcome::fee`.
    let compiled = compile(&sandbox, "fn think(a, b) { fee_test(5); }").unwrap();
    let (paid, _) = think_with(
        &sandbox,
        &compiled,
        vec![Dynamic::UNIT, Dynamic::UNIT],
        b"",
        &CLASS_I,
        500,
        20,
    );
    let paid = paid.unwrap();
    assert_eq!(paid.fee, 25);
    assert_eq!((paid.ops, paid.fee), budget::spent());
    assert!(paid.ops > 0 && paid.ops < 20, "ops {}", paid.ops);

    // The same script with a 400 fee charged at its head trips, at the first
    // operation where ops + fee pass 500.
    let (charged, (ops, fee)) =
        run("fn think(a, b) { fee_test(400); let x = 0; while x < 50 { x += 1; } }");
    assert_eq!(charged, Err(Fault::Ops));
    assert_eq!(fee, 400);
    assert_eq!(ops + fee, 501, "the trip is the next op after the fee");

    // A fee charged by the last statement has no next op: the re-check
    // after the return is that op.
    for source in [
        "fn think(a, b) { fee_test(10_000); }",
        "fn think(a, b) { this.k = 1; fee_test(10_000) }",
    ] {
        let (trailing, (ops, fee)) = run(source);
        assert_eq!(trailing, Err(Fault::Ops), "{source}");
        assert_eq!(fee, 10_000);
        assert!(
            ops < 20,
            "the call returned after {ops} ops, under the limit alone"
        );
    }
}

// ---------------------------------------------------------------------------
// 3–4b. What resolves (D-UM-24, D-UM-25, D-UM-28)
// ---------------------------------------------------------------------------

/// How a script that must never answer a value ended.
#[derive(Debug, PartialEq, Eq)]
enum Refused {
    Parse,
    Type,
}

/// Compile and run `source`; it must fail at compile (`Parse`) or at the
/// call (`Type`), never answer a value.
fn refused(sandbox: &Sandbox, source: &str) -> Refused {
    match compile(sandbox, source) {
        Err(CompileError::Parse(_)) => Refused::Parse,
        Err(CompileError::NoThink) => panic!("{source}: no think"),
        Ok(compiled) => {
            let (result, _) = think_with(
                sandbox,
                &compiled,
                vec![Dynamic::UNIT, Dynamic::UNIT],
                b"",
                &CLASS_I,
                ROOMY,
                20,
            );
            match result {
                Err(Fault::Type(_)) => Refused::Type,
                other => panic!("{source} answered {other:?}"),
            }
        }
    }
}

/// (3) No clock, no `eval`, no keyword-function, no excluded package's
/// function resolves; floats evaluate in expressions and a FLOAT reaching
/// memory is a `type` fault.
#[test]
fn no_time_eval_or_keyword_function_resolves() {
    let sandbox = sandbox();

    // The keyword-functions are disabled symbols: a compile error.
    for source in [
        "fn think(a, b) { eval(\"1\") }",
        "import \"x\" as x; fn think(a, b) { }",
        "fn think(a, b) { Fn(\"f\") }",
        "fn think(a, b) { let f = 1; f.call() }",
        "fn think(a, b) { (|| 1).call() }",
        "fn think(a, b) { let f = 1; f.curry(1) }",
    ] {
        assert_eq!(refused(&sandbox, source), Refused::Parse, "{source}");
    }

    // The excluded packages' functions are not there: a `type` fault.
    for source in [
        "fn think(a, b) { timestamp() }",
        "fn think(a, b) { to_int(1.5) }",
        "fn think(a, b) { sin(1.0) }",
        "fn think(a, b) { parse_int(\"1\") }",
        // `MoreStringPackage`'s string-plus-anything and string `len`.
        "fn think(a, b) { let s = \"k\" + 1; }",
        "fn think(a, b) { let n = \"abc\".len(); }",
    ] {
        assert_eq!(refused(&sandbox, source), Refused::Type, "{source}");
    }

    // `sleep` is shadowed by a refusal, not slept: under 100 ms of wall
    // time for a one-second sleep, INT or FLOAT, and the refusal's words.
    for source in [
        "fn think(a, b) { sleep(1) }",
        "fn think(a, b) { sleep(1.0) }",
    ] {
        let compiled = compile(&sandbox, source).unwrap();
        let started = Instant::now();
        let (result, _) = think_with(
            &sandbox,
            &compiled,
            vec![Dynamic::UNIT, Dynamic::UNIT],
            b"",
            &CLASS_I,
            ROOMY,
            20,
        );
        let took = started.elapsed();
        assert!(took < Duration::from_millis(100), "{source} took {took:?}");
        match result {
            Err(Fault::Type(text)) => {
                assert!(text.contains("sleep is not a thing a mind does"), "{text}");
            }
            other => panic!("{source} answered {other:?}"),
        }
    }

    // Floats evaluate in expressions (D-UM-28)…
    let (result, _) = think(
        &sandbox,
        "fn think(a, b) { let f = 1.5 * 2.0; this.ok = f > 2.9 && f < 3.1; }",
    );
    assert_eq!(wrote(&result), r#"{"ok":true}"#);
    // …and a FLOAT reaching memory is a `type` fault holding the key,
    // anywhere in a value (the brief's `Fault::Type(key)`).
    for source in [
        "fn think(a, b) { this.x = 1.5 }",
        "fn think(a, b) { this.x = [1, 2.5] }",
    ] {
        assert_eq!(
            think(&sandbox, source).0,
            Err(Fault::Type("x".to_string())),
            "{source}"
        );
    }
}

/// (4) Strict variables make a typo a compile error (D-UM-25: a seal-time
/// error); a missing map property and an out-of-bounds index are `type`
/// faults.
#[test]
fn strict_variables_and_invalid_map_properties() {
    let sandbox = sandbox();
    let typo = "fn think(a, b) { let speed = 1; this.s = sped; }";
    assert!(matches!(
        compile(&sandbox, typo),
        Err(CompileError::Parse(_))
    ));
    for source in [
        "fn think(a, b) { let v = this.nosuch; }",
        "fn think(a, b) { let v = []; v[0] }",
    ] {
        assert!(
            matches!(think(&sandbox, source).0, Err(Fault::Type(_))),
            "{source}"
        );
    }
}

/// (4b) The scope contract the game's per-think facts rest on (17b's settled
/// question (ii)): (i) a name no scope the parser sees declares is a compile
/// error, whatever a caller might push later; (ii) a registered zero-argument
/// function resolves inside a helper `fn` called from `think`, on the CACHED
/// AST, answering each think's own value — never folded at compile time; (iii)
/// it runs exactly once per think.
#[test]
fn the_scope_contract_a_function_body_sees_no_call_scope_and_a_native_function_resolves_at_depth() {
    let sandbox = sandbox();

    // (i)
    assert!(matches!(
        compile(&sandbox, "fn think(a, b) { x }"),
        Err(CompileError::Parse(_))
    ));
    // A top-level `const` is invisible in a function body too.
    assert!(matches!(
        compile(&sandbox, "const K = 1; fn think(a, b) { this.k = K; }"),
        Err(CompileError::Parse(_))
    ));

    // (ii), (iii)
    let source = "fn helper() { ctx_probe() } fn think(a, b) { this.v = helper(); }";
    PROBE_CALLS.with(|calls| calls.set(0));
    let compiled = compile(&sandbox, source).unwrap();
    assert_eq!(
        PROBE_CALLS.with(Cell::get),
        0,
        "compiling never calls a registered function"
    );
    for (round, fact) in [7_i64, -3, 42].into_iter().enumerate() {
        PROBE.with(|probe| probe.set(fact));
        let cached = compile(&sandbox, source).unwrap();
        assert!(Arc::ptr_eq(&compiled, &cached), "the cached AST");
        let (result, _) = think_with(
            &sandbox,
            &cached,
            vec![Dynamic::UNIT, Dynamic::UNIT],
            b"",
            &CLASS_I,
            ROOMY,
            20,
        );
        assert_eq!(wrote(&result), format!("{{\"v\":{fact}}}"));
        assert_eq!(
            PROBE_CALLS.with(Cell::get),
            round as u64 + 1,
            "one call per think"
        );
    }
}

// ---------------------------------------------------------------------------
// 5. The trace (doc 17 §14)
// ---------------------------------------------------------------------------

/// (5) `print` and `debug` land in the trace, eight lines of 120 characters
/// at most, answered beside a fault as beside a clean think.
#[test]
fn print_and_debug_go_to_the_trace_and_the_buffer_is_bounded() {
    let sandbox = sandbox();

    let (result, lines) = think(
        &sandbox,
        "fn think(a, b) { for i in 0..9 { print(\"line \" + i.to_string()); } }",
    );
    assert!(result.is_ok());
    assert_eq!(lines.len(), trace::TRACE_LINES);
    assert_eq!(lines.first().map(String::as_str), Some("line 0"));
    assert_eq!(lines.last().map(String::as_str), Some("line 7"));

    let (_, lines) = think(
        &sandbox,
        "fn think(a, b) { let s = \"\"; for i in 0..300 { s += \"x\"; } print(s); }",
    );
    assert_eq!(lines, vec!["x".repeat(trace::TRACE_WIDTH)]);

    let (_, lines) = think(&sandbox, "fn think(a, b) { debug(\"hello\"); }");
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("hello"), "{lines:?}");

    let (result, lines) = think(
        &sandbox,
        "fn think(a, b) { print(\"before\"); let v = []; v[0]; }",
    );
    assert!(matches!(result, Err(Fault::Type(_))));
    assert_eq!(lines, vec!["before".to_string()]);

    // A think's trace is its own: the next think starts empty.
    let (_, lines) = think(&sandbox, "fn think(a, b) { }");
    assert!(lines.is_empty());
}

// ---------------------------------------------------------------------------
// 6–8. The memory (D-UM-30)
// ---------------------------------------------------------------------------

/// (6) The codec's bytes, golden, and its refusals.
#[test]
fn the_codec_writes_sorted_compact_json() {
    let memory = Memory::from([
        ("e".to_string(), Value::Ints(vec![1, -2, 3])),
        ("c".to_string(), Value::Unit),
        ("a".to_string(), Value::Int(1)),
        ("d".to_string(), Value::Str("x\"y\\z".to_string())),
        ("b".to_string(), Value::Bool(true)),
    ]);
    let bytes = memory::encode(&memory);
    assert_eq!(
        bytes,
        br#"{"a":1,"b":true,"c":null,"d":"x\"y\\z","e":[1,-2,3]}"#
    );
    assert_eq!(memory::decode(&bytes), Some(memory));

    let empty = Memory::new();
    assert_eq!(memory::encode(&empty), b"{}");
    assert_eq!(memory::decode(b"{}"), Some(empty));

    let extremes = Memory::from([
        ("lo".to_string(), Value::Int(i64::MIN)),
        ("hi".to_string(), Value::Int(i64::MAX)),
        ("row".to_string(), Value::Ints(vec![i64::MIN, 0, i64::MAX])),
        ("none".to_string(), Value::Ints(vec![])),
        ("off".to_string(), Value::Bool(false)),
        ("text".to_string(), Value::Str(String::new())),
    ]);
    let bytes = memory::encode(&extremes);
    assert_eq!(
        bytes,
        br#"{"hi":9223372036854775807,"lo":-9223372036854775808,"none":[],"off":false,"row":[-9223372036854775808,0,9223372036854775807],"text":""}"#
    );
    assert_eq!(memory::decode(&bytes), Some(extremes));

    for refused in [
        &br#"{ "a":1}"#[..],
        br#"{"a": 1}"#,
        br#"{"a":1.5}"#,
        br#"{"a":1e3}"#,
        br#"{"a":[[1]]}"#,
        br#"{"a":{"b":1}}"#,
        br#"{"a":1,}"#,
        br#"{"a":[1,]}"#,
        br#"{"a":"\n"}"#,
        br#"{"a":-0}"#,
        br#"{"a":01}"#,
        br#"{"a":9223372036854775808}"#,
        br#"{"b":1,"a":2}"#,
        br#"{"a":1,"a":2}"#,
        br#"{"a":1} "#,
        br#"{"a":1}{}"#,
        b"",
        b"{\"a\":\"\xc3\xa9\"}",
    ] {
        assert_eq!(
            memory::decode(refused),
            None,
            "{}",
            String::from_utf8_lossy(refused)
        );
    }
    // A unicode escape, spelled with the backslash built from its byte so no
    // editor or tool can read the escape before the codec does.
    let backslash = char::from(0x5c);
    let unicode = format!("{{\"a\":\"{backslash}u0041\"}}");
    assert_eq!(unicode.len(), 14);
    assert_eq!(memory::decode(unicode.as_bytes()), None, "{unicode}");
}

/// (7) `check` refuses every shape by name, the count before any key; and
/// `from_map` tells a FLOAT from a wrong shape.
#[test]
fn check_refuses_every_shape_by_name() {
    let one = |key: &str, value: Value| Memory::from([(key.to_string(), value)]);
    assert_eq!(
        memory::check(&one(&"k".repeat(25), Value::Int(1)), &CLASS_I),
        Err(MemError::Key("k".repeat(25)))
    );
    assert_eq!(
        memory::check(&one(&"k".repeat(24), Value::Int(1)), &CLASS_I),
        Ok(())
    );
    assert_eq!(
        memory::check(&one("", Value::Int(1)), &CLASS_I),
        Err(MemError::Key(String::new()))
    );
    assert_eq!(
        memory::check(&one("a\tb", Value::Int(1)), &CLASS_I),
        Err(MemError::Key("a\tb".to_string()))
    );
    let mut many: Memory = (0..33).map(|i| (format!("k{i:02}"), Value::Unit)).collect();
    many.insert(String::new(), Value::Unit);
    assert_eq!(memory::check(&many, &CLASS_I), Err(MemError::Count(34)));
    many.remove("");
    assert_eq!(memory::check(&many, &CLASS_I), Err(MemError::Count(33)));
    assert_eq!(
        memory::check(&one("s", Value::Str("x".repeat(33))), &CLASS_I),
        Err(MemError::Value("s".to_string()))
    );
    assert_eq!(
        memory::check(&one("s", Value::Str("x".repeat(32))), &CLASS_I),
        Ok(())
    );
    assert_eq!(
        memory::check(&one("s", Value::Str("é".to_string())), &CLASS_I),
        Err(MemError::Value("s".to_string()))
    );
    assert_eq!(
        memory::check(&one("r", Value::Ints(vec![0; 33])), &CLASS_I),
        Err(MemError::Value("r".to_string()))
    );
    assert_eq!(
        memory::check(&one("r", Value::Ints(vec![0; 32])), &CLASS_I),
        Ok(())
    );

    let mut map = Map::new();
    map.insert("f".into(), Dynamic::from_float(1.5));
    assert_eq!(
        memory::from_map(&map, &CLASS_I),
        Err(MemFault::Float("f".to_string()))
    );
    let mut nested = Map::new();
    nested.insert("n".into(), Dynamic::from_map(Map::new()));
    assert_eq!(
        memory::from_map(&nested, &CLASS_I),
        Err(MemFault::Shape(MemError::Value("n".to_string())))
    );
    let mut mixed = Map::new();
    mixed.insert(
        "m".into(),
        Dynamic::from_array(vec![Dynamic::from_int(1), "x".into()]),
    );
    assert_eq!(
        memory::from_map(&mixed, &CLASS_I),
        Err(MemFault::Shape(MemError::Value("m".to_string())))
    );

    // to_map and from_map are inverses over every shape.
    let shapes = Memory::from([
        ("a".to_string(), Value::Int(-4)),
        ("b".to_string(), Value::Bool(false)),
        ("c".to_string(), Value::Unit),
        ("d".to_string(), Value::Str("hi".to_string())),
        ("e".to_string(), Value::Ints(vec![3, 2, 1])),
    ]);
    assert_eq!(
        memory::from_map(&memory::to_map(&shapes), &CLASS_I),
        Ok(shapes)
    );
}

/// (8) The largest class's memory round-trips under the engine's data-size
/// backstop; an over-count is a `memory` fault, never a `limit`; a runaway
/// string trips the engine's limit; the backstop is twice the class.
#[test]
fn the_largest_class_memory_round_trips_and_an_over_count_is_a_memory_fault() {
    let sandbox = sandbox();
    let class = MAX_MEMORY_ANY_CLASS;
    let run = |source: &str| {
        let compiled = compile(&sandbox, source).unwrap();
        think_with(
            &sandbox,
            &compiled,
            vec![Dynamic::UNIT, Dynamic::UNIT],
            b"",
            &class,
            ROOMY,
            20,
        )
        .0
    };

    let arrays = run("fn think(a, b) { \
                        for i in 0..128 { \
                          let row = []; \
                          for j in 0..32 { row.push(j); } \
                          this[\"k\" + i.to_string()] = row; \
                        } \
                      }");
    let decoded = memory::decode(&arrays.unwrap().memory).unwrap();
    assert_eq!(decoded.len(), 128);
    assert!(
        decoded
            .values()
            .all(|value| matches!(value, Value::Ints(row) if row.len() == 32))
    );

    let strings = run("fn think(a, b) { \
                         for i in 0..128 { \
                           this[\"k\" + i.to_string()] = \"abcdefghijklmnopqrstuvwxyz012345\"; \
                         } \
                       }");
    let decoded = memory::decode(&strings.unwrap().memory).unwrap();
    assert_eq!(decoded.len(), 128);
    assert!(
        decoded
            .values()
            .all(|value| matches!(value, Value::Str(text) if text.len() == 32))
    );

    let over = run("fn think(a, b) { for i in 0..129 { this[\"k\" + i.to_string()] = i; } }");
    assert_eq!(over, Err(Fault::Memory(MemError::Count(129))));
    assert_eq!(Fault::Memory(MemError::Count(129)).cause(), "memory");

    let runaway = run("fn think(a, b) { let s = \"x\"; loop { s += s; } }");
    assert_eq!(runaway, Err(Fault::Limit));
    assert_eq!(Fault::Limit.cause(), "limit");
    // The other engine limits end a think `limit` too: a runaway array, the
    // call levels (12; a recursion thirteen deep), and the variables (64).
    let wide = run("fn think(a, b) { let v = [1]; loop { v += v; } }");
    assert_eq!(wide, Err(Fault::Limit));
    let deep = run("fn r(n) { if n > 0 { r(n - 1) } } fn think(a, b) { r(13); }");
    assert_eq!(deep, Err(Fault::Limit));
    assert!(run("fn r(n) { if n > 0 { r(n - 1) } } fn think(a, b) { r(10); }").is_ok());
    let lets: String = (0..65).map(|i| format!("let v{i} = {i}; ")).collect();
    let crowded = run(&format!("fn think(a, b) {{ {lets} }}"));
    assert_eq!(crowded, Err(Fault::Limit));

    assert!(MAX_MAP_SIZE >= 2 * class.keys);
    assert!(MAX_ARRAY_SIZE >= 2 * class.keys * class.array_len);
    assert!(MAX_STRING_BYTES >= 2 * class.keys * class.string_bytes);
}

// ---------------------------------------------------------------------------
// 9. The cache (D-UM-27)
// ---------------------------------------------------------------------------

/// (9) One AST per `(digest, abi)`; a mind without `fn think(a, b)` does not
/// compile; the top-level statements are cleared.
#[test]
fn the_cache_keys_on_digest_and_abi_and_strips_statements() {
    let sandbox = sandbox();
    let source = "print(\"top\"); fn think(a, b) { }";
    let digest = digest_of(source);
    let first = sandbox.compile(source, digest, ABI).unwrap();
    let again = sandbox.compile(source, digest, ABI).unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    assert_eq!(sandbox.cached(), 1);
    assert_eq!((first.digest, first.abi), (digest, ABI));

    let other_abi = sandbox
        .compile(source, digest, "rhai=1.26.0;api=2;stock=1")
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &other_abi));
    let other_digest = sandbox.compile(source, Digest([9; 32]), ABI).unwrap();
    assert!(!Arc::ptr_eq(&first, &other_digest));
    assert_eq!(sandbox.cached(), 3);

    for source in [
        "fn helper(a, b) { }",
        "fn think(a) { }",
        "fn think(a, b, c) { }",
        "1 + 1",
    ] {
        assert_eq!(
            compile(&sandbox, source).unwrap_err(),
            CompileError::NoThink,
            "{source}"
        );
    }
    assert_eq!(sandbox.cached(), 3, "a refused source is not cached");

    let (result, lines) = think_with(
        &sandbox,
        &first,
        vec![Dynamic::UNIT, Dynamic::UNIT],
        b"",
        &CLASS_I,
        ROOMY,
        20,
    );
    assert!(result.is_ok());
    assert!(lines.is_empty(), "a top-level print never runs: {lines:?}");
    // And nothing but the functions is left to run: evaluating the cached
    // AST whole — on an engine where `print` resolves and lands in the
    // trace — runs clean and prints nothing. (An uncleared AST prints "top"
    // here: the control below proves the probe can see it.)
    trace::begin();
    let whole = sandbox_eval_is_silent(&first.ast);
    assert!(whole.is_ok(), "{whole:?}");
    assert!(trace::take().is_empty(), "the statements were cleared");
    let uncleared = mush_mind::rhai::Engine::new_raw().compile(source).unwrap();
    trace::begin();
    sandbox_eval_is_silent(&uncleared).unwrap();
    assert_eq!(
        trace::take(),
        vec!["top".to_string()],
        "the probe's control"
    );
}

/// Run an AST whole — its top-level statements — on a bare engine carrying
/// the string package (where `print` lives) whose `print` goes to this
/// thread's trace.
fn sandbox_eval_is_silent(ast: &mush_mind::rhai::AST) -> Result<(), Box<EvalAltResult>> {
    use mush_mind::rhai::packages::{BasicStringPackage, Package};
    let mut engine = mush_mind::rhai::Engine::new_raw();
    engine.register_global_module(BasicStringPackage::new().as_shared_module());
    engine.on_print(trace::push);
    engine.run_ast(ast)
}

// ---------------------------------------------------------------------------
// 10–15. The run
// ---------------------------------------------------------------------------

/// (10) A panic in a native function is a `panic` fault, and the sandbox
/// thinks on.
#[test]
fn a_panic_in_a_callback_is_a_fault_and_the_sandbox_survives() {
    let sandbox = sandbox();
    let (result, lines) = think(&sandbox, "fn think(a, b) { print(\"x\"); boom(); }");
    assert_eq!(result, Err(Fault::Panic));
    assert_eq!(Fault::Panic.cause(), "panic");
    assert_eq!(lines, vec!["x".to_string()]);
    let (result, _) = think(&sandbox, "fn think(a, b) { this.k = 1; }");
    assert_eq!(wrote(&result), r#"{"k":1}"#);
}

/// (11) `this` is the memory, read back after the call; an argument is a
/// copy, and writing into it writes nothing anyone keeps.
#[test]
fn this_is_the_memory_and_an_argument_is_a_copy() {
    let sandbox = sandbox();
    let compiled = compile(&sandbox, "fn think(a, b) { this.k = a + b; }").unwrap();
    let (result, _) = think_with(
        &sandbox,
        &compiled,
        vec![Dynamic::from_int(1), Dynamic::from_int(2)],
        b"",
        &CLASS_I,
        ROOMY,
        20,
    );
    assert_eq!(wrote(&result), r#"{"k":3}"#);

    let compiled = compile(&sandbox, "fn think(m, b) { m.k = 1; }").unwrap();
    let (result, _) = think_with(
        &sandbox,
        &compiled,
        vec![Dynamic::from_map(Map::new()), Dynamic::UNIT],
        b"",
        &CLASS_I,
        ROOMY,
        20,
    );
    assert_eq!(wrote(&result), "{}");

    // The memory handed in is the memory `this` starts as; undecodable
    // bytes start over from the empty map.
    let compiled = compile(
        &sandbox,
        "fn think(a, b) { this.n = this.len(); this.k += 1; }",
    )
    .unwrap();
    let (result, _) = think_with(
        &sandbox,
        &compiled,
        vec![Dynamic::UNIT, Dynamic::UNIT],
        br#"{"k":41,"z":"q"}"#,
        &CLASS_I,
        ROOMY,
        20,
    );
    assert_eq!(wrote(&result), r#"{"k":42,"n":2,"z":"q"}"#);
    let compiled = compile(&sandbox, "fn think(a, b) { this.n = this.len(); }").unwrap();
    let (result, _) = think_with(
        &sandbox,
        &compiled,
        vec![Dynamic::UNIT, Dynamic::UNIT],
        b"not memory",
        &CLASS_I,
        ROOMY,
        20,
    );
    assert_eq!(wrote(&result), r#"{"n":0}"#);

    // A `this` replaced by something not a map is a `type` fault.
    assert!(matches!(
        think(&sandbox, "fn think(a, b) { this = 5; }").0,
        Err(Fault::Type(_))
    ));
}

/// (12) The integer helpers, one implementation per name (the packages'
/// `abs`, `min` and `max`; the sandbox's `clamp` and `isqrt` —
/// `engine::tests::the_packages_carry_abs_min_max_and_not_clamp_or_isqrt`).
#[test]
fn the_integer_helpers() {
    let sandbox = sandbox();
    let (result, _) = think(
        &sandbox,
        "fn think(a, b) { \
           this.abs = abs(-3); this.min = min(2, 5); this.max = max(2, 5); \
           this.hi = clamp(15, 0, 10); this.lo = clamp(-5, 0, 10); this.mid = clamp(4, 0, 10); \
           this.r17 = isqrt(17); this.r0 = isqrt(0); this.r16 = isqrt(16); \
           this.rmax = isqrt(9223372036854775807); \
         }",
    );
    assert_eq!(
        wrote(&result),
        r#"{"abs":3,"hi":10,"lo":0,"max":5,"mid":4,"min":2,"r0":0,"r16":4,"r17":4,"rmax":3037000499}"#
    );
    for source in [
        "fn think(a, b) { isqrt(-1) }",
        "fn think(a, b) { clamp(1, 10, 0) }",
    ] {
        assert!(
            matches!(think(&sandbox, source).0, Err(Fault::Type(_))),
            "{source}"
        );
    }
}

/// (13) `RHAI_ABI` names the rhai `Cargo.lock` resolved and the workspace
/// manifest pins exactly.
#[test]
fn the_abi_pin_matches_the_lockfile() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .to_path_buf();

    let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
    let mut versions = Vec::new();
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line == "name = \"rhai\"" {
            let version = lines.next().unwrap();
            versions.push(
                version
                    .strip_prefix("version = \"")
                    .and_then(|rest| rest.strip_suffix('"'))
                    .unwrap()
                    .to_string(),
            );
        }
    }
    assert_eq!(
        versions,
        vec!["1.26.0".to_string()],
        "one rhai in the lockfile"
    );
    assert_eq!(RHAI_ABI, format!("rhai={}", versions[0]));

    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let pins: Vec<&str> = manifest
        .lines()
        .filter(|line| line.starts_with("rhai = "))
        .collect();
    assert_eq!(pins.len(), 1, "{pins:?}");
    assert!(pins[0].contains("version = \"=1.26.0\""), "{}", pins[0]);
}

/// (14) `exit()` is a return: the think ends early and clean with the
/// memory it wrote before the call.
#[test]
fn exit_is_a_return() {
    let sandbox = sandbox();
    for source in [
        "fn think(a, b) { this.k = 1; exit(); this.k = 2; }",
        "fn think(a, b) { this.k = 1; exit(7); this.k = 2; }",
        "fn out() { exit(); } fn think(a, b) { this.k = 1; out(); this.k = 2; }",
        // Through a package method's callback, which hands the exit back
        // wrapped: still a return.
        "fn out(x) { exit(); } fn think(a, b) { this.k = 1; [1].map(out); this.k = 2; }",
    ] {
        assert_eq!(wrote(&think(&sandbox, source).0), r#"{"k":1}"#, "{source}");
    }
}

/// (15) A refusal raised through `system_error` is not caught by `try` —
/// not directly, not through a script function — and ends the think `type`
/// with the memory untouched; a plain `throw` inside the same `try` IS
/// caught. The contrast the game's refusals rely on (D-UM-31).
#[test]
fn a_system_error_is_not_caught_by_try() {
    let sandbox = sandbox();
    for source in [
        "fn think(a, b) { try { refuse(); } catch { this.caught = true; } }",
        "fn inner() { refuse(); } \
         fn think(a, b) { try { inner(); } catch { this.caught = true; } }",
        // Through a package method's callback, which wraps every error in a
        // catchable `ErrorInFunctionCall`: the latch still ends the think
        // with the refusal's own text, caught or not.
        "fn r(x) { refuse(); x } \
         fn think(a, b) { try { [1].map(r); } catch { this.caught = true; } }",
        "fn r(x) { refuse(); x } fn think(a, b) { [1].map(r); }",
    ] {
        let compiled = compile(&sandbox, source).unwrap();
        let (result, _) = think_with(
            &sandbox,
            &compiled,
            vec![Dynamic::UNIT, Dynamic::UNIT],
            br#"{"k":1}"#,
            &CLASS_I,
            ROOMY,
            20,
        );
        // Passed through the helper unwrapped: the text is the refusal's own.
        assert_eq!(
            result,
            Err(Fault::Type("no: refused".to_string())),
            "{source}"
        );
    }
    for source in [
        "fn think(a, b) { try { throw \"x\"; } catch { this.caught = true; } }",
        "fn inner() { throw \"x\"; } \
         fn think(a, b) { try { inner(); } catch { this.caught = true; } }",
    ] {
        let (result, _) = think(&sandbox, source);
        assert_eq!(wrote(&result), r#"{"caught":true}"#, "{source}");
    }
    assert_eq!(Fault::Type(String::new()).cause(), "type");

    // The sandbox's own refusals ride the same latch: a `sleep` caught
    // through a callback still ends the think, in its own words.
    let (result, _) = think(
        &sandbox,
        "fn r(x) { sleep(1); x } \
         fn think(a, b) { try { [1].map(r); } catch { } this.k = 2; }",
    );
    assert_eq!(
        result,
        Err(Fault::Type(
            "sleep is not a thing a mind does: refused".to_string()
        ))
    );
    // And the latch is the think's own: the next think is clean.
    let (result, _) = think(&sandbox, "fn think(a, b) { this.k = 3; }");
    assert_eq!(wrote(&result), r#"{"k":3}"#);
}

// ---------------------------------------------------------------------------
// 17–19. Review findings: callbacks, the backstop, the function cap, the
// fault's text
// ---------------------------------------------------------------------------

/// (17) A package method's callback runs on a clone of Rhai's runtime state
/// whose operation count restarts from its caller's and is thrown away on
/// return; the budget counts every operation itself, so callbacks cannot
/// spend past it — sibling callbacks trip it at the same operation on every
/// run, a trip caught through a callback's wrapping ends the think at its
/// next operation (or at the re-check after the return), and a wrapped
/// termination or engine limit keeps its cause word (review findings F1,
/// F2).
#[test]
fn callbacks_spend_the_same_budget_and_keep_their_cause_words() {
    let sandbox = sandbox();
    let run = |source: &str, ops_limit: u64| {
        let compiled = compile(&sandbox, source).unwrap();
        let (result, _) = think_with(
            &sandbox,
            &compiled,
            vec![Dynamic::UNIT, Dynamic::UNIT],
            b"",
            &CLASS_I,
            ops_limit,
            20,
        );
        (result, budget::spent())
    };

    // Eight thousand sibling callbacks of a few hundred operations each:
    // without the budget's own count each would restart from the caller's
    // and none would ever reach the limit.
    let fan = "fn burn(x) { let i = 0; while i < 50 { i += 1; } x } \
               fn think(a, b) { let v = []; v.pad(8000, 0); v.map(burn); this.k = 1; }";
    let started = Instant::now();
    let (first, first_spent) = run(fan, 1_500);
    let (second, second_spent) = run(fan, 1_500);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(first, Err(Fault::Ops));
    assert_eq!(second, Err(Fault::Ops));
    assert_eq!(first_spent, second_spent, "the same op on two runs");
    assert_eq!(
        first_spent.0 + first_spent.1,
        1_501,
        "the first op past the limit"
    );

    // A trip caught through a callback's wrapping: the next operation trips
    // again, and so does the re-check when no operation follows.
    for source in [
        "fn spin(x) { let n = 0; loop { n += 1; } } \
         fn think(a, b) { try { [1].map(spin); } catch { } this.k = 1; }",
        "fn spin(x) { let n = 0; loop { n += 1; } } \
         fn think(a, b) { this.k = 1; try { [1].map(spin); } catch { } }",
    ] {
        assert_eq!(run(source, 1_500).0, Err(Fault::Ops), "{source}");
    }

    // Uncaught, a wrapped termination is `ops` and a wrapped stack overflow
    // (a recursion through `map`, past the twelve call levels; a function
    // body cannot name itself as a callback, so `s` turns the corner) is
    // `limit`.
    let (spun, _) = run(
        "fn spin(x) { let n = 0; loop { n += 1; } } fn think(a, b) { [1].map(spin); }",
        1_500,
    );
    assert_eq!(spun, Err(Fault::Ops));
    let (deep, _) = run(
        "fn s(x) { r(x) } fn r(x) { if x > 0 { [x - 1].map(s); } x } \
         fn think(a, b) { r(20); }",
        ROOMY,
    );
    assert_eq!(deep, Err(Fault::Limit));
    // A shallow recursion through `map` is fine.
    let (shallow, _) = run(
        "fn s(x) { r(x) } fn r(x) { if x > 0 { [x - 1].map(s); } x } \
         fn think(a, b) { r(2); this.k = 1; }",
        ROOMY,
    );
    assert_eq!(wrote(&shallow), r#"{"k":1}"#);
}

/// (18) Rhai's own operation cap is checked before `on_progress`, so a
/// think whose limit sits above the engine's backstop ends `limit`, not
/// `ops` (`EngineLimits`' doc); and the function cap (32) is a parse-time
/// limit: a 33rd function never compiles (review finding T4).
#[test]
fn the_backstop_ends_a_think_limit_and_the_function_cap_is_a_compile_error() {
    let low = Sandbox::new(
        &EngineLimits {
            max_ops_backstop: 100,
        },
        |_| {},
    );
    let compiled = compile(&low, "fn think(a, b) { let x = 0; loop { x += 1; } }").unwrap();
    let (result, _) = think_with(
        &low,
        &compiled,
        vec![Dynamic::UNIT, Dynamic::UNIT],
        b"",
        &CLASS_I,
        1_000,
        0,
    );
    assert_eq!(result, Err(Fault::Limit));
    let (ops, _) = budget::spent();
    assert_eq!(ops, 100, "the budget saw every op up to the backstop");

    let sandbox = sandbox();
    let fns = |n: usize| -> String {
        let mut source: String = (0..n).map(|i| format!("fn f{i}() {{ {i} }} ")).collect();
        source.push_str("fn think(a, b) { this.k = f0(); }");
        source
    };
    // Thirty-one helpers and `think`: thirty-two functions, at the cap.
    let at_cap = compile(&sandbox, &fns(31)).unwrap();
    let (result, _) = think_with(
        &sandbox,
        &at_cap,
        vec![Dynamic::UNIT, Dynamic::UNIT],
        b"",
        &CLASS_I,
        ROOMY,
        20,
    );
    assert_eq!(wrote(&result), r#"{"k":0}"#);
    // Thirty-two helpers and `think`: one past it.
    assert!(matches!(
        compile(&sandbox, &fns(32)),
        Err(CompileError::Parse(_))
    ));
}

/// (19) A `type` fault's text and a trace line are script-authored: each is
/// one line of at most `TRACE_WIDTH` characters with every control character
/// a space, and a thrown value thousands of elements long is never
/// formatted whole (review finding F3).
#[test]
fn a_fault_text_and_a_trace_line_are_bounded_and_have_no_control_characters() {
    let sandbox = sandbox();
    let started = Instant::now();
    let (result, _) = think(
        &sandbox,
        "fn think(a, b) { let v = [9223372036854775807]; for i in 0..12 { v += v; } throw v; }",
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    match result {
        Err(Fault::Type(text)) => {
            assert_eq!(text.chars().count(), trace::TRACE_WIDTH, "{text}");
            assert!(
                text.starts_with("Runtime error: [9223372036854775807, "),
                "{text}"
            );
        }
        other => panic!("answered {other:?}"),
    }

    // A thrown string with a newline and an escape, through a helper (whose
    // wrapping adds its own newline): one line, no control character.
    let (result, lines) = think(
        &sandbox,
        "fn inner() { print(\"a\\nb\\x1bc\"); throw \"x\\ny\"; } fn think(a, b) { inner(); }",
    );
    assert_eq!(lines, vec!["a b c".to_string()]);
    match result {
        Err(Fault::Type(text)) => {
            assert!(!text.chars().any(char::is_control), "{text:?}");
            assert!(text.starts_with("Runtime error: x y"), "{text:?}");
        }
        other => panic!("answered {other:?}"),
    }
}
