//! The exact-mode bridge (E5b, 08d §4's "exact mode" and D-E5-2).
//!
//! **No housing code moves money.** In exact mode a tenancy is the shadow of two E4
//! records — a lease contract and the rent obligation it creates at activation — and
//! everything this file does is read their transitions and mirror them onto the
//! tenancy ladder. The rent is settled by the economy sweep, under the economy's own
//! authority, out of the tenant's own account; housing stores two ids.
//!
//! ## The flow, end to end
//!
//! 1. `@tenancy/create <property>=<tenant…> mode=exact rent=<amount> from=<acct>
//!    to=<acct>` drafts and offers a **lease contract**
//!    ([`mush_economy::offer_lease`]): parties are the landlord and the tenant(s), the
//!    landlord's acceptance is the offer itself (it is a staff act on the landlord's
//!    behalf, exactly the `landlord_accepted` half of E5a's `accept`), the premises
//!    are pledged as the landlord's collateral, and the rent is an `Obligation`
//!    deliverable. The tenancy is written `Pending` with
//!    [`crate::Funding::Exact`] naming the contract and [`PENDING_OBLIGATION`].
//! 2. **The tenant accepts** — `@contract/accept <id>`, their own verb under their own
//!    rights. That acceptance is the consent, and it is the last one, so the contract
//!    activates: the economy places the `Lease` encumbrance on the premises and
//!    creates the rent obligation with `cause: Contract(id)`.
//! 3. This observer sees `ContractChanged { state: Active }`, finds that obligation,
//!    writes it into the tenancy's funding, and runs E5a's `accept` — so the tenancy
//!    goes `Active`, `Obj.home` is pointed at the anchor, and the property latches
//!    `Let`, through exactly the same code path abstract mode uses.
//! 4. Every period the sweep settles the obligation. When it does not, the obligation
//!    goes `Grace` then `Delinquent`, this observer mirrors that onto the tenancy, and
//!    the tick does the period arithmetic that reaches the story window
//!    ([`crate::tick`]).
//! 5. A terminal contract — settled after a staff attestation, or cancelled — ends the
//!    tenancy with [`crate::Resolution::Settled`]. That is not an automatic ending:
//!    every route into a terminal contract state is somebody's explicit act, which is
//!    precisely the `Settled` resolution E5a reserved for it (D-E5-1).

use mush_core::Ctx;
use mush_economy::{
    ContractStateKind, EconomicEvent, EconomyObserver, EconomyStore, ObligationCause, ObligationId,
    ObligationStatusKind,
};

use crate::module::HousingStore;
use crate::record::{Funding, Resolution, Tenancy};

/// The obligation id a drafted-but-not-yet-activated exact tenancy carries.
///
/// [`crate::Funding::Exact`] names both ids by type, and the rent obligation does not
/// exist until the contract activates — so the draft needs *something*. `u64::MAX` is
/// the one value that can never collide with a real one: the economy's allocators
/// refuse to mint at the ceiling (E4b1's B2), so no obligation is ever numbered this.
/// Every read of an exact tenancy's obligation checks for it first, and boot's
/// reconciliation only inspects **live** tenancies, so a draft is never mistaken for
/// a lease whose obligation went missing.
pub const PENDING_OBLIGATION: ObligationId = ObligationId(u64::MAX);

/// Whether this funding is a lease that has not activated yet.
#[must_use]
pub fn is_pending_lease(f: &Funding) -> bool {
    matches!(f, Funding::Exact { obligation, .. } if *obligation == PENDING_OBLIGATION)
}

/// Housing's ear on the economy (E4c's `with_observers` table).
///
/// It is wired in `mush-server` alongside the canon bridge. Omitting it does not
/// break abstract mode at all — it is exactly and only the exact-mode mirror.
#[derive(Debug)]
pub struct HousingEconomyObserver;

impl EconomyObserver for HousingEconomyObserver {
    fn on_economic_event(&self, ctx: &mut Ctx<'_>, ev: &EconomicEvent) {
        match ev {
            EconomicEvent::ObligationChanged { id, status_kind } => {
                on_obligation(ctx, *id, *status_kind);
            }
            EconomicEvent::ContractChanged { id, state, .. } => {
                on_contract(ctx, *id, *state);
            }
            // Ids and kinds only (D-E4-13), and none of the other three says anything
            // about a lease: a posted transaction, a reservation and a title move are
            // all things the two records above already summarise for housing.
            EconomicEvent::TransactionPosted { .. }
            | EconomicEvent::HoldChanged { .. }
            | EconomicEvent::AssetTitleChanged { .. } => {}
        }
    }
}

