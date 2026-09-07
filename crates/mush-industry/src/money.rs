//! **E7r1: R1 — the work-order money `Hold`** (10b §15.1 item 6; cut 4's deferral).
//!
//! Cut 4 left `WorkOrder::budget_account` and `WorkOrder::holds` as landing pads and
//! never placed anything in them: a grep for `HoldOp` across this crate returned
//! **zero**, which is the exact sentence "production is free today". This module is
//! the money leg, and it is `crates/mush-campaign/src/bond.rs`'s shape line for line,
//! for the same reason cut 9 gives:
//!
//! * The hold is placed **from the command**, because a `prepare_authoritative`
//!   provider has no `&World`, `Ctx` or `Outbox` and therefore cannot place, capture
//!   or release one (cut 4's As-built deviation (a)). `mush_economy::post` is the
//!   economy's own carve-out lane, not a second `Ctx::submit_authoritative`, so
//!   `@wo`'s declared class does not move.
//! * It is placed **before** the submit, so an unfunded order refuses on the
//!   engine's own `EconomyError::InsufficientAvailable` (invariant 26) and reserves
//!   nothing at all. Every early return after the place compensates with
//!   [`release`], and [`sweep`] is the backstop for the returns nobody wrote.
//! * `"industry.workorder"` is in `mush_economy::ENGINE_OWNED_RESERVATIONS`, so the
//!   hold moves **only** through `mush_economy::settle_engine_reservation`. Without
//!   that, the order's own owner could `@hold/release` the reservation out from
//!   under a running order and produce for free — the precise bug the engine-owned
//!   seam was built for (E7 cut 9 fold, E2(a)).
//!
//! ## What this module does NOT do
//!
//! No faucet, no minting `AssetOp`, no `mint_origin`, no credit line and no second
//! quantity authority. The amount reserved is [`crate::workorder::quote_price`] —
//! the tree's **existing** pure function of the order's own pinned recipe, which
//! `@wo/quote` already renders — widened to an `Amount`; this pass invents no new
//! price field and no new arithmetic. A capture spends the **live hold's own**
//! amount read from the snapshot, so the ledger total is unchanged across
//! place → capture exactly as it is across place → release.
//!
//! ## Exactly once, and a vanished hold is never a hit
//!
//! Every post carries a deterministic key: `industry.wo.hold/{order}` for the
//! placement and `industry.wo.settle/{order}` for the settlement. A repeat is
//! `AlreadyPosted` — the same transaction id, no second effect. The **authority** on
//! "is this order's money still reserved" is the live hold found **by purpose**
//! (never by the id denormalized onto the record), so an `AlreadyPosted` retry and a
//! lost document converge; and an order that is holding resources with no hold at
//! all is **paused and paged**, never read as a replay hit.

use mush_core::{Ctx, ObjId};
use mush_economy::{
    Account, AccountId, AccountStatus, Amount, EconomicCause, EconomyError, EconomyPlan,
    EconomyStore, EscalationKind, EscalationSubject, Hold, HoldId, HoldOp, HoldPurpose,
    IdempotencyKey, PostOutcome, RequestedLine,
};
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::workorder::{WorkOrder, WorkOrderId, quote_price};

/// The `HoldPurpose::Reservation` namespace every work-order budget hold is placed
/// under. It is in `mush_economy::ENGINE_OWNED_RESERVATIONS`, and the two must be
/// edited together — `e7r1_r1_the_work_order_kind_is_engine_owned` is the tripwire.
pub const WORK_ORDER_PURPOSE: &str = "industry.workorder";

/// How many terminal orders one [`sweep`] fire may settle. Its **own** budget, so
/// the settlement lane can never starve the reconcile that shares the tick.
pub const MAX_WO_SETTLE_PER_FIRE: usize = 8;

/// How many work orders one [`sweep`] fire may **visit**, whether or not it settles
/// them (D-LBK-5: a walk is bounded on records visited, never on work done).
pub const MAX_WO_VISITED_PER_FIRE: usize = 64;

/// The deterministic key one order's budget placement posts under.
#[must_use]
pub fn wo_hold_key(order: WorkOrderId) -> IdempotencyKey {
    IdempotencyKey::new(
        SmolStr::new_static("industry.wo.hold"),
        SmolStr::new(order.0.to_string()),
    )
}

