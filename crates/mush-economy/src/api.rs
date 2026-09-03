//! The E2b public seam + persistence layer.
//!
//! [`post`] and [`reverse`] are the ONLY balance-mutation paths (the §18-E2 exit
//! criterion): each validates through the pure E2a engine, allocates ids, applies,
//! writes the whole bundle — the journal append + every touched account projection +
//! the allocator `meta` — on one `WriteBatch`, and gates the confirmation on the
//! **strict** durable ack (invariant 9 / R-R1). [`WorldAuth`] bridges the injected
//! [`AuthEval`] to `mush_org::authorize_in_world`, re-evaluated live at every post.
//!
//! Persistence (namespace [`NS`] = `economy`): `instrument:{id}`, `account:{id}`,
//! a `meta` allocator doc, and the journal under `log:journal:*`. The idempotency
//! index, the reversed set, and the txid index are NOT persisted separately — they
//! are re-derived from the journal at boot, so a companion-doc drop can never cause a
//! double-post (R-B2).

use mush_core::{Ctx, Deferred, ModuleOp, ModuleWrite, Outbox, Powers, Role, World, perm};
use mush_org::{AuthRequest, CapabilityId, PrincipalId, authorize_in_world};
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::engine::{AuthEval, apply, plan_reversal, validate};
use crate::ids::{AccountId, InstrumentId, TransactionId};
use crate::module::{EconomyStore, JOURNAL_LOG, NS};
use crate::plan::{EconomyError, EconomyPlan, PlanResult, Prepared};
use crate::record::{Account, EconomicCause, InstrumentDef};

/// The kv key of the allocator meta document.
pub const META_KEY: &str = "meta";

// ---- keys -----------------------------------------------------------------

/// `instrument:{id}`.
#[must_use]
pub fn instrument_key(id: &InstrumentId) -> SmolStr {
    SmolStr::new(format!("instrument:{}", id.as_str()))
}
/// `account:{id}`.
#[must_use]
pub fn account_key(id: AccountId) -> SmolStr {
    SmolStr::new(format!("account:{}", id.0))
}

// ---- persist helpers ------------------------------------------------------

pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(Deferred::Persist(ModuleOp { ns: SmolStr::new_static(NS), op }));
}
pub(crate) fn persist_instrument(out: &mut Outbox, def: &InstrumentDef) {
    push_op(out, ModuleWrite::JsonSet {
        key: instrument_key(&def.id),
        value: serde_json::to_value(def).expect("instrument serializes"),
    });
}
pub(crate) fn persist_account(out: &mut Outbox, a: &Account) {
    push_op(out, ModuleWrite::JsonSet {
        key: account_key(a.id),
        value: serde_json::to_value(a).expect("account serializes"),
    });
}
pub(crate) fn persist_meta(out: &mut Outbox, meta: serde_json::Value) {
    push_op(out, ModuleWrite::JsonSet { key: SmolStr::new_static(META_KEY), value: meta });
}
/// Wrap the journal `Ledger::append` write in the economy namespace.
pub(crate) fn persist_journal(out: &mut Outbox, op: ModuleWrite) {
    push_op(out, op);
}

// ---- the authorization bridge ---------------------------------------------

/// The economy capability an actor must hold to move value on a scoped account.
#[must_use]
pub fn post_capability() -> CapabilityId {
    CapabilityId::new("economy.account.post").expect("valid capability id")
}

/// Bridges the injected [`AuthEval`] to the live org authorization engine (R-D4).
/// Money-safe defaults: receiving value needs no authority; you may move your own
/// unscoped account; a scoped (org) account defers to the live org policy; a
/// system source/sink line is gated by the (staff-only) command that builds it.
pub(crate) struct WorldAuth<'a> {
    pub(crate) w: &'a World,
}

impl AuthEval for WorldAuth<'_> {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        // A credit (receiving value) needs no authority.
        if !delta.is_negative() {
            return true;
        }
        // Own it → may move it.
        if *actor == account.owner {
            return true;
        }
        // Staff may post attributed moves on any account (corrections, admin, mint).
        if let PrincipalId::Object(obj) = actor
            && (perm::has_power(self.w, *obj, Powers::SIM_ADMIN) || perm::has_role(self.w, *obj, Role::Royalty))
        {
            return true;
        }
        // System-owned accounts (faucets/sinks) are gated by the command layer.
        if matches!(account.owner, PrincipalId::System(_)) {
            return true;
        }
        // Org-scoped → the live org policy decides (E1 capability + amount limit).
        if let (Some(scope), PrincipalId::Object(obj)) = (account.scope, actor) {
            let amount = delta.minor().checked_abs().unwrap_or(i64::MAX);
            let req = AuthRequest {
                actor: *obj,
                capability: post_capability(),
                scope,
                now: self.w.now(),
                amount: Some(amount),
            };
            return authorize_in_world(self.w, &req).is_allow();
        }
        false
    }
}

// ---- instrument + account creation (non-money writes) ---------------------

/// Register (or replace) an instrument definition. Not money — a normal write.
/// The caller has validated the scale ≤ [`crate::MAX_SCALE`].
pub fn register_instrument(ctx: &mut Ctx<'_>, def: InstrumentDef) {
    persist_instrument(ctx.out, &def);
    if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
        store.snapshot.put_instrument(def);
    }
}