/// Mirror an obligation transition onto the tenancy it funds.
///
/// It mirrors **into the ladder's first rung and out of it**, and nothing else: a
/// missed period puts the tenancy in `Grace`, a settled one brings it back to
/// `Active`. Walking further down the ladder is period arithmetic on the tenancy's
/// own cadence, and that is the tick's ([`crate::tick`]) — an obligation can go
/// delinquent on its own schedule, which need not be the campaign clock the tenancy
/// is assessed on, and reading one clock's elapsed time off another is the bug
/// [`mush_economy::EconomicDeadline`] exists to make unwritable.
fn on_obligation(ctx: &mut Ctx<'_>, id: ObligationId, kind: ObligationStatusKind) {
    let Some(t) = tenancy_funded_by(ctx, id) else {
        // **The activation ordering.** `ContractChanged { Active }` is emitted by the
        // contract's own state write, which happens *before* the economy creates the
        // obligations its `Obligation` deliverables describe — so the tenancy cannot
        // be bound from that event: the rent record does not exist yet. This is the
        // edge that does exist by then, because creating the obligation is what emits
        // it. A pending lease waiting on exactly this obligation binds here.
        bind_pending_lease(ctx, id);
        return;
    };
    if !t.status.is_live() {
        return;
    }
    let Some(now) = crate::module::cadence_now(ctx.w, t.cadence) else { return };
    let before = t.status.kind();
    let moved = match kind {
        // Paid, or finished paying: back to Active, silently (§3.2).
        ObligationStatusKind::Active | ObligationStatusKind::Satisfied => {
            crate::period_supported(&t, now)
        }
        // Behind: onto the first rung. Already on it (or further down) is a no-op —
        // `period_missed` never re-stamps a run that has already started.
        ObligationStatusKind::Grace | ObligationStatusKind::Delinquent => {
            crate::period_missed(&t, now)
        }
        // Suspended is an escalation the economy has already paged about, and
        // Cancelled is the contract's unwind, which arrives as `ContractChanged`.
        // Neither is evidence about the tenant.
        ObligationStatusKind::Suspended | ObligationStatusKind::Cancelled => return,
    };
    match moved {
        Ok(next) if next.revision != t.revision => crate::flow::commit(ctx, before, next),
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(tenancy = t.id.0, obligation = id.0, %error, "housing: could not mirror an obligation");
        }
    }
}

/// Bind (and activate) the pending lease this newly-created obligation belongs to.
///
/// Matched on `cause: Contract(id)` — the economy's own key for "this obligation is
/// that contract's" — and only for a tenancy still `Pending` on that contract with
/// the [`PENDING_OBLIGATION`] sentinel, so a second obligation under the same lease
/// (a later amendment) can never re-activate a tenancy that is already live.
fn bind_pending_lease(ctx: &mut Ctx<'_>, id: ObligationId) {
    let Some(contract) = ctx.w.module_state::<EconomyStore>().and_then(|s| {
        s.snapshot().obligation(id).and_then(|o| match &o.cause {
            ObligationCause::Contract(c) => Some(*c),
            _ => None,
        })
    }) else {
        return;
    };
    let Some(t) = tenancy_under(ctx, contract) else { return };
    if !matches!(t.status, crate::record::TenancyStatus::Pending) || !is_pending_lease(&t.funding) {
        return;
    }
    let Some(now) = crate::module::cadence_now(ctx.w, t.cadence) else { return };
    activate_lease(ctx, &t, contract, now);
}

/// Catch-up for the same binding, from the tick (08d §4).
///
/// The event above is the fast path; this is the one that survives a restart, a
/// dropped observer table, or a contract that activated while housing was mid-boot.
/// It asks the two questions the event cannot be trusted to have asked: is this
/// lease's contract live, and does its rent record exist yet?
pub(crate) fn bind_if_ready(ctx: &mut Ctx<'_>, t: &Tenancy) {
    if !matches!(t.status, crate::record::TenancyStatus::Pending) || !is_pending_lease(&t.funding) {
        return;
    }
    let Funding::Exact { contract, .. } = &t.funding else { return };
    let contract = *contract;
    let live = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().contract(contract).map(|c| c.state.is_live()));
    if live != Some(true) {
        return;
    }
    let Some(now) = crate::module::cadence_now(ctx.w, t.cadence) else { return };
    activate_lease(ctx, t, contract, now);
}