/// The deterministic key one order's **settlement** posts under — capture and
/// release share it, because an order settles exactly once. A capture attempted
/// after a release therefore meets the key as well as a hold that is no longer live.
#[must_use]
pub fn wo_settle_key(order: WorkOrderId) -> IdempotencyKey {
    IdempotencyKey::new(
        SmolStr::new_static("industry.wo.settle"),
        SmolStr::new(order.0.to_string()),
    )
}

/// The purpose every work-order budget hold carries. One value, built in one place,
/// so the placement and the lookup can never disagree.
#[must_use]
pub fn purpose(order: WorkOrderId) -> HoldPurpose {
    HoldPurpose::Reservation {
        kind: SmolStr::new_static(WORK_ORDER_PURPOSE),
        id: order.0,
    }
}

/// The live hold backing this order's budget, found **by purpose** rather than by
/// the id denormalized onto the record, so an `AlreadyPosted` retry and a lost
/// document both converge (bond.rs's argument, verbatim).
#[must_use]
pub fn live_hold(ctx: &Ctx<'_>, order: WorkOrderId) -> Option<Hold> {
    let want = purpose(order);
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .holds()
        .find(|h| h.purpose == want && h.status.is_held())
        .cloned()
}

/// Any hold this order ever placed, live or settled — the idempotency evidence the
/// sweep reads before deciding it still owes a settlement.
#[must_use]
pub fn any_hold(ctx: &Ctx<'_>, order: WorkOrderId) -> Option<Hold> {
    let want = purpose(order);
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .holds()
        .find(|h| h.purpose == want)
        .cloned()
}

/// The money this order reserves: [`quote_price`] over the order's own pinned
/// recipe and quantity, widened into an `Amount`. A saturating `u64` that does not
/// fit `i64` clamps at `i64::MAX` rather than wrapping — the engine then refuses the
/// placement on `InsufficientAvailable`, which is the honest answer.
#[must_use]
pub fn order_price(order: &WorkOrder) -> Amount {
    Amount::from_minor(
        i64::try_from(quote_price(order.pinned(), order.quantity())).unwrap_or(i64::MAX),
    )
}

/// What a budget operation refused on. `bond.rs`'s [`BondError`] shape, including
/// W2's transient rule: `RetryShortly` / `JournalUnavailable` / `EconomyFrozen` are
/// a **wait with no attempt counted**, never a re-apply and never a miss
/// (docs/design/13, W-D2 / W-D4).
///
/// [`BondError`]: mush_campaign
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HoldError {
    /// The economy is not composed, or is frozen.
    #[error("the economy is not available")]
    NoEconomy,
    /// The engine refused the post. `InsufficientAvailable` is the never-negative
    /// gate firing, and it is reported as such: **an unfunded order cannot submit**.
    #[error("the economy refused: {0}")]
    Refused(String),
    /// The post landed but no hold carries this order's purpose, which can only mean
    /// an `AlreadyPosted` for a hold that has since settled. Never a silent success.
    #[error("no live hold carries this work order's purpose")]
    NoHold,
    /// A post is needed but the account's approval rule fired first.
    #[error("the account's approval rule fired")]
    NeedsApproval,
    /// **A wait, not a refusal** — the caller defers without counting an attempt.
    #[error("the economy asked to be retried: {0}")]
    Transient(String),
}

impl HoldError {
    /// Whether this refusal is a **wait**.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(self, HoldError::Transient(_) | HoldError::NoEconomy)
    }
}

fn refuse(error: EconomyError) -> HoldError {
    match error {
        EconomyError::EconomyFrozen => HoldError::NoEconomy,
        EconomyError::RetryShortly | EconomyError::JournalUnavailable => {
            HoldError::Transient(error.to_string())
        }
        other => HoldError::Refused(other.to_string()),
    }
}

fn cause(order: WorkOrderId) -> EconomicCause {
    EconomicCause::Internal {
        kind: SmolStr::new_static("industry.wo.budget"),
        id: order.0,
    }
}

