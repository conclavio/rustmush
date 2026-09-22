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
//! Every post carries a deterministic key, and both keys carry the **attempt term**
//! the fold added (independent verify fold, C4/D1 + D5, owner decision F2(a)):
//! `industry.wo.hold/{order}/{n}` for the placement and
//! `industry.wo.settle/{order}/{n}` for the settlement, where `n` is
//! [`attempt`] — the number of holds the ledger already carries under this order's
//! own [`purpose`]. Both keys move **together**, and they must: an unscoped
//! `industry.wo.hold/{order}` was spent for ever by the first compensating release,
//! so a fully funded player who lost a submission was told their account could not
//! pay until the end of the world; and scoping the placement alone would let the
//! second attempt's capture meet a **spent settle key**, answer `AlreadyPosted`,
//! write nothing and ship the goods free.
//!
//! A repeat *of the same attempt* is still `AlreadyPosted` — the same transaction id,
//! no second effect. The **authority** on "is this order's money still reserved" is
//! the live hold found **by purpose** (never by the id denormalized onto the record),
//! so an `AlreadyPosted` retry and a lost document converge; and an order that is
//! holding resources with no hold at all is **paused and paged**, never read as a
//! replay hit — the pause is a recorded `Interrupted` state written by [`sweep`],
//! not an opinion two ingresses each hold privately.
//!
//! The attempt term is read from the ledger and never stored, so no document gains a
//! field. Its one limitation is written down rather than left to be discovered: the
//! economy garbage-collects a terminal hold past `HOLD_GC_AFTER_SECS`, so an order
//! left drafted for longer than that retention window can see `n` fall back and meet
//! its own spent key again. It then refuses **honestly** — naming `@wo/cancel` —
//! rather than blaming a solvent account.

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

/// How many settlement notes one [`sweep`] may carry between fires. The note set is
/// RAM-only and rebuilt from nothing at boot, so its ceiling was requesters ×
/// `MAX_WORK_ORDERS_PER_REQUESTER` rather than a constant (independent verify fold,
/// D9). Past this, a fresh note is **dropped and counted** — dropping one only
/// delays an orphan release by one rotation, which the idempotent settle key makes
/// safe.
pub const MAX_SETTLE_PENDING: usize = 256;

/// The deterministic key one order's budget placement posts under, scoped by the
/// **attempt term** so a compensating release does not brick the order for ever
/// (independent verify fold, C4/D1; owner decision F2(a)).
#[must_use]
pub fn wo_hold_key(order: WorkOrderId, attempt: u32) -> IdempotencyKey {
    IdempotencyKey::new(
        SmolStr::new_static("industry.wo.hold"),
        SmolStr::new(format!("{}/{attempt}", order.0)),
    )
}

/// The deterministic key one order's **settlement** posts under — capture and
/// release share it, because one *attempt* settles exactly once. A capture attempted
/// after a release of the **same** hold therefore meets the key as well as a hold
/// that is no longer live; a capture of the **next** attempt's hold meets neither,
/// which is the whole reason the two keys carry the same term and moved together.
#[must_use]
pub fn wo_settle_key(order: WorkOrderId, attempt: u32) -> IdempotencyKey {
    IdempotencyKey::new(
        SmolStr::new_static("industry.wo.settle"),
        SmolStr::new(format!("{}/{attempt}", order.0)),
    )
}

/// **The attempt term**: how many holds the ledger already carries under this
/// order's [`purpose`], live or settled. The next placement is attempt `n`; the hold
/// it creates then reads back as attempt `n` from [`attempt_of`].
///
/// Derived from the same snapshot walk [`any_hold`] already does, so no record gains
/// a field and no document changes shape.
#[must_use]
pub fn attempt(ctx: &Ctx<'_>, order: WorkOrderId) -> u32 {
    let want = purpose(order);
    ctx.w
        .module_state::<EconomyStore>()
        .map_or(0, |store| {
            store
                .snapshot()
                .holds()
                .filter(|h| h.purpose == want)
                .count()
        })
        .try_into()
        .unwrap_or(u32::MAX)
}

/// The attempt term **of one hold**: its position among the holds carrying this
/// order's purpose, in the snapshot's own id order. The hold placed as attempt `n`
/// answers `n`, so [`release`] and [`capture`] settle under the same term their
/// placement used.
#[must_use]
pub fn attempt_of(ctx: &Ctx<'_>, order: WorkOrderId, hold: HoldId) -> u32 {
    let want = purpose(order);
    ctx.w
        .module_state::<EconomyStore>()
        .and_then(|store| {
            store
                .snapshot()
                .holds()
                .filter(|h| h.purpose == want)
                .position(|h| h.id == hold)
        })
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
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
    live_hold_in(ctx.w, order)
}

