//! **The budget** (17b cut M2.1; doc 17 §7 "The budget"; D-UM-26).
//!
//! A think's cost is Rhai's operation count plus the fees the game's kernels
//! charge, against the think's limit: `on_progress(ops)` ends the think with
//! the cause word `ops` the first time `ops + fee > limit`. **There is no
//! clock anywhere**: Rhai's operation count is a deterministic function of the
//! AST and the engine version, and the fees are integers, so the trip lands
//! at the same operation on every host and every run
//! (`the_budget_trips_at_the_same_op_on_two_runs`). Fees are calibrated once
//! on the blessing host at about 42 ns per op (doc 17 §7); a kernel adds its
//! fee when it is called ([`charge`]) and the trip lands at the next
//! operation — or, for a fee charged by the last call before `think` returns,
//! at the re-check [`crate::Sandbox::think`] makes after the call (that check
//! is the operation that would have come next).
//!
//! **The count is the budget's own.** Rhai hands `on_progress` the
//! `num_operations` of the runtime state it is running on, and a callback a
//! package method runs — `v.map(f)`, `filter`, `sort`, `reduce` and the rest
//! of the array and map packages, reachable by naming a script function —
//! runs on a CLONE of that state, whose count starts from the caller's and is
//! thrown away when the callback returns (rhai 1.26 `types/fn_ptr.rs:444`,
//! `func/native.rs:516`). Trusting the number Rhai reports would let a think
//! spend unbounded operations in sibling callbacks while its own count stood
//! still (17b cut M2.1, review finding F1). But every `on_progress` call is
//! exactly one operation counted (`eval/data_check.rs:189-201`), on whatever
//! clone, so [`check`] counts its calls and ignores the argument: the count
//! is every operation the think ran, deterministic as before, and once the
//! budget trips every later operation trips again — a script that catches a
//! callback's wrapped termination ends at its next operation, or at the
//! re-check after the return ([`tripped`]).
//!
//! The counters are thread-locals: one think runs on one thread from start to
//! end and a thread runs one think at a time (the crate head, "One think per
//! thread"). [`begin`] resets them at every think's start, so a think that
//! ended in a panic leaves nothing behind for the next.

use std::cell::Cell;

use rhai::Dynamic;

/// One think's account.
#[derive(Clone, Copy, Debug, Default)]
struct Budget {
    /// The think's limit: operations plus fees may not pass it.
    limit: u64,
    /// The fees charged so far (the fixed per-think fee among them).
    fee: u64,
    /// The operations counted: one per `on_progress` call.
    ops: u64,
}

thread_local! {
    static BUDGET: Cell<Budget> = Cell::new(Budget::default());
}

/// **Open a think's account** at `limit`: no fee charged, no operation
/// counted.
pub fn begin(limit: u64) {
    BUDGET.with(|budget| {
        budget.set(Budget {
            limit,
            fee: 0,
            ops: 0,
        });
    });
}

/// **Charge a fee** (D-UM-26): a kernel's price, or the think's fixed fee.
/// Saturating, so no fee can wrap the account back under its limit.
pub fn charge(fee: u64) {
    BUDGET.with(|budget| {
        let mut now = budget.get();
        now.fee = now.fee.saturating_add(fee);
        budget.set(now);
    });
}

/// **What the think has spent**: `(ops, fee)` — the operations counted and
/// the fees charged.
#[must_use]
pub fn spent() -> (u64, u64) {
    BUDGET.with(|budget| {
        let now = budget.get();
        (now.ops, now.fee)
    })
}

/// **The `on_progress` callback** (D-UM-26): counts ONE operation — the
/// number Rhai reports is its runtime state's, which a callback's clone
/// restarts (the file head), so it is not read — and answers the cause word
/// `ops`, which Rhai turns into `ErrorTerminated`, the [`crate::Fault::Ops`]
/// fault, when `ops + fee > limit`; otherwise `None` and the think runs on.
#[must_use]
pub fn check(_reported: u64) -> Option<Dynamic> {
    BUDGET.with(|budget| {
        let mut now = budget.get();
        now.ops = now.ops.saturating_add(1);
        budget.set(now);
        (now.ops.saturating_add(now.fee) > now.limit).then(|| Dynamic::from("ops"))
    })
}

/// **Whether the think is past its limit**, counting nothing: the re-check
/// [`crate::Sandbox::think`] makes once after the call returns, for a fee
/// charged by the last kernel call, which has no next operation to trip on —
/// and for a trip a script caught through a callback's wrapped error with no
/// operation after it.
#[must_use]
pub fn tripped() -> bool {
    BUDGET.with(|budget| {
        let now = budget.get();
        now.ops.saturating_add(now.fee) > now.limit
    })
}