/// **Place the order's budget hold** (`@wo/submit`, before the submit).
///
/// One `HoldOp::Place` on the order's own `budget_account` under
/// `industry.wo.hold/{order}`. A requester without the funds cannot submit:
/// `validate`'s invariant-26 check refuses the placement with
/// `InsufficientAvailable` **before anything is written**, and the caller leaves the
/// order in `Draft`.
pub fn place(ctx: &mut Ctx<'_>, order: &WorkOrder) -> Result<HoldId, HoldError> {
    if ctx.w.module_state::<EconomyStore>().is_none() {
        return Err(HoldError::NoEconomy);
    }
    let id = order.id();
    let plan = EconomyPlan {
        hold_ops: vec![HoldOp::Place {
            account: order.budget_account(),
            amount: order_price(order),
            beneficiary: Some(order.beneficiary().clone()),
            purpose: purpose(id),
            expires: None,
        }],
        description: format!("work order {id} budget"),
        ..EconomyPlan::new(
            wo_hold_key(id),
            order.requester().clone(),
            cause(id),
            Vec::new(),
        )
    };
    match mush_economy::post(ctx, &plan) {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => {}
        Ok(PostOutcome::NeedsApproval(_)) => return Err(HoldError::NeedsApproval),
        Err(error) => return Err(refuse(error)),
    }
    live_hold(ctx, id).map(|h| h.id).ok_or(HoldError::NoHold)
}

/// **Release the budget hold unspent** — the compensating leg of every early return
/// after [`place`], and the sweep's answer to a cancelled or failed order.
///
/// Posted through `mush_economy::settle_engine_reservation`, the engine's own seam:
/// the placer clause of `WorldAuth::authorize_hold` does not authorize it, which is
/// exactly what stops the order's owner releasing it themselves.
pub fn release(ctx: &mut Ctx<'_>, order: WorkOrderId, hold: &Hold) -> Result<(), HoldError> {
    let plan = EconomyPlan {
        hold_ops: vec![HoldOp::Release { hold: hold.id }],
        description: format!("work order {order} budget released"),
        ..EconomyPlan::new(
            wo_settle_key(order),
            hold.placed_by.clone(),
            cause(order),
            Vec::new(),
        )
    };
    match mush_economy::settle_engine_reservation(ctx, &plan, hold.id, None) {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => Ok(()),
        Ok(PostOutcome::NeedsApproval(_)) => Err(HoldError::NeedsApproval),
        Err(error) => Err(refuse(error)),
    }
}

/// **Capture the budget hold** at the terminal transition — the order completed, so
/// the reservation is spent to the facility's operating account.
///
/// The captured amount is the live hold's **own** amount read from the snapshot;
/// this crate computes no number, so the capture is trivially `≤ hold.amount` and
/// the two lines are exactly zero-sum. The ledger total does not move.
pub fn capture(
    ctx: &mut Ctx<'_>,
    order: WorkOrderId,
    hold: &Hold,
    payee: AccountId,
) -> Result<(), HoldError> {
    let amount = hold.amount;
    let debit = amount.checked_neg().ok_or(HoldError::NoHold)?;
    let plan = EconomyPlan {
        hold_ops: vec![HoldOp::Capture {
            hold: hold.id,
            amount,
        }],
        description: format!("work order {order} budget captured"),
        ..EconomyPlan::new(
            wo_settle_key(order),
            hold.placed_by.clone(),
            cause(order),
            vec![
                RequestedLine {
                    account: hold.account,
                    delta: debit,
                    memo: SmolStr::new_static("industry.wo.budget"),
                },
                RequestedLine {
                    account: payee,
                    delta: amount,
                    memo: SmolStr::new_static("industry.wo.budget"),
                },
            ],
        )
    };
    match mush_economy::settle_engine_reservation(ctx, &plan, hold.id, Some(payee)) {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => Ok(()),
        Ok(PostOutcome::NeedsApproval(_)) => Err(HoldError::NeedsApproval),
        Err(error) => Err(refuse(error)),
    }
}

// ---- the accounts, validated at the EDIT (check_bond_accounts' shape) ------

/// Why a work order's money accounts do not resolve. One typed code so the
/// command renders one sentence and the sweep renders another.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountRefusal {
    /// The economy is not composed.
    NoEconomy,
    /// The budget account does not exist.
    NoBudget,
    /// The budget account is not `Active`.
    BudgetInactive,
    /// The budget account belongs to somebody else (and the actor is not staff).
    NotYours,
    /// The facility's owner has no distinct `Active` account in the budget's
    /// instrument, so a completed order would have nowhere to pay.
    NoPayee,
}

