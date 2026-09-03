//! The pure posting engine (E2a) — [`validate`], [`apply`], and [`reversal_plan`].
//!
//! [`validate`] answers *may this plan post, against this immutable snapshot, for
//! this actor?* over the nine §7.1 invariants, and returns a [`Prepared`] that
//! [`apply`] folds **infallibly** (validation already proved it against the pre-state
//! — the M32 `unit::apply` shape). It is pure and world-free; authorization is
//! injected as an [`AuthEval`] so `mush-economy` never depends on the org module.
//!
//! The boundedness caps (R-B5) are validate invariants, not an afterthought: an
//! oversized entry would build a journal chunk past `MAX_SAFE_CHUNK_BYTES` and latch
//! the E0 poison-freeze at APPLY time — so "infallible apply" would be false. Every
//! accepted plan provably serializes below [`MAX_ENTRY_BYTES`].

use std::collections::BTreeMap;

use mush_org::PrincipalId;

use crate::amount::Amount;
use crate::hash::plan_hash;
use crate::ids::{AccountId, InstrumentId, TransactionId};
use crate::plan::{EconomyError, EconomyPlan, PlanResult, Prepared};
use crate::record::{
    Account, AccountStatus, EconomicTransaction, IdempotencyRecord, InstrumentStatus, JournalLine,
};
use crate::snapshot::EconomySnapshot;

// ---- boundedness caps (R-B5) ----------------------------------------------

/// Maximum journal lines in one transaction.
pub const MAX_LINES_PER_TX: usize = 256;
/// Maximum bytes in a line memo.
pub const MAX_MEMO_BYTES: usize = 512;
/// Maximum bytes in a transaction description.
pub const MAX_DESCRIPTION_BYTES: usize = 4096;
/// Maximum serialized bytes of one journal entry. Far below the ledger's 8 MiB
/// `MAX_SAFE_CHUNK_BYTES` poison threshold, so no accepted plan can build a chunk
/// the store rejects — a fresh chunk holding one accepted entry stays well under.
pub const MAX_ENTRY_BYTES: usize = 256 * 1024;

// ---- the authorization seam -----------------------------------------------

/// Resolves *may this actor move value on this account, now?* — re-evaluated LIVE at
/// every post (R-D4), never a captured revision. `mush-economy` defines the seam;
/// E2b bridges it to `mush_org::authorize_in_world`. A system source/sink line is
/// the module's own to authorize; a player line checks the actor against the
/// account's scope.
pub trait AuthEval {
    /// `true` if `actor` may apply `delta` to `account`; `false` denies (mapped to
    /// [`EconomyError::Unauthorized`]).
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool;
}

/// An [`AuthEval`] that authorizes everything — for tests and trusted internal posts.
pub struct AllowAll;
impl AuthEval for AllowAll {
    fn authorize(&self, _: &PrincipalId, _: &Account, _: Amount) -> bool {
        true
    }
}

// ---- validate -------------------------------------------------------------

/// Validate `plan` against `snapshot` for its actor. Pure + total; mutates nothing.
#[must_use]
pub fn validate(plan: &EconomyPlan, snapshot: &EconomySnapshot, auth: &dyn AuthEval) -> PlanResult {
    let hash = plan_hash(plan);

    // Idempotency FIRST: an exact repeat short-circuits to the original id; a reuse
    // with a different plan is an error (never a silent re-post).
    if let Some(rec) = snapshot.idempotency(&plan.idempotency) {
        if rec.plan_hash == hash {
            return PlanResult::AlreadyPosted(rec.transaction);
        }
        return PlanResult::Refused(EconomyError::IdempotencyConflict);
    }

    match validate_inner(plan, snapshot, auth, hash) {
        Ok(prepared) => PlanResult::Post(Box::new(prepared)),
        Err(e) => PlanResult::Refused(e),
    }
}