/// [`live_hold`] against a bare world — the shape `project_offer` needs, because a
/// projection holds `&engine.world` and never a `Ctx` (independent verify fold, D4).
#[must_use]
pub fn live_hold_in(world: &mush_core::World, order: WorkOrderId) -> Option<Hold> {
    let want = purpose(order);
    world
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

/// What a budget operation refused on. `bond.rs`'s `BondError` shape, including
/// W2's transient rule: `RetryShortly` / `JournalUnavailable` / `EconomyFrozen` are
/// a **wait with no attempt counted**, never a re-apply and never a miss
/// (docs/design/13, W-D2 / W-D4).
///
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
    ///
    /// After the fold's attempt term this is no longer a permanent brick: a fresh
    /// attempt posts under a fresh key. It survives only for the retention edge
    /// (a hold garbage-collected out from under a long-drafted order), where the
    /// caller renders [`SPENT_ATTEMPT`] and names `@wo/cancel` rather than blaming a
    /// solvent account.
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

/// What a player is told when the placement's key is spent and no hold is live —
/// doc 13's honest answer, never *"that account cannot fund"* (independent verify
/// fold, C4/D1; owner decision F2(a)).
pub const SPENT_ATTEMPT: &str = "That work order's earlier budget reservation was already settled, so it cannot \
     be submitted again; `@wo/cancel` it and draft a fresh one.";

/// **The delivery offer's server-authored list of this actor's unfunded orders**
/// (independent verify fold, D4; F3(a) adapted).
///
/// `provider::wo_deliver` is a `prepare_authoritative` with no `&World` and no
/// `Ctx`, so it can read no money at all, and `IndustryVerb::WoDeliver`'s offer
/// projection declares exactly one parameter — `order` — which is all the prepare
/// needs. An offer-route delivery therefore reached the transition, consumed every
/// material reservation and materialised the output lot **without any money check**:
/// production for free on precisely the ingress R1 guarded on the submit side.
///
/// **Why a supplied parameter cannot close it.** A command's submission and a direct
/// offer submission take the *same* route: `Ctx::submit_authoritative` is turned into
/// an `OfferSubmission` against the actor's own projection and run through
/// `resolve_legal_action`, so anything `@wo/deliver` can put in `parameters` a
/// hand-built submission can put there too. An "attestation" parameter would be
/// forgeable in one line.
///
/// **Why a fixed parameter can.** `ProviderOfferDraft::fixed_parameters` is authored
/// by `project_offer`, which holds `&engine.world` and can therefore read the ledger;
/// `merge_parameters` refuses a submitted key that collides with a fixed one
/// (`OfferResolveError::InvalidParameters`), so a submitter can neither supply nor
/// override it. This key carries the ids of **this actor's** orders that are holding
/// production resources with no live budget hold, bounded by
/// [`MAX_UNFUNDED_CHOICES`], and `wo_deliver` refuses any order named in it. Both
/// ingresses now read one fact, computed once, with a world.
///
/// **Deviation from F3(a) as written**, recorded here rather than left to be
/// rediscovered: F3(a) asked the sweep to write `WorkOrderState::Interrupted` so both
/// ingresses read one recorded fact. A module tick cannot: `ModuleCtx`'s typed-family
/// capability is read-only by construction (*"authoritative changes still require an
/// `ActionTxn`"*) and cut 2's fold C4 forbids this arm from staging an authoritative
/// submission at all. The observable outcome F3(a) wanted is delivered in full.
pub const UNFUNDED_ORDERS: &str = "unfunded";

/// How many unfunded orders one delivery offer names. A requester can hold at most
/// [`crate::MAX_WORK_ORDERS_PER_REQUESTER`] open orders, so this is a ceiling and not
/// a truncation in any reachable world; past it the offer is refused rather than
/// silently short, because a short list is a hole.
pub const MAX_UNFUNDED_CHOICES: usize = 32;

/// What a player is told when a **transient** answer came back from the ledger —
/// doc 13 W2's wording. A wait, never a funding refusal (independent verify fold,
/// D8; owner decision F2(a)).
pub const LEDGER_BUSY: &str =
    "That request is being checked against the ledger's history; try again shortly.";

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
    // The attempt term, read from the ledger immediately before the post: a second
    // placement after a compensating release posts under a *fresh* key rather than
    // meeting its own spent one.
    let n = attempt(ctx, id);
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
            wo_hold_key(id, n),
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
    let n = attempt_of(ctx, order, hold.id);
    let plan = EconomyPlan {
        hold_ops: vec![HoldOp::Release { hold: hold.id }],
        description: format!("work order {order} budget released"),
        ..EconomyPlan::new(
            wo_settle_key(order, n),
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
///
/// **The owner-operator case is a recorded ZERO-NET posting** (independent verify
/// fold, C1/B9; owner decision F1(b)). When the facility's owner holds only the
/// budget account, `payee_of` answers that account, and the capture still writes
/// both lines — the debit and the credit land on one account, so the posted balance
/// is unmoved and only the reservation is spent. It is memoed
/// `industry.wo.self` so a reader of the journal can tell a self-transfer from a
/// payment without re-deriving the ownership question.
pub fn capture(
    ctx: &mut Ctx<'_>,
    order: WorkOrderId,
    hold: &Hold,
    payee: AccountId,
) -> Result<(), HoldError> {
    let amount = hold.amount;
    let debit = amount.checked_neg().ok_or(HoldError::NoHold)?;
    let n = attempt_of(ctx, order, hold.id);
    let memo = if payee == hold.account {
        SmolStr::new_static("industry.wo.self")
    } else {
        SmolStr::new_static("industry.wo.budget")
    };
    let plan = EconomyPlan {
        hold_ops: vec![HoldOp::Capture {
            hold: hold.id,
            amount,
        }],
        description: if payee == hold.account {
            format!("work order {order} budget captured (zero-net self-transfer)")
        } else {
            format!("work order {order} budget captured")
        },
        ..EconomyPlan::new(
            wo_settle_key(order, n),
            hold.placed_by.clone(),
            cause(order),
            vec![
                RequestedLine {
                    account: hold.account,
                    delta: debit,
                    memo: memo.clone(),
                },
                RequestedLine {
                    account: payee,
                    delta: amount,
                    memo,
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
    /// The facility's owner has no `Active` account in the budget's instrument at
    /// all, so a completed order would have nowhere to pay. Since F1(b) an
    /// owner-operator's *own* budget account satisfies this, so the refusal now means
    /// what it says: the owner holds nothing payable in that instrument.
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

/// The facility owner's payout account for a budget on `account`.
///
/// **F1(b) (independent verify fold, C1/B9):** the distinctness clause is a
/// preference, not a requirement. A facility owner who holds a *second* `Active`
/// account in the instrument is paid into it, exactly as before; an owner-operator
/// who holds only the budget account is paid into **that** account, and the capture
/// is a recorded zero-net self-transfer rather than a refusal to draft at all. The
/// clause was the whole of `AccountRefusal::NoPayee`'s reach in the shipped tree,
/// where `@facility/open` is staff-only and hardcodes the opener as owner — so
/// keeping it meant a one-account owner could not draft, and a two-account owner
/// paid themselves. Both are now honest and both are recorded.
fn payee_of(account: &Account, owner: &PrincipalId, store: &EconomyStore) -> Option<AccountId> {
    let eligible = |candidate: &&Account| {
        candidate.instrument == account.instrument
            && matches!(candidate.status, AccountStatus::Active)
    };
    let snapshot = store.snapshot();
    snapshot
        .accounts_of(owner)
        .find(|candidate| candidate.id != account.id && eligible(candidate))
        .or_else(|| {
            // The owner-operator fall-back: only their OWN budget account, and only
            // when the budget belongs to them. A requester paying somebody else's
            // facility never lands here.
            (account.owner == *owner)
                .then(|| snapshot.accounts_of(owner).find(|c| c.id == account.id))
                .flatten()
        })
        .map(|candidate| candidate.id)
}

/// The account a completed order's budget is captured **to**: the facility owner's
/// first `Active` account in the budget's instrument, preferring one that is not the
/// budget account itself and falling back to the budget account when the facility's
/// owner *is* the budget's owner (F1(b)). Deterministic (`accounts_of` is canonical
/// order), and re-resolved at settlement rather than pinned onto the record — a work
/// order gains no field.
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
    _actor: ObjId,
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
    // The budget is a reservation on the REQUESTER's own account — **including for
    // staff** (independent verify fold, C13). The staff escape used to let a staffer
    // name any player's account as the budget of an order drafted under
    // `PrincipalId::Object(ctx.actor)`; the resulting hold carries
    // `"industry.workorder"`, which is in `ENGINE_OWNED_RESERVATIONS`, so
    // `authorize_hold` refuses the account's own owner **unconditionally** and the
    // player could not get their money back without a staff `@wo/cancel`. Staff can
    // already move money directly; what they may not do is reserve somebody else's
    // with no player-side exit. The diagnostic refusals above (`NoBudget`,
    // `BudgetInactive`) stay exactly as they were, because reading a status strands
    // nothing.
    if account.owner != *requester {
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
    budget_missing_in(ctx.w, order)
}

/// [`budget_missing`] against a bare world, so `project_offer` and the command arm
/// read the **same** predicate (independent verify fold, D4).
#[must_use]
pub fn budget_missing_in(world: &mush_core::World, order: &WorkOrder) -> bool {
    order.state().holds_resources()
        && world.module_state::<EconomyStore>().is_some()
        && live_hold_in(world, order.id()).is_none()
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
                // D11 (independent verify fold): the cause resolved itself. A
                // terminal order with nothing reserved is finished, so the level-1
                // record the settlement opened about it is acknowledged here — the
                // seam E5a added for exactly this. An operator who typed the
                // recommended `@wo/cancel` no longer keeps a standing page about a
                // finished order.
                ack_order(ctx, id);
                continue;
            };
            let outcome = if matches!(
                order.state(),
                crate::workorder::WorkOrderState::Completed { .. }
            ) {
                match facility_payee(ctx, &order, hold.account) {
                    Some(payee) => capture(ctx, id, &hold, payee),
                    // C3/F3(a) (independent verify fold): nowhere to pay is a
                    // settlement **REFUSAL**, not a refund. `wo_deliver` materialises
                    // the output lot at the transition while the capture waits for
                    // this Coarse tick, so releasing here handed the requester the
                    // goods AND every coin back. The hold stays live, the money stays
                    // reserved, and the page names the exit.
                    None => {
                        settled += 1;
                        page_order(
                            ctx,
                            id,
                            "its facility owner holds no account to be paid into",
                            &[format!("@wo/show {id}"), payee_recommendation(ctx, &order)],
                        );
                        continue;
                    }
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
                        &[format!("@wo/show {id}"), format!("@wo/cancel {id}")],
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
            // D8 (independent verify fold): a doc-13 transient is a **wait**, and it
            // must cost this order nothing. The `continue` runs BEFORE `settled += 1`
            // and before `clear_settle_pending`, so a blip neither burns one of
            // `MAX_WO_SETTLE_PER_FIRE` nor resets the two-sighting counter — which
            // used to cost the orphan two more full rotations (at
            // `SWEEP_PERIOD_SECS`) per blip.
            match release(ctx, id, &hold) {
                Err(error) if error.is_transient() => continue,
                Ok(()) => {}
                Err(error) => {
                    page_order(
                        ctx,
                        id,
                        &format!("its orphaned budget could not be released: {error}"),
                        &[format!("@wo/show {id}"), format!("@wo/cancel {id}")],
                    );
                }
            }
            settled += 1;
            if let Some(store) = ctx.w.module_state_mut::<crate::store::IndustryStore>() {
                store.clear_settle_pending(id);
            }
        } else {
            // Reserving. Anything noted while it was drafting is stale now.
            if let Some(store) = ctx.w.module_state_mut::<crate::store::IndustryStore>() {
                store.clear_settle_pending(id);
            }
            if budget_missing(ctx, &order) {
                // D4/B5 (independent verify fold; F3(a) adapted — see the deviation
                // note on `UNFUNDED_ORDERS`): a vanished hold stops **both**
                // ingresses, and the page names the subject and its exit. The sweep
                // cannot itself write `Interrupted`: a module tick holds a `Ctx`,
                // whose typed-family capability is read-only by construction
                // (`ModuleCtx::action_state` — *"authoritative changes still require
                // an `ActionTxn`"*), and the alternative would be one
                // `submit_authoritative` per paused order, which cut 2's fold C4
                // forbids this arm outright. The delivery ingress is therefore closed
                // where it can be closed exactly once: in the provider's own prepare,
                // on the parameter schema that already closes `wo_submit`.
                settled += 1;
                page_order(
                    ctx,
                    id,
                    "it is holding production resources with no budget reservation",
                    &[format!("@wo/show {id}"), format!("@wo/cancel {id}")],
                );
            }
        }
    }
}

/// Acknowledge the settlement page on a finished order, if one stands (D11).
fn ack_order(ctx: &mut Ctx<'_>, order: WorkOrderId) {
    let open = ctx.w.module_state::<EconomyStore>().and_then(|store| {
        store
            .escalations()
            .find(|e| e.subject == EscalationSubject::WorkOrder(order.0) && e.is_open())
            .map(|e| e.id)
    });
    if let Some(id) = open {
        let by = ctx.actor;
        mush_economy::acknowledge_escalation(ctx, id, by);
    }
}

/// The `@account/open` line a `NoPayee` settlement refusal recommends, with the
/// facility owner's dbref and the budget's instrument filled in (F3(a)).
fn payee_recommendation(ctx: &Ctx<'_>, order: &WorkOrder) -> String {
    let owner = ctx
        .w
        .module_state::<crate::store::IndustryStore>()
        .and_then(|store| store.facility(order.facility()).cloned())
        .map(|facility| facility.owner().clone());
    let instrument = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|store| store.snapshot().account(order.budget_account()).cloned())
        .map(|account| account.instrument.to_string());
    match (owner, instrument) {
        (Some(PrincipalId::Object(obj)), Some(instrument)) => {
            format!("@account/open #{}={instrument}", obj.0)
        }
        (_, Some(instrument)) => format!("@account/open <owner>={instrument}"),
        _ => "@account/open <owner>=<instrument>".to_owned(),
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
fn page_order(ctx: &mut Ctx<'_>, order: WorkOrderId, why: &str, recommended: &[String]) {
    mush_economy::open_escalation(
        ctx,
        EscalationKind::SettlementRefused,
        EscalationSubject::WorkOrder(order.0),
        format!("work order {order}: {why}"),
        recommended.to_vec(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e7r1_r1_the_two_key_shapes_are_deterministic_and_inside_the_kernel_cap() {
        let key = wo_hold_key(WorkOrderId(12), 0);
        assert_eq!(
            (key.namespace.as_str(), key.key.as_str()),
            ("industry.wo.hold", "12/0")
        );
        assert_eq!(key, wo_hold_key(WorkOrderId(12), 0));
        let settle = wo_settle_key(WorkOrderId(u64::MAX), u32::MAX);
        assert_eq!(settle.namespace.as_str(), "industry.wo.settle");
        assert_eq!(settle, wo_settle_key(WorkOrderId(u64::MAX), u32::MAX));
        // The worst case a key can ever be: the widest id, the widest attempt, the
        // namespace included.
        for key in [wo_hold_key(WorkOrderId(u64::MAX), u32::MAX), settle] {
            let encoded = key.namespace.len() + 1 + key.key.len();
            assert!(
                encoded <= mush_core::MAX_IDEMPOTENCY_KEY_BYTES,
                "{encoded} bytes is past the kernel's cap"
            );
        }
        // The placement and the settlement are never the same key, and two orders
        // are never the same key.
        assert_ne!(
            wo_hold_key(WorkOrderId(1), 0),
            wo_settle_key(WorkOrderId(1), 0)
        );
        assert_ne!(
            wo_settle_key(WorkOrderId(1), 0),
            wo_settle_key(WorkOrderId(2), 0)
        );
    }

    /// **The attempt term is what stops a compensating release bricking an order**
    /// (independent verify fold, C4/D1; owner decision F2(a)): two attempts on one
    /// order are two different placement keys AND two different settlement keys, and
    /// the two shapes move together — a settle key that did not carry the term would
    /// let attempt 1's capture answer `AlreadyPosted` and ship the goods free (D5's
    /// refutation, which expires the instant one key moves alone).
    #[test]
    fn e7r1_r1_both_money_keys_carry_the_attempt_term_and_move_together() {
        let order = WorkOrderId(7);
        assert_ne!(wo_hold_key(order, 0), wo_hold_key(order, 1));
        assert_ne!(wo_settle_key(order, 0), wo_settle_key(order, 1));
        // The order is still the outer scope: two orders never collide at any attempt.
        assert_ne!(
            wo_hold_key(WorkOrderId(1), 1),
            wo_hold_key(WorkOrderId(11), 0)
        );
        assert_ne!(
            wo_settle_key(WorkOrderId(1), 1),
            wo_settle_key(WorkOrderId(11), 0)
        );
        // Both shapes end in `/{n}`: neither can be scoped without the other.
        assert!(wo_hold_key(order, 3).key.as_str().ends_with("/3"));
        assert!(wo_settle_key(order, 3).key.as_str().ends_with("/3"));
    }

    /// The settle-note ceiling is **encoded**, not restated in prose (D9).
    #[test]
    fn vf_d9b_the_settle_pending_cap_is_encoded() {
        assert_eq!(MAX_SETTLE_PENDING, 256);
        const { assert!(MAX_SETTLE_PENDING >= MAX_WO_VISITED_PER_FIRE) }
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