impl AccountRefusal {
    /// The one line a principal sees.
    #[must_use]
    pub const fn public(self) -> &'static str {
        match self {
            AccountRefusal::NoEconomy => {
                "The economy is not available, so a work order cannot be funded."
            }
            AccountRefusal::NoBudget => {
                "That work order names a budget account that does not exist."
            }
            AccountRefusal::BudgetInactive => {
                "That work order names a budget account that is not active."
            }
            AccountRefusal::NotYours => {
                "A work order's budget is a reservation on the requester's own account."
            }
            AccountRefusal::NoPayee => {
                "That facility's owner holds no operating account in the budget's instrument."
            }
        }
    }
}

fn payee_of(account: &Account, owner: &PrincipalId, store: &EconomyStore) -> Option<AccountId> {
    store
        .snapshot()
        .accounts_of(owner)
        .find(|candidate| {
            candidate.id != account.id
                && candidate.instrument == account.instrument
                && matches!(candidate.status, AccountStatus::Active)
        })
        .map(|candidate| candidate.id)
}

/// The account a completed order's budget is captured **to**: the facility owner's
/// first `Active` account in the budget's instrument that is not the budget account
/// itself. Deterministic (`accounts_of` is canonical order), and re-resolved at
/// settlement rather than pinned onto the record — a work order gains no field.
#[must_use]
pub fn payee_account(
    ctx: &Ctx<'_>,
    budget: AccountId,
    facility_owner: &PrincipalId,
) -> Option<AccountId> {
    let store = ctx.w.module_state::<EconomyStore>()?;
    let account = store.snapshot().account(budget)?.clone();
    payee_of(&account, facility_owner, store)
}

/// **Resolve a work order's money accounts at the edit** — `check_bond_accounts`'
/// exact shape (E7 cut 9 fold, C8), for the same reason: the placement side is saved
/// by the engine (invariants 25/26 refuse a hold on someone else's account, or an
/// unfunded one), but the **capture** side was validated nowhere, so an order could
/// run to completion and then have nowhere to pay.
///
/// # Errors
/// [`AccountRefusal`], one typed code per reason.
pub fn check_wo_accounts(
    ctx: &Ctx<'_>,
    actor: ObjId,
    requester: &PrincipalId,
    budget: AccountId,
    facility_owner: &PrincipalId,
) -> Result<AccountId, AccountRefusal> {
    let Some(store) = ctx.w.module_state::<EconomyStore>() else {
        return Err(AccountRefusal::NoEconomy);
    };
    let Some(account) = store.snapshot().account(budget).cloned() else {
        return Err(AccountRefusal::NoBudget);
    };
    if !matches!(account.status, AccountStatus::Active) {
        return Err(AccountRefusal::BudgetInactive);
    }
    // The budget is a reservation on the REQUESTER's own account, so the refusal
    // arrives here rather than at `@wo/submit`.
    if account.owner != *requester && !crate::store::is_staff(ctx.w, actor) {
        return Err(AccountRefusal::NotYours);
    }
    payee_of(&account, facility_owner, store).ok_or(AccountRefusal::NoPayee)
}

// ---- the sweep arm ---------------------------------------------------------

/// Whether this order is holding resources with **no** budget hold at all — the
/// vanished-hold shape. It is a pure predicate over live state, so the command that
/// refuses and the sweep that pages agree by construction and neither keeps a
/// second copy of the answer.
#[must_use]
pub fn budget_missing(ctx: &Ctx<'_>, order: &WorkOrder) -> bool {
    order.state().holds_resources()
        && ctx.w.module_state::<EconomyStore>().is_some()
        && live_hold(ctx, order.id()).is_none()
}

