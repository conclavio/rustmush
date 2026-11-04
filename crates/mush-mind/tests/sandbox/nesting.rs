//! Doc 43 cut TS.4: nested source against the mind sandbox's depth guard.
//!
//! A player's mind source (capped at `mush_roll::MAX_SOURCE_BYTES`, 32 KiB)
//! is compiled on the minds pool. A stack overflow there is not a panic the
//! pool can catch: it aborts the process. `set_max_expr_depths(48, 24)` is
//! the guard; these tests feed every nesting shape the grammar has, at the
//! guard's edge and far past it, at the source cap, on a thread with the
//! pool's default 2 MiB stack, and require a compile error or a clean think,
//! never an abort.

use mush_mind::rhai::Dynamic;
use mush_mind::{CompileError, Sandbox};

use super::{CLASS_I, ROOMY, compile, sandbox, think_with};

/// The mind source cap (`mush_roll::MAX_SOURCE_BYTES`).
const SOURCE_CAP: usize = 32 * 1024;

/// The guard's limits as `Sandbox::new` sets them (D-UM-25).
const MAX_EXPR_DEPTH: usize = 48;
const MAX_FN_EXPR_DEPTH: usize = 24;

/// The stack a minds-pool thread has: the platform default for a spawned
/// thread (nothing in the tree sets a larger one).
const POOL_STACK: usize = 2 * 1024 * 1024;

/// One nesting shape: an expression nested `n` deep.
struct Shape {
    name: &'static str,
    nest: fn(usize) -> String,
}

fn rep(s: &str, n: usize) -> String {
    s.repeat(n)
}

/// Every way the grammar nests an expression or a block.
const SHAPES: &[Shape] = &[
    Shape {
        name: "parens",
        nest: |n| format!("{}1{}", rep("(", n), rep(")", n)),
    },
    Shape {
        name: "unary minus",
        nest: |n| format!("{}1", rep("- ", n)),
    },
    Shape {
        name: "unary not",
        nest: |n| format!("{}true", rep("!", n)),
    },
    Shape {
        name: "arrays",
        nest: |n| format!("{}{}", rep("[", n), rep("]", n)),
    },
    Shape {
        name: "maps",
        nest: |n| format!("{}1{}", rep("#{a:", n), rep("}", n)),
    },
    Shape {
        name: "blocks",
        nest: |n| format!("{}1{}", rep("{", n), rep("}", n)),
    },
    Shape {
        name: "if",
        nest: |n| format!("{}1{}", rep("if true { ", n), rep(" }", n)),
    },
    Shape {
        name: "while",
        nest: |n| format!("{}1{}", rep("while false { ", n), rep(" }", n)),
    },
    Shape {
        name: "calls",
        nest: |n| format!("{}1{}", rep("abs(", n), rep(")", n)),
    },
    Shape {
        name: "interpolation",
        nest: |n| format!("{}1{}", rep("`${", n), rep("}`", n)),
    },
    Shape {
        name: "flat sum",
        nest: |n| format!("1{}", rep(" + 1", n)),
    },
    Shape {
        name: "flat comparison chain",
        nest: |n| format!("true{}", rep(" && true", n)),
    },
    Shape {
        name: "index chain",
        nest: |n| format!("[[1]]{}", rep("[0]", n)),
    },
    Shape {
        name: "method chain",
        nest: |n| format!("1{}", rep(".abs()", n)),
    },
    Shape {
        name: "right-nested assignment rhs",
        nest: |n| format!("{}1{}", rep("(1 + ", n), rep(")", n)),
    },
];

/// Depths to try for a shape: the guard's edges and far past them, and as
/// deep as fits under the source cap.
fn depths(shape: &Shape) -> Vec<usize> {
    let unit = (shape.nest)(1).len().max(1);
    let at_cap = SOURCE_CAP / unit;
    let mut out = vec![
        1,
        MAX_FN_EXPR_DEPTH - 1,
        MAX_FN_EXPR_DEPTH,
        MAX_FN_EXPR_DEPTH + 1,
        MAX_EXPR_DEPTH - 1,
        MAX_EXPR_DEPTH,
        MAX_EXPR_DEPTH + 1,
        256,
        2_048,
        at_cap,
    ];
    out.retain(|&d| d <= at_cap);
    out
}

