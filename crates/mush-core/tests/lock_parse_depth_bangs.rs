//! A run of `!` in a lock key is refused, not walked into the guard page.
//!
//! **This is its own test binary on purpose.** The failure it guards against
//! is a stack overflow, and a stack overflow is a guard-page fault that kills
//! the whole test *process*: every test co-resident in the binary dies with
//! it, and the harness reports an opaque abort rather than a named failing
//! assert. Keeping this input class alone in its own target means a
//! regression names itself. Reproduce in isolation with
//! `cargo test -p mush-core --test lock_parse_depth_bangs -- --test-threads=1`; a
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

/// 8192 `!` — one full input line of negation.
///
/// `parse_unary` calls itself for each `!`, so this is one frame per
/// character of unbounded recursion. The frame guard stops the descent long
/// before the AST-depth guard would have a tree to measure.
#[test]
fn eight_thousand_bangs_are_refused() {
    let err = on_a_small_stack(|| {
        let src = "!".repeat(8192);
        parse_lock(&src, resolve_any).map(|_| ())
    })
    .expect_err("an 8192-deep negation nest must not parse");
    assert!(
        matches!(err, LockParseError::TooDeep { .. }),
        "expected TooDeep, got {err:?}"
    );
}