/// **The settlement arm on `mush-industry`'s existing `SWEEP_TAG` Coarse tick.**
///
/// It registers no tick, no clock, no pulse and no schedule source: a
/// `schedule_source`-fired action runs in `prepare_authoritative` with no `&World`,
/// `Ctx` or `Outbox`, so it could not place, capture or release a hold if it wanted
/// to (the kernel fact, unchanged since cut 6). `wo_deliver` has the same problem —
/// it is a provider's prepare — which is why the terminal capture lives here and not
/// at the transition.
///
/// It stages **no authoritative submission**: every post rides the economy's own
/// carve-out lane, so the tick-cadence rule (one `submit_authoritative` per actor per
/// drain) is not even approached.
///
/// Bounded on records **visited** with its own budget, advancing a rotation cursor
/// past the last *visited* id, and a **no-op past the applied point** — the hold's
/// own status is the marker, so a reboot re-running it *is* the reconcile.
pub fn sweep(ctx: &mut Ctx<'_>, families: &crate::IndustryFamilies) {
    if ctx.w.module_state::<EconomyStore>().is_none() {
        return;
    }
    let cursor = ctx
        .w
        .module_state::<crate::store::IndustryStore>()
        .and_then(|s| s.settle_cursor().map(ToOwned::to_owned));

    // One bounded page, from the rotation cursor. A separate read borrow, dropped
    // before the first post.
    let (orders, next) = {
        let page = match ctx.scan_action_state(
            &families.work_orders,
            cursor.as_deref(),
            MAX_WO_VISITED_PER_FIRE,
        ) {
            Ok(page) => page,
            Err(error) => {
                tracing::error!(
                    target: "industry",
                    %error,
                    "industry: the work-order family is unreadable; the budget sweep did nothing"
                );
                return;
            }
        };
        let rows: Vec<WorkOrder> = page.entries().iter().map(|e| e.value().clone()).collect();
        (rows, page.next_after().map(ToString::to_string))
    };
    // The rotation restarts at the head when the walk runs off the end, so no order
    // is ever permanently past the cursor.
    if let Some(store) = ctx.w.module_state_mut::<crate::store::IndustryStore>() {
        store.set_settle_cursor(next);
    }

    let mut settled = 0usize;
    for order in orders {
        if settled >= MAX_WO_SETTLE_PER_FIRE {
            break;
        }
        let id = order.id();
        if order.state().is_terminal() {
            // Past the applied point this is a no-op: the hold is no longer live.
            let Some(hold) = live_hold(ctx, id) else {
                continue;
            };
            let outcome = if matches!(
                order.state(),
                crate::workorder::WorkOrderState::Completed { .. }
            ) {
                match facility_payee(ctx, &order, hold.account) {
                    Some(payee) => capture(ctx, id, &hold, payee),
                    // Nowhere to pay: release rather than strand the money. The
                    // reservation is still spent exactly once, and the ledger total
                    // is unchanged either way.
                    None => release(ctx, id, &hold),
                }
            } else {
                release(ctx, id, &hold)
            };
            match outcome {
                Ok(()) => settled += 1,
                // W2: a transient is a WAIT. The next fire is the retry, no attempt
                // is counted, and nothing is paged.
                Err(error) if error.is_transient() => {}
                Err(error) => {
                    settled += 1;
                    page_order(
                        ctx,
                        id,
                        &format!("its budget could not be settled: {error}"),
                    );
                }
            }
        } else if !order.state().holds_resources() {
            // Not terminal and not reserving: a `Draft` (or `AwaitingApproval`)
            // order whose submission never landed. The command compensates for
            // every early return it can see; this is the backstop for the one it
            // cannot — a submission the ingress refused after the place. Released
            // on the SECOND consecutive sighting, so a hold placed in the drain
            // that is still submitting is never taken out from under it.
            let Some(hold) = live_hold(ctx, id) else {
                if let Some(store) = ctx.w.module_state_mut::<crate::store::IndustryStore>() {
                    store.clear_settle_pending(id);
                }
                continue;
            };
            let due = ctx
                .w
                .module_state_mut::<crate::store::IndustryStore>()
                .is_some_and(|store| store.note_settle_pending(id));
            if !due {
                continue;
            }
            settled += 1;
            match release(ctx, id, &hold) {
                Ok(()) | Err(HoldError::Transient(_)) | Err(HoldError::NoEconomy) => {}
                Err(error) => {
                    page_order(
                        ctx,
                        id,
                        &format!("its orphaned budget could not be released: {error}"),
                    );
                }
            }
            if let Some(store) = ctx.w.module_state_mut::<crate::store::IndustryStore>() {
                store.clear_settle_pending(id);
            }
        } else {
            // Reserving. Anything noted while it was drafting is stale now.
            if let Some(store) = ctx.w.module_state_mut::<crate::store::IndustryStore>() {
                store.clear_settle_pending(id);
            }
            if budget_missing(ctx, &order) {
                // A vanished hold is a **pause and a page, never a replay hit**. The
                // pause is `@wo/deliver`'s refusal, which reads the same predicate.
                settled += 1;
                page_order(
                    ctx,
                    id,
                    "it is holding production resources with no budget reservation",
                );
            }
        }
    }
}

