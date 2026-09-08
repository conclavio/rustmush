//! Nested `(` in a lock key is refused, not walked into the guard page.
//!
//! **This is its own test binary on purpose.** The failure it guards against
//! is a stack overflow, and a stack overflow is a guard-page fault that kills
//! the whole test *process*: every test co-resident in the binary dies with
//! it, and the harness reports an opaque abort rather than a named failing
//! assert. Keeping this input class alone in its own target means a
//! regression names itself. Reproduce in isolation with
//! `cargo test -p mush-core --test lock_parse_depth_parens -- --test-threads=1`; a
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

/// 8192 `(` — the longest run that fits in one input line
/// (`mush_net_ssh::line::MAX_LINE_CHARS`), which is what a player can send.
///
/// Each `(` re-enters the `parse_or` → `parse_and` → `parse_unary` →
/// `parse_primary` descent, so before the frame guard this was 8192 × 4
/// frames of unbounded recursion driven straight from `@lock me=<key>`.
#[test]
fn eight_thousand_open_parens_are_refused() {
    let err = on_a_small_stack(|| {
        let src = "(".repeat(8192);
        parse_lock(&src, resolve_any).map(|_| ())
    })
    .expect_err("an 8192-deep paren nest must not parse");
    assert!(
        matches!(err, LockParseError::TooDeep { .. }),
        "expected TooDeep, got {err:?}"
    );
}