/// Open an account (zero balance). Not money — a normal write. The `id` must come
/// from [`EconomyStore::alloc_account`].
pub fn open_account(ctx: &mut Ctx<'_>, account: Account) {
    let meta = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return };
        store.snapshot.put_account(account.clone());
        store.meta_doc()
    };
    persist_account(ctx.out, &account);
    persist_meta(ctx.out, meta);
}

// ---- the money seam -------------------------------------------------------

/// The result of a post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostOutcome {
    /// A new transaction posted (its confirmation is strict-durable-gated).
    Posted(TransactionId),
    /// The idempotency key already resolved to this transaction (no new post).
    AlreadyPosted(TransactionId),
}

/// Post a validated plan: the whole money path. Refuses when the economy is frozen,
/// validates live, applies, writes the atomic bundle, and gates the confirmation on
/// the strict durable ack.
pub fn post(ctx: &mut Ctx<'_>, plan: &EconomyPlan) -> Result<PostOutcome, EconomyError> {
    let outcome = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or(EconomyError::EconomyFrozen)?;
        if store.frozen {
            return Err(EconomyError::EconomyFrozen);
        }
        let auth = WorldAuth { w: ctx.w };
        validate(plan, store.snapshot(), &auth)
    };
    finish(ctx, outcome)
}

/// Reverse transaction `target`: fetch the real original from the journal-derived
/// index and validate its inverse (so the `reverses` back-reference is engine-set,
/// never forgeable). Strict-durable-gated like any money move.
pub fn reverse(
    ctx: &mut Ctx<'_>,
    target: TransactionId,
    actor: PrincipalId,
    cause: EconomicCause,
) -> Result<PostOutcome, EconomyError> {
    let outcome = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or(EconomyError::EconomyFrozen)?;
        if store.frozen {
            return Err(EconomyError::EconomyFrozen);
        }
        let original = store.transaction(target).ok_or(EconomyError::NoSuchTransaction(target))?.clone();
        let auth = WorldAuth { w: ctx.w };
        plan_reversal(&original, actor, cause, store.snapshot(), &auth)
    };
    finish(ctx, outcome)
}

/// Common tail: turn a [`PlanResult`] into a durable-gated outcome or an error.
fn finish(ctx: &mut Ctx<'_>, outcome: PlanResult) -> Result<PostOutcome, EconomyError> {
    match outcome {
        PlanResult::AlreadyPosted(id) => {
            // R-R2: an idempotency HIT during the ORIGINAL's durability window must
            // not confirm before the original is durable — else a retry (a
            // deterministic reversal key, or a ruleset's stable key) could deliver
            // "already posted" for a transaction whose sole durable write then drops,
            // so it never happened at reboot (balances are journal-derived). Gating
            // rides the E0 retained-seal re-emit: a HIT's durable flush carries any
            // un-acked sealed journal chunk (the original's), so the confirmation
            // waits for it to ack. (An original still in the open chunk is covered by
            // its own in-flight strict gate; that residual sub-window closes with the
            // deferred durability checkpoint, R-D1.)
            ctx.gate_on_durable_ack_strict();
            Ok(PostOutcome::AlreadyPosted(id))
        }
        PlanResult::Refused(e) => Err(e),
        PlanResult::Post(prepared) => {
            let now = ctx.w.now();
            let txid = commit_prepared(ctx, &prepared, now);
            // Invariant 9 / R-R1: the "posted" confirmation must never outrun the
            // durable ack — strict so a dropped write suppresses it entirely.
            ctx.gate_on_durable_ack_strict();
            Ok(PostOutcome::Posted(txid))
        }
    }
}

/// Allocate ids, apply the prepared post to the live snapshot, and queue the whole
/// bundle (journal append + touched account projections + allocator meta) on one
/// write batch. The store mutation happens in a scope that ends before `ctx.out` is
/// used (the shipped module discipline).
fn commit_prepared(ctx: &mut Ctx<'_>, prepared: &Prepared, now: i64) -> TransactionId {
    let (txid, journal_op, account_docs, meta) = {
        let store = ctx.w.module_state_mut::<EconomyStore>().expect("economy store");
        let txid = store.alloc_transaction();
        // Peek the seq the journal will assign, so the transaction carries it before
        // the append (which then returns the same seq).
        let seq = store.journal.next_seq(JOURNAL_LOG);
        let (snapshot2, tx) = apply(store.snapshot(), prepared, txid, seq, now);
        let value = serde_json::to_value(&tx).expect("transaction serializes");
        let (_seq2, op) = store.journal.append(JOURNAL_LOG, value);
        debug_assert_eq!(seq, _seq2, "journal seq peeked == appended");
        store.snapshot = snapshot2;
        store.index_transaction(tx);
        let account_docs: Vec<Account> = prepared
            .touched_accounts()
            .filter_map(|id| store.snapshot().account(id).cloned())
            .collect();
        (txid, op, account_docs, store.meta_doc())
    };
    persist_journal(ctx.out, journal_op);
    for a in &account_docs {
        persist_account(ctx.out, a);
    }
    persist_meta(ctx.out, meta);
    txid
}