/// The facility owner's payout account for a completed order, or `None`.
fn facility_payee(ctx: &Ctx<'_>, order: &WorkOrder, budget: AccountId) -> Option<AccountId> {
    let owner = ctx
        .w
        .module_state::<crate::store::IndustryStore>()
        .and_then(|store| store.facility(order.facility()).cloned())
        .map(|facility| facility.owner().clone())?;
    payee_account(ctx, budget, &owner)
}

/// Page level 1 on one order, with the recommended command filled in and **only
/// that order stopped** (08c §4.6). The economy's ladder dedupes per subject, so a
/// sweep that keeps finding the same order opens one record, not one per fire.
fn page_order(ctx: &mut Ctx<'_>, order: WorkOrderId, why: &str) {
    mush_economy::open_escalation(
        ctx,
        EscalationKind::SettlementRefused,
        EscalationSubject::WorkOrder(order.0),
        format!("work order {order}: {why}"),
        vec![format!("@wo/show {order}"), format!("@wo/cancel {order}")],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e7r1_r1_the_two_key_shapes_are_deterministic_and_inside_the_kernel_cap() {
        let key = wo_hold_key(WorkOrderId(12));
        assert_eq!(
            (key.namespace.as_str(), key.key.as_str()),
            ("industry.wo.hold", "12")
        );
        assert_eq!(key, wo_hold_key(WorkOrderId(12)));
        let settle = wo_settle_key(WorkOrderId(u64::MAX));
        assert_eq!(settle.namespace.as_str(), "industry.wo.settle");
        assert_eq!(settle, wo_settle_key(WorkOrderId(u64::MAX)));
        // The worst case a key can ever be: the widest id, the namespace included.
        for key in [wo_hold_key(WorkOrderId(u64::MAX)), settle] {
            let encoded = key.namespace.len() + 1 + key.key.len();
            assert!(
                encoded <= mush_core::MAX_IDEMPOTENCY_KEY_BYTES,
                "{encoded} bytes is past the kernel's cap"
            );
        }
        // The placement and the settlement are never the same key, and two orders
        // are never the same key.
        assert_ne!(wo_hold_key(WorkOrderId(1)), wo_settle_key(WorkOrderId(1)));
        assert_ne!(wo_settle_key(WorkOrderId(1)), wo_settle_key(WorkOrderId(2)));
    }

    #[test]
    fn e7r1_r1_the_work_order_purpose_is_one_value_built_in_one_place() {
        assert_eq!(
            purpose(WorkOrderId(5)),
            HoldPurpose::Reservation {
                kind: SmolStr::new_static("industry.workorder"),
                id: 5,
            }
        );
        assert_eq!(WORK_ORDER_PURPOSE, "industry.workorder");
    }

    /// **The table and the purpose are edited together or not at all.** Without the
    /// entry the hold is an ordinary reservation and `@hold/release` by the order's
    /// own owner would take it — production back to free, silently.
    #[test]
    fn e7r1_r1_the_work_order_kind_is_engine_owned() {
        assert!(mush_economy::ENGINE_OWNED_RESERVATIONS.contains(&WORK_ORDER_PURPOSE));
        assert!(mush_economy::is_engine_owned_reservation(&purpose(
            WorkOrderId(9)
        )));
    }

    /// Both budgets are **encoded**, not restated in prose.
    #[test]
    fn e7r1_r1_the_sweep_budgets_are_encoded() {
        assert_eq!(MAX_WO_SETTLE_PER_FIRE, 8);
        assert_eq!(MAX_WO_VISITED_PER_FIRE, 64);
        const { assert!(MAX_WO_SETTLE_PER_FIRE < MAX_WO_VISITED_PER_FIRE) }
    }
}