/// Activation and unwind.
fn on_contract(ctx: &mut Ctx<'_>, id: mush_economy::ContractId, state: ContractStateKind) {
    let Some(t) = tenancy_under(ctx, id) else { return };
    let Some(now) = crate::module::cadence_now(ctx.w, t.cadence) else { return };
    match state {
        ContractStateKind::Active => activate_lease(ctx, &t, id, now),
        // Every route into a terminal contract state is an explicit act — a staff
        // attestation that settles it, the issuer or staff cancelling it, a deadline
        // the parties wrote into the surface — so this is a `Resolution`, not an
        // automatic ending (D-E5-1).
        ContractStateKind::Settled | ContractStateKind::Cancelled | ContractStateKind::Expired => {
            if t.status.is_ended() {
                return;
            }
            let before = t.status.kind();
            match crate::resolve(&t, Resolution::Settled, now) {
                Ok(next) => {
                    crate::flow::notify_tenants(
                        ctx,
                        &next,
                        &format!(
                            "HOME: the lease behind tenancy #{} has ended (contract #{id}). Your \
                             home there is closed.",
                            next.id
                        ),
                    );
                    crate::flow::commit(ctx, before, next);
                }
                Err(error) => tracing::warn!(
                    tenancy = t.id.0, contract = id.0, %error,
                    "housing: could not end a tenancy whose lease ended"
                ),
            }
        }
        // A lease is offered by `offer_lease` and never drafted, disputed or
        // re-offered by housing; `ReadyToSettle` is the step before `Settled`, which
        // is the transition that matters.
        ContractStateKind::Draft
        | ContractStateKind::Offered
        | ContractStateKind::ReadyToSettle
        | ContractStateKind::Disputed => {}
    }
}

/// The lease activated: bind the obligation it created, then run E5a's `accept`.
fn activate_lease(
    ctx: &mut Ctx<'_>,
    t: &Tenancy,
    contract: mush_economy::ContractId,
    now: i64,
) {
    if !matches!(t.status, crate::record::TenancyStatus::Pending) {
        return;
    }
    let Some(obligation) = obligation_of_contract(ctx, contract) else {
        // The contract activated but minted no rent record (an exhausted allocator —
        // the economy logs that loudly). The tenancy stays `Pending`: a home nobody
        // is being charged for is not a home anybody agreed to.
        tracing::error!(
            tenancy = t.id.0,
            contract = contract.0,
            "housing: a lease activated with no rent obligation; the tenancy stays pending"
        );
        return;
    };
    let mut bound = t.clone();
    bound.funding = Funding::Exact { contract, obligation };
    let before = bound.status.kind();
    // Both sides have consented by construction: the landlord's acceptance is the
    // offer (a staff act on their behalf) and every tenant's is the acceptance that
    // just activated the contract. `accept` still asks for both, so it is told both.
    match crate::accept(&bound, true, &bound.tenants.clone(), now) {
        Ok(next) => {
            crate::flow::notify_tenants(
                ctx,
                &next,
                &format!(
                    "HOME: tenancy #{} is active. Rent is obligation #{obligation} on contract \
                     #{contract}.",
                    next.id
                ),
            );
            crate::flow::commit(ctx, before, next.clone());
            crate::flow::activate(ctx, &next);
        }
        Err(error) => tracing::error!(
            tenancy = t.id.0, contract = contract.0, %error,
            "housing: a lease activated but its tenancy would not"
        ),
    }
}

/// The live tenancy funded by this obligation.
fn tenancy_funded_by(ctx: &Ctx<'_>, id: ObligationId) -> Option<Tenancy> {
    let store = ctx.w.module_state::<HousingStore>()?;
    store
        .tenancies()
        .find(|t| matches!(&t.funding, Funding::Exact { obligation, .. } if *obligation == id))
        .cloned()
}

/// The tenancy standing on this contract, whatever its status.
fn tenancy_under(ctx: &Ctx<'_>, id: mush_economy::ContractId) -> Option<Tenancy> {
    let store = ctx.w.module_state::<HousingStore>()?;
    store
        .tenancies()
        .find(|t| matches!(&t.funding, Funding::Exact { contract, .. } if *contract == id))
        .cloned()
}

/// The obligation a contract's `Obligation` deliverable created — matched on
/// `cause: Contract(id)`, which is the economy's own key for exactly this question.
fn obligation_of_contract(ctx: &Ctx<'_>, id: mush_economy::ContractId) -> Option<ObligationId> {
    let store = ctx.w.module_state::<EconomyStore>()?;
    store
        .snapshot()
        .obligations()
        .find(|o| o.cause == ObligationCause::Contract(id))
        .map(|o| o.id)
}