/// The mind wrapping: the nested expression inside `think`, and one at the
/// top level (a separate guard depth in Rhai).
fn minds(expr: &str) -> [String; 2] {
    [
        format!("fn think(view, card) {{ let x = {expr}; }}"),
        format!("let y = {expr};\nfn think(view, card) {{ }}"),
    ]
}

fn run_on_pool_stack<F: FnOnce() + Send + 'static>(what: String, f: F) {
    std::thread::Builder::new()
        .name(what.clone())
        .stack_size(POOL_STACK)
        .spawn(f)
        .expect("spawn")
        .join()
        .unwrap_or_else(|_| panic!("{what}: panicked"));
}

/// Compile (and, if it compiles, think) one source; never abort.
fn exercise(sandbox: &Sandbox, source: &str) -> Result<(), CompileError> {
    let compiled = compile(sandbox, source)?;
    let _ = think_with(
        sandbox,
        &compiled,
        vec![Dynamic::UNIT, Dynamic::UNIT],
        b"",
        &CLASS_I,
        ROOMY,
        20,
    );
    Ok(())
}

/// Every shape at every depth compiles to an error or thinks cleanly. The
/// process surviving the loop is the assertion; the guard engaging past its
/// limit is checked for the shapes that nest expressions.
#[test]
fn every_nesting_shape_is_bounded() {
    run_on_pool_stack("nesting".into(), || {
        let sandbox = sandbox();
        for shape in SHAPES {
            for depth in depths(shape) {
                let expr = (shape.nest)(depth);
                for source in minds(&expr) {
                    if source.len() > SOURCE_CAP {
                        continue;
                    }
                    let _ = exercise(&sandbox, &source);
                }
            }
        }
    });
}

/// Every shape compiles one deep (the shapes are valid Rhai), and twice the
/// guard's depth is refused by the guard itself: an "exceeds maximum
/// complexity" parse error, not an AST deep enough to overflow on evaluation
/// or drop. Rhai counts the flat chains too.
#[test]
fn the_depth_guard_engages_past_its_limit() {
    run_on_pool_stack("guard".into(), || {
        let sandbox = sandbox();
        for shape in SHAPES {
            for source in minds(&(shape.nest)(1)) {
                assert!(
                    exercise(&sandbox, &source).is_ok(),
                    "{}: one deep must compile: {source}",
                    shape.name
                );
            }
            for source in minds(&(shape.nest)(MAX_EXPR_DEPTH * 2)) {
                match compile(&sandbox, &source) {
                    Err(CompileError::Parse(e)) if e.contains("maximum complexity") => {}
                    other => panic!(
                        "{}: {} deep was not refused by the guard: {:?}",
                        shape.name,
                        MAX_EXPR_DEPTH * 2,
                        other.map(|_| ())
                    ),
                }
            }
        }
    });
}

/// Recursion at run time is bounded by the call-level limit (12), not the
/// stack: each think faults.
#[test]
fn runtime_recursion_is_bounded() {
    run_on_pool_stack("recursion".into(), || {
        let sandbox = sandbox();
        for source in [
            "fn f(n) { f(n + 1) }
fn think(view, card) { f(0); }",
            "fn f(n) { g(n) }
fn g(n) { f(n) }
fn think(view, card) { f(0); }",
            "fn think(view, card) { think(view, card); }",
        ] {
            let compiled = compile(&sandbox, source).unwrap_or_else(|e| panic!("{source}: {e}"));
            let (result, _) = think_with(
                &sandbox,
                &compiled,
                vec![Dynamic::UNIT, Dynamic::UNIT],
                b"",
                &CLASS_I,
                ROOMY,
                20,
            );
            assert!(
                result.is_err(),
                "{source}: unbounded recursion thought cleanly"
            );
        }
    });
}
