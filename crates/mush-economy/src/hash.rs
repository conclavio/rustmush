//! `plan_hash` — the canonical SHA-256 of a plan's economic identity (E2a, R-B3).
//!
//! It is the linchpin of idempotency (same key + same plan ⇒ same result) and of
//! the future E3 approval round-trip (approvals bind to the hash). Two properties
//! make it safe:
//!
//! - **Determinism.** The hash is a pure function of the plan ALONE (never the
//!   snapshot), over field-ordered structs with the lines **sorted by a total key**
//!   (account, delta, memo) — so reordered request lines and any map-iteration
//!   difference hash identically. No `HashMap`-backed value is ever hashed.
//! - **Stability across a retry.** It EXCLUDES ephemeral concurrency metadata —
//!   `expected_revisions`, timestamps, and the idempotency key itself — so a retry
//!   rebuilt against a re-fetched snapshot still matches, and "same key, different
//!   plan" (a real conflict) is the only thing that changes the hash.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::ids::TransactionId;
use crate::plan::EconomyPlan;
use crate::record::EconomicVisibility;
use mush_org::PrincipalId;

#[derive(Serialize)]
struct CanonicalLine<'a> {
    account: u64,
    delta: i64,
    memo: &'a str,
}

#[derive(Serialize)]
struct CanonicalPlan<'a> {
    actor: &'a PrincipalId,
    cause: &'a crate::record::EconomicCause,
    description: &'a str,
    visibility: EconomicVisibility,
    lines: Vec<CanonicalLine<'a>>,
}

/// The canonical SHA-256 hash of `plan`'s economic identity.
#[must_use]
pub fn plan_hash(plan: &EconomyPlan) -> [u8; 32] {
    let mut lines: Vec<CanonicalLine<'_>> = plan
        .requested_lines
        .iter()
        .map(|l| CanonicalLine { account: l.account.0, delta: l.delta.minor(), memo: l.memo.as_str() })
        .collect();
    // Total order over (account, delta, memo) — reordered request lines hash the same.
    lines.sort_by(|a, b| {
        (a.account, a.delta, a.memo).cmp(&(b.account, b.delta, b.memo))
    });
    let canonical = CanonicalPlan {
        actor: &plan.actor,
        cause: &plan.cause,
        description: &plan.description,
        visibility: plan.visibility,
        lines,
    };
    // serde_json over field-ordered structs + a sorted Vec is deterministic here
    // (no floats, no maps). This is the tactics `canon()` discipline for money.
    let bytes = serde_json::to_vec(&canonical).expect("canonical plan serializes");
    let out: [u8; 32] = Sha256::digest(&bytes).into();
    out
}

/// The canonical hash of the reversal of `target` — derived from the TARGET alone,
/// not the caller's attribution. A reversal's identity is which transaction it
/// undoes (its lines are the target's, negated); the staff reason and actor are
/// attribution, not identity. So two `reverse` calls for the same target hash
/// identically and the second cleanly resolves to `AlreadyPosted` rather than a
/// spurious idempotency conflict.
#[must_use]
pub fn reversal_hash(target: TransactionId) -> [u8; 32] {
    let out: [u8; 32] = Sha256::digest(format!("econ.reverse:{}", target.0).as_bytes()).into();
    out
}
