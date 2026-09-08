//! A flat `&`/`|` chain is refused — the class a frame counter cannot see.
//!
//! **This is its own test binary on purpose.** The failure it guards against
//! is a stack overflow, and a stack overflow is a guard-page fault that kills
//! the whole test *process*: every test co-resident in the binary dies with
//! it, and the harness reports an opaque abort rather than a named failing
//! assert. Keeping this input class alone in its own target means a
//! regression names itself. Reproduce in isolation with
//! `cargo test -p mush-core --test lock_parse_depth_chain -- --test-threads=1`; a
//! regression presents as an opaque harness abort — `error: test failed, to
//! rerun pass ...` with a signal or STATUS_STACK_OVERFLOW and no failing
//! assertion — not as a red assert.
//!
//! The parse runs on a deliberately small (128 KiB) thread stack so the
//! overflow happens here, quickly and locally, instead of depending on how
//! much stack the test harness happened to leave.

use std::thread;

use mush_core::lock::{LockParseError, parse_lock};
use mush_core::obj::ObjId;

/// A resolver that accepts anything, so the parse fails for depth and for no
/// other reason.
fn resolve_any(_name: &str) -> Option<ObjId> {
    Some(ObjId(1))
}

/// Run `f` on a 128 KiB stack and hand back what it returned.
fn on_a_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    thread::Builder::new()
        .stack_size(128 * 1024)
        .name("lock-depth".into())
        .spawn(f)
        .expect("spawn the small-stack thread")
        .join()
        .expect("the parse must return an error, not die on the guard page")
}

/// `#1&#1&…#1`, 2730 operators, 8192 characters: one input line.
///
/// This is the input the *frame* guard is blind to. `parse_and` is a loop —
/// O(1) stack no matter how long the chain — and it left-nests, so before
/// this step the parser happily returned `Ok` with a ~2730-deep `And` tree.
/// That tree then recursed everywhere it was touched afterwards: `Display`,
/// `eval_lock`, serde on the way to Redis, and `Drop`. So a stack-safe
/// parser is not enough; the *tree* has to be bounded too, and that is what
/// `MAX_LOCK_EXPR_DEPTH` does.
///
/// Run on the small stack as well: pre-change this test did not die in the
/// parser, it died dropping what the parser built.
#[test]
fn a_full_line_of_ands_is_refused() {
    let err = on_a_small_stack(|| {
        let src = format!("{}#1", "#1&".repeat(2730));
        assert_eq!(src.len(), 8192, "the chain must be a full input line");
        parse_lock(&src, resolve_any).map(|_| ())
    })
    .expect_err("a 2730-term & chain must not parse");
    assert!(
        matches!(err, LockParseError::TooDeep { .. }),
        "expected TooDeep, got {err:?}"
    );
}

/// The `|` twin: same shape, the other operator, its own loop in `parse_or`.
#[test]
fn a_full_line_of_ors_is_refused() {
    let err = on_a_small_stack(|| {
        let src = format!("{}#1", "#1|".repeat(2730));
        assert_eq!(src.len(), 8192, "the chain must be a full input line");
        parse_lock(&src, resolve_any).map(|_| ())
    })
    .expect_err("a 2730-term | chain must not parse");
    assert!(
        matches!(err, LockParseError::TooDeep { .. }),
        "expected TooDeep, got {err:?}"
    );
}