fn validate_inner(
    plan: &EconomyPlan,
    snapshot: &EconomySnapshot,
    auth: &dyn AuthEval,
    hash: [u8; 32],
) -> Result<Prepared, EconomyError> {
    // Boundedness caps (R-B5) — cheap, first, so a huge plan is refused before work.
    if plan.requested_lines.len() > MAX_LINES_PER_TX {
        return Err(EconomyError::TooLarge("too many lines"));
    }
    if plan.description.len() > MAX_DESCRIPTION_BYTES {
        return Err(EconomyError::TooLarge("description too long"));
    }
    for l in &plan.requested_lines {
        if l.memo.len() > MAX_MEMO_BYTES {
            return Err(EconomyError::TooLarge("memo too long"));
        }
    }

    // Expected-revision staleness on ECONOMIC data (authorization is live, below).
    for e in &plan.expected_revisions {
        let cur = snapshot.account(e.account).map(|a| a.revision);
        if cur != Some(e.revision) {
            return Err(EconomyError::Stale { account: e.account, expected: e.revision, found: cur });
        }
    }

    // Resolve every line: account active, instrument active + matching, delta non-zero.
    // Build the canonical journal lines and the net per-account + per-instrument sums.
    let mut lines: Vec<JournalLine> = Vec::with_capacity(plan.requested_lines.len());
    let mut per_account: BTreeMap<AccountId, i128> = BTreeMap::new();
    let mut per_instrument: BTreeMap<InstrumentId, i128> = BTreeMap::new();
    let mut non_zero = 0usize;

    for (i, rl) in plan.requested_lines.iter().enumerate() {
        if rl.delta.is_zero() {
            return Err(EconomyError::ZeroLine(i));
        }
        non_zero += 1;
        let account = snapshot.account(rl.account).ok_or(EconomyError::NoSuchAccount(rl.account))?;
        if account.status != AccountStatus::Active {
            return Err(EconomyError::InactiveAccount(rl.account));
        }
        let instrument = snapshot
            .instrument(&account.instrument)
            .ok_or_else(|| EconomyError::NoSuchInstrument(account.instrument.clone()))?;
        if instrument.status != InstrumentStatus::Active {
            return Err(EconomyError::InactiveInstrument(account.instrument.clone()));
        }
        *per_account.entry(rl.account).or_insert(0) += rl.delta.widen();
        *per_instrument.entry(account.instrument.clone()).or_insert(0) += rl.delta.widen();
        lines.push(JournalLine {
            account: rl.account,
            instrument: account.instrument.clone(),
            delta: rl.delta,
            memo: rl.memo.clone(),
        });
    }

    // Invariant 1: at least two non-zero lines.
    if non_zero < 2 {
        return Err(EconomyError::TooFewLines);
    }
    // Invariant 3: each instrument balances to exactly zero.
    for (instrument, sum) in &per_instrument {
        if *sum != 0 {
            return Err(EconomyError::NotBalanced(instrument.clone()));
        }
    }

    // Invariant 4 + credit policy: fold each net delta into the account's balance
    // with checked arithmetic and refuse a credit-policy breach.
    let mut balance_deltas: BTreeMap<AccountId, Amount> = BTreeMap::new();
    for (&account_id, &net128) in &per_account {
        let net = Amount::from_widened(net128).ok_or(EconomyError::Overflow)?;
        let account = snapshot.account(account_id).expect("resolved above");
        let new_balance = account.posted_balance.checked_add(net).ok_or(EconomyError::Overflow)?;
        if !account.credit.permits(new_balance) {
            return Err(EconomyError::CreditViolation(account_id));
        }
        balance_deltas.insert(account_id, net);
    }

    // Authorization — re-evaluated LIVE per touched account (R-D4).
    for (&account_id, &net) in &balance_deltas {
        let account = snapshot.account(account_id).expect("resolved above");
        if !auth.authorize(&plan.actor, account, net) {
            return Err(EconomyError::Unauthorized);
        }
    }

    // Canonical line order (matches the hash's total key) so the persisted entry is
    // deterministic regardless of request order.
    lines.sort_by(|a, b| {
        (a.account.0, a.delta.minor(), a.memo.as_str()).cmp(&(b.account.0, b.delta.minor(), b.memo.as_str()))
    });

    let prepared = Prepared {
        idempotency: plan.idempotency.clone(),
        plan_hash: hash,
        actor: plan.actor.clone(),
        cause: plan.cause.clone(),
        description: plan.description.clone(),
        visibility: plan.visibility,
        lines,
        // A normal post never reverses; only `reverse` sets this, from a real tx.
        reverses: None,
        balance_deltas,
    };

    // Boundedness: the serialized entry must fit MAX_ENTRY_BYTES so apply can never
    // build a poison-sized chunk (R-B5). Measure the real entry shape.
    let probe = entry_probe(&prepared);
    let size = serde_json::to_vec(&probe).map(|v| v.len()).unwrap_or(usize::MAX);
    if size > MAX_ENTRY_BYTES {
        return Err(EconomyError::TooLarge("serialized entry exceeds the per-entry cap"));
    }

    Ok(prepared)
}

/// Build the exact `EconomicTransaction` shape that would be journaled, with
/// placeholder id/seq/time, purely to measure its serialized size in validate.
fn entry_probe(p: &Prepared) -> EconomicTransaction {
    EconomicTransaction {
        id: TransactionId(0),
        journal_seq: 0,
        idempotency: p.idempotency.clone(),
        cause: p.cause.clone(),
        actor: p.actor.clone(),
        posted_at: 0,
        description: p.description.clone(),
        lines: p.lines.clone(),
        reverses: p.reverses,
        visibility: p.visibility,
        plan_hash: p.plan_hash,
    }
}

// ---- apply ----------------------------------------------------------------

/// Apply a validated [`Prepared`] to `snapshot`, returning the advanced snapshot and
/// the immutable transaction. The module supplies the allocated `txid`, monotonic
/// `journal_seq`, and `posted_at`. Infallible: validation proved every checked op.
#[must_use]
pub fn apply(
    snapshot: &EconomySnapshot,
    prepared: &Prepared,
    txid: TransactionId,
    journal_seq: u64,
    posted_at: i64,
) -> (EconomySnapshot, EconomicTransaction) {
    let mut next = snapshot.clone();
    for (&account_id, &net) in &prepared.balance_deltas {
        let account = next.account_mut(account_id).expect("validated account exists");
        account.posted_balance = account
            .posted_balance
            .checked_add(net)
            .expect("validate proved no overflow");
        account.last_journal_seq = journal_seq;
        account.revision += 1;
    }
    let tx = EconomicTransaction {
        id: txid,
        journal_seq,
        idempotency: prepared.idempotency.clone(),
        cause: prepared.cause.clone(),
        actor: prepared.actor.clone(),
        posted_at,
        description: prepared.description.clone(),
        lines: prepared.lines.clone(),
        reverses: prepared.reverses,
        visibility: prepared.visibility,
        plan_hash: prepared.plan_hash,
    };
    next.put_idempotency(IdempotencyRecord {
        idempotency: prepared.idempotency.clone(),
        plan_hash: prepared.plan_hash,
        transaction: txid,
        posted_at,
    });
    if let Some(target) = prepared.reverses {
        next.mark_reversed(target);
    }
    (next, tx)
}

// ---- reversal -------------------------------------------------------------

/// Validate the reversal of `original` — a dedicated path so the `reverses`
/// back-reference is set ONLY by the engine from a **real transaction the caller
/// supplied** (E2b fetches it from the journal by id), never a forgeable plan field.
///
/// It builds the inverse lines, uses the deterministic idempotency key
/// `econ.reverse/{id}`, and validates the inverse post against the CURRENT snapshot
/// (live balances, credit policy, authorization). A repeat resolves to
/// [`PlanResult::AlreadyPosted`]; the at-most-once [`EconomySnapshot::is_reversed`]
/// guard is the backstop. A reversal may itself be reversed — a distinct new
/// transaction with its own deterministic key.
#[must_use]
pub fn plan_reversal(
    original: &EconomicTransaction,
    actor: PrincipalId,
    cause: crate::record::EconomicCause,
    snapshot: &EconomySnapshot,
    auth: &dyn AuthEval,
) -> PlanResult {
    // Inverse lines; `i64::MIN` has no negation (cannot arise from a balanced posted
    // set in practice, but refused rather than wrapped).
    let mut lines = Vec::with_capacity(original.lines.len());
    for l in &original.lines {
        let Some(neg) = l.delta.checked_neg() else {
            return PlanResult::Refused(EconomyError::Overflow);
        };
        lines.push(crate::plan::RequestedLine { account: l.account, delta: neg, memo: l.memo.clone() });
    }
    let plan = EconomyPlan {
        idempotency: crate::record::IdempotencyKey::reversal_of(original.id),
        actor,
        description: format!("reversal of transaction {}", original.id),
        cause,
        visibility: original.visibility,
        requested_lines: lines,
        expected_revisions: Vec::new(),
    };
    // The reversal's identity is the TARGET, not the caller's cause/actor — so a
    // re-reverse with a different staff reason still resolves to AlreadyPosted.
    let hash = crate::hash::reversal_hash(original.id);

    // A clean retry of the SAME reversal returns the original reversal id.
    if let Some(rec) = snapshot.idempotency(&plan.idempotency) {
        if rec.plan_hash == hash {
            return PlanResult::AlreadyPosted(rec.transaction);
        }
        return PlanResult::Refused(EconomyError::IdempotencyConflict);
    }
    // At-most-once backstop: refuse if the target is already reversed by any path.
    if snapshot.is_reversed(original.id) {
        return PlanResult::Refused(EconomyError::AlreadyReversed(original.id));
    }
    match validate_inner(&plan, snapshot, auth, hash) {
        Ok(mut prepared) => {
            prepared.reverses = Some(original.id);
            PlanResult::Post(Box::new(prepared))
        }
        Err(e) => PlanResult::Refused(e),
    }
}
